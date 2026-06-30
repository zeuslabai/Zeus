//! Pantheon Tab — multi-agent mission control: mission list, war room shell,
//! pending-plan area, and event stream shell.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`PantheonTab`, JSX 647–812).
//! The production TUI keeps the prototype structure but does not fabricate the
//! old demo mission/war-room/event data. Mission rows are sourced from the live
//! `GET /v1/pantheon/missions` overlay when present; otherwise the tab renders
//! honest empty/loading states.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

type Mission = crate::api::PantheonMissionResponse;

/// JSX `statusColors` map.
fn status_color(status: &str) -> Color {
    match status {
        "draft" => theme::DIM,
        "planning" => theme::AMBER,
        "assembling" => theme::CYAN,
        "active" => theme::GREEN,
        "reviewing" => theme::PURPLE,
        "completed" => theme::DIM,
        _ => theme::DIM,
    }
}

fn status_dim_color(status: &str) -> Color {
    match status {
        "planning" => theme::AMBER_DIM,
        "assembling" => theme::CYAN_DIM,
        "active" => theme::GREEN_DIM,
        "reviewing" => theme::PURPLE_DIM,
        "completed" | "draft" => theme::BG_HIGHLIGHT,
        _ => theme::BG_HIGHLIGHT,
    }
}

/// Live gateway data overlay for the Pantheon tab.
#[derive(Default, Clone, Copy)]
pub struct PantheonLive<'a> {
    /// Live missions from `GET /v1/pantheon/missions`.
    pub missions: Option<&'a [Mission]>,
}

/// The Pantheon tab widget. `selected` indexes into the live mission list.
pub struct PantheonTab<'a> {
    pub selected: usize,
    /// Live gateway data overlay. `None` in standalone / pre-fetch.
    pub live: Option<PantheonLive<'a>>,
}

impl PantheonTab<'_> {
    pub fn new(selected: usize) -> Self {
        Self {
            selected,
            live: None,
        }
    }

    /// Build a tab that renders live gateway mission data.
    pub fn with_live<'a>(selected: usize, live: PantheonLive<'a>) -> PantheonTab<'a> {
        PantheonTab {
            selected,
            live: Some(live),
        }
    }
}

impl Widget for PantheonTab<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        if area.width == 0 || area.height == 0 {
            return;
        }

        // JSX: left mission rail (320px), flexible mission detail, right event rail.
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(40),
                Constraint::Min(42),
                Constraint::Length(38),
            ])
            .split(area);

        let missions = self.live.and_then(|l| l.missions);
        let selected = selected_mission(self.selected, missions.unwrap_or(&[]));

        render_mission_list(self.selected, cols[0], buf, missions);
        render_center(selected, cols[1], buf);
        render_events(selected, cols[2], buf);
    }
}

