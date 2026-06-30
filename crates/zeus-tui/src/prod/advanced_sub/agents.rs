//! Agents — Local + fleet roster, personas, bindings.
//!
//! Advanced subview (id: `agents`). Matches JSX `AdvancedSubview` `agents`
//! branch (docs/zeus-tui-production.jsx ~line 1428): one row per agent —
//! status dot · name · LOCAL badge · host · role · channel count · MSG button.
//! Local agents get an accent left-bar; remote get a dim one. Theme tokens
//! only, geometric glyphs, no emoji, opaque-bg inherited.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::AgentResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// One render row — owns its strings so live (`AgentResponse`) entries unify
/// behind a single draw loop.
struct Row {
    name: String,
    host: String,
    role: String,
    local: bool,
    status: String,
    channels: u8,
}

/// Build the row list from live `/v1/network/agents` data.
///
/// `/v1/network/agents` exposes `name`/`status`/`type` (local vs channel) but
/// has **no** host/role/channel-count fields — those columns honestly fall to
/// "—"/0 for live entries rather than fabricate. When no live data is present
/// (pre-fetch or empty fleet) we return **no rows** — the render path draws an
/// honest empty/loading state rather than a fabricated representative roster
/// (#260/#266: never show invented agent names/hosts/counts as if real).
fn build_rows(live: Option<&[AgentResponse]>) -> Vec<Row> {
    match live {
        Some(agents) if !agents.is_empty() => agents
            .iter()
            .map(|a| Row {
                name: a.name.clone(),
                host: "—".to_string(),
                role: "—".to_string(),
                local: a.agent_type == "local",
                status: a.status.clone(),
                channels: 0,
            })
            .collect(),
        // Pre-fetch or empty fleet → no fabricated roster.
        _ => Vec::new(),
    }
}

/// Render the `agents` subview body into `area`.
///
/// Draws live `/v1/network/agents` data when present; when no agents are
/// available it draws an honest empty/loading line rather than a fabricated
/// roster (#260/#266).
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[AgentResponse]>) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());
    let mut y = area.y + 1;

    let rows = build_rows(live);
    if rows.is_empty() {
        // Honest empty/loading state — no invented agents.
        let msg = "No agents — fetching from /v1/network/agents…";
        set_str(area.x + 2, y, msg, Style::default().fg(theme::DIM), right, buf);
        return;
    }
    for a in rows.iter() {
        if y + 1 >= area.bottom() {
            break;
        }
        let bar = if a.local { theme::ACCENT } else { theme::DIM };
        // Left accent bar (2px in JSX → 1 cell) + row bg panel.
        for ry in y..(y + 2).min(area.bottom()) {
            for x in (area.x + 1)..right {
                if let Some(c) = cell_mut_clamped(buf, x, ry) { c.set_bg(theme::BG_PANEL); }
            }
            if (area.x) < right {
                if let Some(c) = cell_mut_clamped(buf, area.x, ry) { c.set_char('\u{2502}').set_fg(bar).set_bg(theme::BG_PANEL); }
            }
        }

        let mut x = area.x + 2;
        // Status dot — green active, dim otherwise.
        let dot = if a.status == "active" { theme::GREEN } else { theme::DIM };
        x = set_str(x, y, "\u{25cf}", Style::default().fg(dot), right, buf) + 1;
        // Name (white, bold).
        set_str(x, y, &a.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
        x = (area.x + 2 + 16).min(right);
        // LOCAL badge.
        if a.local {
            set_str(x, y, "[LOCAL]", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), right, buf);
        }
        // Host (dim) on row 1.
        let _ = set_str((area.x + 2 + 26).min(right), y, &a.host, Style::default().fg(theme::DIM), right, buf);
        // Role + channels on row 2.
        let y2 = y + 1;
        let _ = set_str(area.x + 4, y2, &a.role, Style::default().fg(theme::TEXT), right, buf);
        let ch = format!("{} channels  [MSG]", a.channels);
        let cx = right.saturating_sub(ch.len() as u16 + 1);
        let _ = set_str(cx, y2, &ch, Style::default().fg(theme::DIM), right, buf);

        y += 3;
    }
}

/// Write `s` at (x,y), clipped to `max_x`. Returns the x after the last cell.
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
    fn no_live_yields_no_fabricated_rows() {
        // Pre-fetch (None) and empty fleet both → zero rows: no invented roster.
        assert!(build_rows(None).is_empty());
        assert!(build_rows(Some(&[])).is_empty());
    }

    #[test]
    fn empty_state_renders_honest_message_not_fake_agents() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None);
        // The honest empty line is present; no fabricated agent name leaks in.
        // Read row-major (each row's cells contiguous) so horizontal strings
        // survive the scan.
        let dump: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(dump.contains("No agents"), "honest empty state must render");
        for fake in ["Hermes", "Argus", "Hestia", "Calliope", "Atlas"] {
            assert!(!dump.contains(fake), "fabricated agent '{fake}' must NOT render");
        }
    }

    #[test]
    fn render_no_panic_small() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None);
        // Degenerate sizes must not panic.
        render(Rect::new(0, 0, 2, 1), &mut buf, None);
    }

    #[test]
    fn live_agents_render_with_honest_dashes() {
        // Populated live → real agents, host/role honest "—", local from type.
        let live = vec![AgentResponse {
            id: "z1".into(),
            name: "zeus106".into(),
            status: "active".into(),
            agent_type: "local".into(),
            ..Default::default()
        }];
        let rows = build_rows(Some(&live));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "zeus106");
        assert!(rows[0].local);
        assert_eq!(rows[0].host, "—");
        assert_eq!(rows[0].channels, 0);
    }
}
