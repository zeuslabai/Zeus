//! Projects — Create projects, assign agents, track status.
//!
//! Advanced subview (id: `projects`). No specific JSX `AdvancedSubview` block
//! exists for this id, so this is a clean representative panel consistent with
//! the agents/skills siblings: a summary line + NEW PROJECT button, then one
//! row per project — status dot · name · lead · agent count · progress.
//! Theme tokens, geometric glyphs, no emoji, opaque-bg inherited. No live
//! data → honest "awaiting" line, no fabricated projects (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::ProjectResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// An owned project row — built from live `/v1/projects` data. `lead`/
/// `progress` are backed by #249 (live rows carry the real values; empty
/// lead → `—`, progress defaults 0).
struct Row {
    name: String,
    lead: String,
    agents: u8,
    progress: u8,
    status: Status,
}

#[derive(Clone, Copy)]
enum Status {
    Active,
    Planning,
    Blocked,
    Done,
}

impl Status {
    fn color(self) -> Color {
        match self {
            Status::Active => theme::GREEN,
            Status::Planning => theme::CYAN,
            Status::Blocked => theme::AMBER,
            Status::Done => theme::DIM,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Status::Active => "ACTIVE",
            Status::Planning => "PLANNING",
            Status::Blocked => "BLOCKED",
            Status::Done => "DONE",
        }
    }
    /// Map the server's free-form status string onto a known state.
    fn from_str(s: &str) -> Status {
        match s.to_ascii_lowercase().as_str() {
            "planning" | "pending" => Status::Planning,
            "blocked" | "error" | "failed" => Status::Blocked,
            "done" | "complete" | "completed" | "archived" => Status::Done,
            _ => Status::Active,
        }
    }
}

/// Build the rows to render. Live overlay when `Some` and non-empty
/// (name/status/agent-count/lead/progress all live-backed per #249; empty
/// lead → `—`). No live data → empty (render shows an honest "awaiting" line).
fn build_rows(live: Option<&[ProjectResponse]>) -> Vec<Row> {
    match live {
        Some(ps) if !ps.is_empty() => ps
            .iter()
            .map(|p| Row {
                name: p.name.clone(),
                // #249: lead defaults to "" server-side → render `—` when empty.
                lead: if p.lead.is_empty() {
                    "—".to_string()
                } else {
                    p.lead.clone()
                },
                agents: p.agents.len().min(255) as u8,
                progress: p.progress, // #249: 0–100, server-defaulted to 0.
                status: Status::from_str(&p.status),
            })
            .collect(),
        // Pre-fetch or no projects → no fabricated roster (#284 de-mock).
        _ => Vec::new(),
    }
}

