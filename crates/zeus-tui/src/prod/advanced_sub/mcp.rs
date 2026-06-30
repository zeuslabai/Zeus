//! MCP — Model Context Protocol servers + exposed tools.
//!
//! Advanced subview (id: `mcp`). The JSX prototype has no dedicated `mcp`
//! branch (only agents/skills/voice/economy are specialized), so this builds a
//! clean representative panel consistent with the others: a connection summary
//! line then one row per MCP server — status dot · name · transport · tool
//! count · RECONNECT/INSPECT actions. Theme tokens, geometric glyphs, no emoji.
//! No live data → honest "awaiting" line, no fabricated servers (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::McpServerResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// One render row — unified shape for live (`McpServerResponse`) data.
struct Row<'a> {
    name: &'a str,
    transport: &'a str,
    status: &'a str,
    tools: u8,
}

fn status_color(status: &str) -> ratatui::style::Color {
    match status {
        "connected" => theme::GREEN,
        "degraded" => theme::AMBER,
        "offline" => theme::DIM,
        _ => theme::DIM,
    }
}

/// Render the `mcp` subview body into `area`.
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[McpServerResponse]>) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());

    // Build the row list: live overlay when present, else empty.
    // `/v1/mcp/servers` (McpServerResponse) exposes name/command/transport only
    // — no status/tools field, so live rows show "connected"/0 (honest
    // degradation, same pattern as skills→category/tools).
    let rows: Vec<Row> = match live {
        Some(servers) => servers
            .iter()
            .map(|s| Row {
                name: &s.name,
                transport: &s.transport,
                status: "connected",
                tools: 0,
            })
            .collect(),
        // Pre-fetch or no servers → no fabricated roster (#284 de-mock).
        None => Vec::new(),
    };

    let connected = rows.iter().filter(|s| s.status == "connected").count();
    let total_tools: u16 = rows.iter().map(|s| s.tools as u16).sum();

    // Summary line: "<N> connected · <M> tools exposed".
    let mut x = area.x + 2;
    x = set_str(x, area.y + 1, &format!("{}", connected), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), right, buf);
    x = set_str(x, area.y + 1, &format!("/{} connected   ", rows.len()), Style::default().fg(theme::TEXT), right, buf);
    x = set_str(x, area.y + 1, &format!("{}", total_tools), Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD), right, buf);
    let _ = set_str(x, area.y + 1, " tools exposed", Style::default().fg(theme::TEXT), right, buf);
    // ADD SERVER button (accent, right-aligned).
    let btn = " ADD SERVER ";
    let bx = right.saturating_sub(btn.len() as u16 + 1);
    paint_button(bx, area.y + 1, btn, theme::ACCENT, theme::BG, buf);

    // Server rows.
    let mut y = area.y + 3;
    if rows.is_empty() {
        // Honest empty state — no fabricated servers.
        set_str(area.x + 2, y, "No MCP servers — fetching from /v1/mcp/servers…", Style::default().fg(theme::DIM), right, buf);
        return;
    }
    for s in rows.iter() {
        if y >= area.bottom() {
            break;
        }
        let bar = status_color(s.status);
        for cx in (area.x + 1)..right {
            if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_bg(theme::BG_PANEL); }
        }
        if area.x < right {
            if let Some(c) = cell_mut_clamped(buf, area.x, y) { c.set_char('\u{2502}').set_fg(bar).set_bg(theme::BG_PANEL); }
        }
        let mut x = area.x + 2;
        // Status dot.
        x = set_str(x, y, "\u{25cf}", Style::default().fg(bar), right, buf) + 1;
        // Name (white bold).
        let _ = set_str(x, y, s.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
        x = (area.x + 2 + 18).min(right);
        // Transport badge (cyan, uppercase).
        x = set_str(x, y, &format!("\u{25aa} {}", s.transport.to_uppercase()), Style::default().fg(theme::CYAN), right, buf);
        let _ = x;
        // Tools + actions, right-aligned.
        let action = if s.status == "offline" || s.status == "degraded" { "[RECONNECT]" } else { "[INSPECT]" };
        let acol = if s.status == "connected" { theme::DIM } else { theme::AMBER };
        let lead = format!("{} tools  ", s.tools);
        let tail_len = lead.len() + action.len();
        let tx = right.saturating_sub(tail_len as u16 + 1);
        let after = set_str(tx, y, &lead, Style::default().fg(theme::DIM), right, buf);
        let _ = set_str(after, y, action, Style::default().fg(acol).add_modifier(Modifier::BOLD), right, buf);

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
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_style(Style::default().add_modifier(Modifier::BOLD).bg(fill).fg(text)); }
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
    fn status_colors_cover_all_states() {
        assert_eq!(status_color("connected"), theme::GREEN);
        assert_eq!(status_color("degraded"), theme::AMBER);
        assert_eq!(status_color("offline"), theme::DIM);
    }

    #[test]
    fn none_yields_no_fabricated_rows() {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        // No live data → honest empty state, no fabricated servers.
        render(area, &mut buf, None);
        let dump: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("fetching from /v1/mcp/servers"));
        assert!(!dump.contains("filesystem"));
        assert!(!dump.contains("puppeteer"));
    }

    #[test]
    fn render_no_panic() {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None);
        render(Rect::new(0, 0, 3, 1), &mut buf, None);
    }
}