fn selected_mission(selected: usize, missions: &[Mission]) -> Option<&Mission> {
    if missions.is_empty() {
        None
    } else {
        missions.get(selected).or_else(|| missions.first())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Left — mission list (JSX width 320)
// ─────────────────────────────────────────────────────────────────────────────

fn render_mission_list(
    selected: usize,
    area: Rect,
    buf: &mut Buffer,
    missions: Option<&[Mission]>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    draw_vline(area.right().saturating_sub(1), area.top(), area.height, buf);
    let inner = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    fill_bg(rows[0], theme::BG_PANEL, buf);
    set_str(
        rows[0].x + 1,
        rows[0].y,
        "MISSIONS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
        rows[0].right(),
        buf,
    );
    let count = missions.map_or(0, <[Mission]>::len);
    let count_x = rows[0].right().saturating_sub(12);
    set_str(
        count_x,
        rows[0].y,
        &format!("{} live", count),
        Style::default().fg(theme::DIM),
        rows[0].right(),
        buf,
    );
    set_str(
        rows[0].x + 1,
        rows[0].y + 1,
        "Active war rooms + scheduled work",
        Style::default().fg(theme::DIM),
        rows[0].right(),
        buf,
    );
    draw_hline(
        rows[0].x,
        rows[0].bottom().saturating_sub(1),
        rows[0].width,
        buf,
    );

    let list = rows[1];
    match missions {
        Some([]) => render_empty_list(
            list,
            "no live missions",
            "waiting on /v1/pantheon/missions",
            buf,
        ),
        None => render_empty_list(
            list,
            "loading missions",
            "fetching /v1/pantheon/missions...",
            buf,
        ),
        Some(items) => render_mission_rows(selected, list, items, buf),
    }

    fill_bg(rows[2], theme::BG_PANEL, buf);
    render_hints(
        rows[2].x + 1,
        rows[2].y,
        &[("n", "new mission"), ("p", "pause"), ("c", "cancel")],
        rows[2].right(),
        buf,
    );
}

fn render_empty_list(area: Rect, title: &str, detail: &str, buf: &mut Buffer) {
    if area.height == 0 {
        return;
    }
    let y = area.y + area.height.min(3).saturating_sub(1);
    set_str(
        area.x + 2,
        y,
        title,
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
        area.right(),
        buf,
    );
    if area.height > 2 {
        set_str(
            area.x + 2,
            y + 1,
            detail,
            Style::default().fg(theme::MUTED),
            area.right(),
            buf,
        );
    }
}

fn render_mission_rows(selected: usize, area: Rect, missions: &[Mission], buf: &mut Buffer) {
    let mut y = area.y;
    for (i, mission) in missions.iter().enumerate() {
        if y >= area.bottom() {
            break;
        }
        let row_h = 4.min(area.bottom().saturating_sub(y));
        let row = Rect {
            x: area.x,
            y,
            width: area.width,
            height: row_h,
        };
        let is_selected = i == selected || (selected >= missions.len() && i == 0);
        let color = status_color(&mission.status);
        if is_selected {
            fill_bg(row, theme::BG_HIGHLIGHT, buf);
        }
        for dy in 0..row.height {
            buf[(row.x, row.y + dy)]
                .set_char('┃')
                .set_fg(if is_selected { color } else { theme::BG });
        }

        let tx = row.x + 2;
        buf[(tx, row.y)].set_char('●').set_fg(color);
        set_str(
            tx + 2,
            row.y,
            non_empty(&mission.name, "untitled mission"),
            Style::default()
                .fg(if is_selected {
                    theme::WHITE
                } else {
                    theme::TEXT
                })
                .add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            row.right(),
            buf,
        );

        let status = non_empty(&mission.status, "draft");
        set_str(
            tx,
            row.y + 1,
            &status.to_uppercase(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
            row.right(),
            buf,
        );
        let meta = format!("{} agents · live", mission.agent_count);
        let meta_x = row.right().saturating_sub(meta.len() as u16 + 1);
        set_str(
            meta_x,
            row.y + 1,
            &meta,
            Style::default().fg(theme::DIM),
            row.right(),
            buf,
        );

        if row.height > 2 {
            render_progress_bar(
                tx,
                row.y + 2,
                row.width.saturating_sub(4),
                mission_progress(status),
                color,
                buf,
            );
        }
        draw_hline(row.x, row.bottom().saturating_sub(1), row.width, buf);
        y = y.saturating_add(4);
    }
}

fn mission_progress(status: &str) -> u16 {
    match status {
        "draft" => 0,
        "planning" => 12,
        "assembling" => 32,
        "active" => 68,
        "reviewing" => 95,
        "completed" => 100,
        _ => 0,
    }
}

fn render_progress_bar(x: u16, y: u16, width: u16, pct: u16, color: Color, buf: &mut Buffer) {
    if width == 0 {
        return;
    }
    let pct = pct.min(100);
    let fill = (u32::from(width) * u32::from(pct) / 100) as u16;
    for dx in 0..width {
        let cell = &mut buf[(x + dx, y)];
        cell.set_char('━')
            .set_fg(if dx < fill { color } else { theme::BG });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Center — mission header + war room + plan card shell
// ─────────────────────────────────────────────────────────────────────────────

fn render_center(mission: Option<&Mission>, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(8),
        ])
        .split(area);

    render_mission_header(mission, rows[0], buf);
    render_war_room(mission, rows[1], buf);
    render_plan_card(mission, rows[2], buf);
}

fn render_mission_header(mission: Option<&Mission>, area: Rect, buf: &mut Buffer) {
    fill_bg(area, theme::BG_PANEL, buf);
    match mission {
        Some(m) => {
            let status = non_empty(&m.status, "draft");
            let color = status_color(status);
            buf[(area.x + 1, area.y)].set_char('●').set_fg(color);
            set_str(
                area.x + 3,
                area.y,
                non_empty(&m.name, "untitled mission"),
                Style::default()
                    .fg(theme::WHITE)
                    .add_modifier(Modifier::BOLD),
                area.right(),
                buf,
            );
            let badge = format!(" {} ", status.to_uppercase());
            let badge_x = area.right().saturating_sub(badge.len() as u16 + 1);
            set_str(
                badge_x,
                area.y,
                &badge,
                Style::default()
                    .fg(color)
                    .bg(status_dim_color(status))
                    .add_modifier(Modifier::BOLD),
                area.right(),
                buf,
            );
            let meta = format!("{} agents · lead — · started —", m.agent_count);
            set_str(
                area.x + 3,
                area.y + 1,
                &meta,
                Style::default().fg(theme::DIM),
                area.right(),
                buf,
            );
        }
        None => {
            set_str(
                area.x + 1,
                area.y,
                "No Pantheon mission selected",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
                area.right(),
                buf,
            );
            set_str(
                area.x + 1,
                area.y + 1,
                "live mission data has not arrived yet",
                Style::default().fg(theme::MUTED),
                area.right(),
                buf,
            );
        }
    }
    draw_hline(area.x, area.bottom().saturating_sub(1), area.width, buf);
}

fn render_war_room(mission: Option<&Mission>, area: Rect, buf: &mut Buffer) {
    set_str(
        area.x + 1,
        area.y,
        "WAR ROOM",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
        area.right(),
        buf,
    );
    let room = mission
        .map(|m| format!("#{}", slug(&m.name, &m.id)))
        .unwrap_or_else(|| "#pantheon".to_string());
    let room_x = area.right().saturating_sub(room.len() as u16 + 1);
    set_str(
        room_x,
        area.y,
        &room,
        Style::default().fg(theme::DIM),
        area.right(),
        buf,
    );
    if area.height > 3 {
        set_str(
            area.x + 2,
            area.y + 2,
            "No live war-room transcript yet",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
            area.right(),
            buf,
        );
        set_str(
            area.x + 2,
            area.y + 3,
            "waiting on /v1/pantheon/rooms/{id}/messages",
            Style::default().fg(theme::MUTED),
            area.right(),
            buf,
        );
    }
}

fn render_plan_card(mission: Option<&Mission>, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    draw_hline_colored(area.x, area.y, area.width, theme::AMBER, buf);
    let card = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(1),
        ..area
    };
    fill_bg(card, theme::AMBER_DIM, buf);
    if card.height == 0 {
        return;
    }
    buf[(card.x + 1, card.y)].set_char('!').set_fg(theme::AMBER);
    set_str(
        card.x + 3,
        card.y,
        "PLAN CARD",
        Style::default()
            .fg(theme::AMBER)
            .add_modifier(Modifier::BOLD),
        card.right(),
        buf,
    );
    let source = mission
        .map(|m| format!("mission {}", non_empty(&m.id, "—")))
        .unwrap_or_else(|| "no mission selected".to_string());
    set_str(
        card.right().saturating_sub(source.len() as u16 + 1),
        card.y,
        &source,
        Style::default().fg(theme::MUTED),
        card.right(),
        buf,
    );
    if card.height > 2 {
        set_str(
            card.x + 3,
            card.y + 2,
            "No pending plan awaiting approval",
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
            card.right(),
            buf,
        );
        set_str(
            card.x + 3,
            card.y + 3,
            "live plan approvals will render here with approve / reject controls",
            Style::default().fg(theme::TEXT),
            card.right(),
            buf,
        );
    }
    if card.height > 5 {
        render_hints(
            card.x + 3,
            card.bottom().saturating_sub(1),
            &[("a", "approve"), ("r", "reject"), ("i", "intervene")],
            card.right(),
            buf,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Right — live event stream shell
// ─────────────────────────────────────────────────────────────────────────────

fn render_events(mission: Option<&Mission>, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    draw_vline(area.x, area.y, area.height, buf);
    let inner = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(1),
        ..area
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    fill_bg(rows[0], theme::BG_PANEL, buf);
    set_str(
        rows[0].x + 1,
        rows[0].y,
        "LIVE EVENTS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
        rows[0].right(),
        buf,
    );
    set_str(
        rows[0].x + 1,
        rows[0].y + 1,
        "mission stream + agent actions",
        Style::default().fg(theme::DIM),
        rows[0].right(),
        buf,
    );
    draw_hline(
        rows[0].x,
        rows[0].bottom().saturating_sub(1),
        rows[0].width,
        buf,
    );

    set_str(
        rows[1].x + 2,
        rows[1].y + 1,
        "No live Pantheon events",
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
        rows[1].right(),
        buf,
    );
    set_str(
        rows[1].x + 2,
        rows[1].y + 2,
        "waiting on /v1/pantheon/rooms/{id}/stream",
        Style::default().fg(theme::MUTED),
        rows[1].right(),
        buf,
    );

    fill_bg(rows[2], theme::BG_PANEL, buf);
    let status = mission
        .map(|m| {
            format!(
                "● stream ready · /v1/pantheon/rooms/{}/stream",
                non_empty(&m.id, "{id}")
            )
        })
        .unwrap_or_else(|| "● stream idle · select a live mission".to_string());
    let dot_color = if mission.is_some() {
        theme::GREEN
    } else {
        theme::DIM
    };
    if rows[2].width > 0 {
        buf[(rows[2].x + 1, rows[2].y)]
            .set_char('●')
            .set_fg(dot_color);
        set_str(
            rows[2].x + 3,
            rows[2].y,
            status.strip_prefix("● ").unwrap_or(&status),
            Style::default().fg(theme::MUTED),
            rows[2].right(),
            buf,
        );
    }
}

fn non_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn slug(name: &str, id: &str) -> String {
    let base = if name.trim().is_empty() { id } else { name };
    let mut out = String::new();
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 18 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn set_str(x: u16, y: u16, s: &str, style: Style, max_x: u16, buf: &mut Buffer) {
    if y >= buf.area.bottom() || x >= max_x || x >= buf.area.right() {
        return;
    }
    let limit = max_x.min(buf.area.right());
    for (i, ch) in s.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= limit {
            break;
        }
        buf[(cx, y)].set_char(ch).set_style(style);
    }
}

fn fill_bg(area: Rect, color: Color, buf: &mut Buffer) {
    let right = area.right().min(buf.area.right());
    let bottom = area.bottom().min(buf.area.bottom());
    for y in area.y..bottom {
        for x in area.x..right {
            buf[(x, y)].set_bg(color);
        }
    }
}

fn draw_vline(x: u16, y: u16, h: u16, buf: &mut Buffer) {
    if x >= buf.area.right() {
        return;
    }
    for yy in y..y.saturating_add(h).min(buf.area.bottom()) {
        buf[(x, yy)].set_char('│').set_fg(theme::MUTED);
    }
}

fn draw_hline(x: u16, y: u16, w: u16, buf: &mut Buffer) {
    draw_hline_colored(x, y, w, theme::MUTED, buf);
}

fn draw_hline_colored(x: u16, y: u16, w: u16, color: Color, buf: &mut Buffer) {
    if y >= buf.area.bottom() {
        return;
    }
    let right = x.saturating_add(w).min(buf.area.right());
    for xx in x..right {
        buf[(xx, y)].set_char('─').set_fg(color);
    }
}

fn render_hints(x: u16, y: u16, hints: &[(&str, &str)], max_x: u16, buf: &mut Buffer) {
    let mut cx = x;
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            set_str(cx, y, " · ", Style::default().fg(theme::MUTED), max_x, buf);
            cx += 3;
        }
        set_str(
            cx,
            y,
            key,
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
            max_x,
            buf,
        );
        cx += key.len() as u16 + 1;
        set_str(cx, y, label, Style::default().fg(theme::DIM), max_x, buf);
        cx += label.len() as u16;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::PantheonMissionResponse;
    use ratatui::layout::Rect;

    const FAKE_LEAKS: &[&str] = &[
        "v0.4.7 release prep",
        "Onboarding wizard impl",
        "Fleet shakedown audit",
        "Q1 marketing campaign",
        "DGX Spark integration",
        "Aegis hardening",
        "Hephaestus",
        "Hermes",
        "Atlas",
        "Calliope",
        "Phase 2 — Tools browser + memory tab",
        "PR #2847",
        "7,801 passed",
    ];

    fn dump(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn status_colors_match_jsx() {
        assert_eq!(status_color("active"), theme::GREEN);
        assert_eq!(status_color("planning"), theme::AMBER);
        assert_eq!(status_color("reviewing"), theme::PURPLE);
        assert_eq!(status_color("assembling"), theme::CYAN);
        assert_eq!(status_color("draft"), theme::DIM);
        assert_eq!(status_color("completed"), theme::DIM);
    }

    #[test]
    fn no_live_data_renders_honest_empty_state_without_fabricated_catalog() {
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        PantheonTab::new(0).render(area, &mut buf);
        let d = dump(&buf);

        for expected in [
            "MISSIONS",
            "loading missions",
            "/v1/pantheon/missions",
            "No Pantheon mission selected",
            "WAR ROOM",
            "No live war-room transcript yet",
            "PLAN CARD",
            "No pending plan awaiting approval",
            "LIVE EVENTS",
            "No live Pantheon events",
        ] {
            assert!(d.contains(expected), "missing {expected:?}:\n{d}");
        }
        for fake in FAKE_LEAKS {
            assert!(!d.contains(fake), "fabricated data leaked: {fake:?}");
        }
    }

    #[test]
    fn live_missions_render_real_names_and_honest_agent_count() {
        let live = vec![
            PantheonMissionResponse {
                id: "x1".into(),
                name: "Live mission alpha".into(),
                status: "active".into(),
                agent_count: 3,
            },
            PantheonMissionResponse {
                id: "x2".into(),
                name: "Live mission beta".into(),
                status: "planning".into(),
                agent_count: 1,
            },
        ];
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        PantheonTab::with_live(
            0,
            PantheonLive {
                missions: Some(&live),
            },
        )
        .render(area, &mut buf);
        let d = dump(&buf);

        for expected in [
            "Live mission alpha",
            "Live mission beta",
            "3 agents · live",
            "ACTIVE",
            "WAR ROOM",
            "#live-mission-alpha",
            "stream ready · /v1/pantheon/rooms/",
        ] {
            assert!(d.contains(expected), "missing {expected:?}:\n{d}");
        }
        for fake in FAKE_LEAKS {
            assert!(
                !d.contains(fake),
                "fabricated data leaked with live: {fake:?}"
            );
        }
    }

    #[test]
    fn empty_live_list_renders_honest_empty_state() {
        let live: Vec<PantheonMissionResponse> = Vec::new();
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        PantheonTab::with_live(
            0,
            PantheonLive {
                missions: Some(&live),
            },
        )
        .render(area, &mut buf);
        let d = dump(&buf);

        assert!(d.contains("0 live"), "live count missing:\n{d}");
        assert!(d.contains("no live missions"), "empty state missing:\n{d}");
        assert!(
            d.contains("waiting on /v1/pantheon/missions"),
            "source hint missing:\n{d}"
        );
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 30));
        PantheonTab::new(0).render(Rect::new(0, 0, 2, 1), &mut buf);
        PantheonTab::new(0).render(Rect::new(0, 0, 0, 0), &mut buf);
    }

    #[test]
    fn selection_index_is_stored_unclamped() {
        assert_eq!(PantheonTab::new(99).selected, 99);
        assert_eq!(PantheonTab::new(0).selected, 0);
    }
}