/// Render the `projects` subview body into `area`.
///
/// `live` overlays real `/v1/projects` data when present; `None`/empty shows
/// an honest "awaiting" line — no fabricated roster (#284 de-mock).
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[ProjectResponse]>) {
    Clear.render(area, buf);
    if area.width < 4 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());

    let rows = build_rows(live);
    let active = rows.iter().filter(|r| matches!(r.status, Status::Active)).count() as u16;
    let total = rows.len() as u16;

    // Summary line: "<N> active · <M> total".
    let mut x = area.x + 2;
    x = set_str(x, area.y + 1, &format!("{}", active), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), right, buf);
    x = set_str(x, area.y + 1, " active   ", Style::default().fg(theme::TEXT), right, buf);
    x = set_str(x, area.y + 1, &format!("{}", total), Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
    let _ = set_str(x, area.y + 1, " total", Style::default().fg(theme::TEXT), right, buf);
    // NEW PROJECT button (right-aligned, accent).
    let btn = " NEW PROJECT ";
    let bx = right.saturating_sub(btn.len() as u16 + 1);
    paint_button(bx, area.y + 1, btn, theme::ACCENT, theme::BG, buf);

    // Project rows.
    let mut y = area.y + 3;
    if rows.is_empty() {
        // Honest empty state — no fabricated projects.
        set_str(area.x + 2, y, "No projects — fetching from /v1/projects…", Style::default().fg(theme::DIM), right, buf);
        return;
    }
    for p in rows.iter() {
        if y >= area.bottom() {
            break;
        }
        // Row bg panel.
        for cx in (area.x + 1)..right {
            if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_bg(theme::BG_PANEL); }
        }
        let mut cx = area.x + 2;
        // Status dot.
        cx = set_str(cx, y, "● ", Style::default().fg(p.status.color()), right, buf);
        // Name (white bold).
        cx = set_str(cx, y, &p.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
        cx = set_str(cx, y, "  ", Style::default().fg(theme::TEXT), right, buf);
        // Status label.
        cx = set_str(cx, y, p.status.label(), Style::default().fg(p.status.color()), right, buf);
        cx = set_str(cx, y, "   ", Style::default().fg(theme::TEXT), right, buf);
        // Lead.
        cx = set_str(cx, y, "lead ", Style::default().fg(theme::DIM), right, buf);
        cx = set_str(cx, y, &p.lead, Style::default().fg(theme::TEXT), right, buf);
        cx = set_str(cx, y, "   ", Style::default().fg(theme::TEXT), right, buf);
        // Agent count.
        cx = set_str(cx, y, &format!("{} agents", p.agents), Style::default().fg(theme::CYAN), right, buf);
        let _ = cx;

        // Progress (right-aligned: "NN%" + small bar).
        let pct = format!("{}%", p.progress);
        let bar_w = 10u16;
        let pcol = if p.progress >= 100 { theme::GREEN } else { theme::ACCENT };
        let bx = right.saturating_sub(bar_w + 6);
        // Bar track + fill.
        let filled = (p.progress as u16 * bar_w) / 100;
        for i in 0..bar_w {
            let gx = bx + i;
            if gx >= right {
                break;
            }
            let (ch, col) = if i < filled { ("▪", pcol) } else { ("·", theme::DIM) };
            let _ = set_str(gx, y, ch, Style::default().fg(col), right, buf);
        }
        let _ = set_str(bx + bar_w + 1, y, &pct, Style::default().fg(pcol).add_modifier(Modifier::BOLD), right, buf);

        y += 2;
    }
}

/// Paint a solid filled button (bg=fill, fg=text).
fn paint_button(x: u16, y: u16, label: &str, fill: Color, text: Color, buf: &mut Buffer) {
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
        // Pre-fetch (None) → zero rows: no invented roster.
        assert!(build_rows(None).is_empty());
    }

    #[test]
    fn empty_live_yields_no_fabricated_rows() {
        // Empty fleet → zero rows: no invented roster.
        assert!(build_rows(Some(&[])).is_empty());
    }

    #[test]
    fn live_rows_are_honest() {
        let live = vec![
            ProjectResponse {
                name: "live-alpha".to_string(),
                status: "blocked".to_string(),
                agents: vec![serde_json::Value::Null, serde_json::Value::Null],
                lead: "Hermes".to_string(), // #249: real lead flows through
                progress: 73,               // #249: real progress flows through
            },
            ProjectResponse {
                name: "live-beta".to_string(),
                status: "done".to_string(),
                agents: vec![],
                lead: String::new(), // #249: empty lead → honest `—`
                progress: 0,
            },
        ];
        let rows = build_rows(Some(&live));
        assert_eq!(rows.len(), 2);
        // Backed columns are live.
        assert_eq!(rows[0].name, "live-alpha");
        assert_eq!(rows[0].agents, 2);
        assert!(matches!(rows[0].status, Status::Blocked));
        assert!(matches!(rows[1].status, Status::Done));
        // #249: lead/progress now flow live when present.
        assert_eq!(rows[0].lead, "Hermes");
        assert_eq!(rows[0].progress, 73);
        // Empty lead still honestly renders `—`, never fabricated.
        assert_eq!(rows[1].lead, "—");
        assert_eq!(rows[1].progress, 0);
    }

    #[test]
    fn status_from_str_maps_known_states() {
        assert!(matches!(Status::from_str("active"), Status::Active));
        assert!(matches!(Status::from_str("Planning"), Status::Planning));
        assert!(matches!(Status::from_str("FAILED"), Status::Blocked));
        assert!(matches!(Status::from_str("completed"), Status::Done));
        assert!(matches!(Status::from_str("weird-unknown"), Status::Active));
    }

    #[test]
    fn render_no_panic() {
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None);
        render(Rect::new(0, 0, 3, 1), &mut buf, None);
        render(Rect::new(0, 0, 20, 4), &mut buf, None);
        // Live path renders without panic too.
        let live = vec![ProjectResponse { name: "x".into(), status: "active".into(), agents: vec![], lead: String::new(), progress: 0 }];
        render(area, &mut buf, Some(&live));
    }
}
