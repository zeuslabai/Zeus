// S91 audit: reviewed by zeus106
//! The Office — pixel art fleet visualization in terminal.
//!
//! Half-block rendering: each terminal cell shows 2 vertical pixels using `▀`.
//! An 80×40 pixel office becomes 80×20 terminal cells for the main scene.

pub mod background;
#[allow(dead_code)]
pub mod palette;
#[allow(dead_code)]
pub mod renderer;
#[allow(dead_code)]
pub mod sprites;
pub mod state;

use ratatui::prelude::*;
use ratatui::widgets::*;
use state::{OfficeState, Zone};
use renderer::{clone_grid, stamp_sprite, render_halfblock};
use sprites::make_sprite;

/// Generate a background sized for the given terminal area.
/// The scene area is `area.width - 28` (sidebar) by `area.height * 2` (half-block = 2 pixels/row).
/// Call this when the terminal resizes and cache the result.
pub fn generate_bg_for_area(area: Rect) -> renderer::PixelGrid {
    let sidebar_w = 26u16;
    let scene_w = area.width.saturating_sub(sidebar_w + 2) as usize; // -2 for borders
    let scene_h = area.height.saturating_sub(2) as usize * 2; // half-block: 2 pixel rows per terminal row, -2 for borders
    background::generate(scene_w, scene_h)
}

/// Update the OfficeState scene dimensions to match the terminal area.
pub fn update_scene_dimensions(state: &mut OfficeState, area: Rect) {
    let sidebar_w = 26u16;
    let w = area.width.saturating_sub(sidebar_w + 2) as usize;
    let h = area.height.saturating_sub(2) as usize * 2;
    state.scene_width = w.max(20);
    state.scene_height = h.max(10);
}

/// Compose the full scene: background + agent sprites + monitor glow.
fn compose_scene(bg: &renderer::PixelGrid, state: &OfficeState) -> renderer::PixelGrid {
    let mut scene = clone_grid(bg);
    let w = state.scene_width;
    let h = state.scene_height;

    // Monitor glow: brighten screens near active agents at desks
    let glow = ratatui::style::Color::Rgb(60, 120, 180);
    for agent in &state.agents {
        if agent.state == state::AgentState::Idle { continue; }
        let ax = agent.x.round() as usize;
        let ay = agent.y.round() as usize;
        let half_w = w / 2;
        // Engineering desks (top-left, y~14 scaled, monitors at y=8-9 scaled)
        let eng_y_lo = background::scale_y(12, h) as usize;
        let eng_y_hi = background::scale_y(16, h) as usize;
        let mon_y0 = background::scale_y(8, h) as usize;
        let mon_y1 = background::scale_y(9, h) as usize;
        if ay >= eng_y_lo && ay <= eng_y_hi && ax < half_w {
            for d in 0..3 {
                let desk_x_base = background::scale_x((4 + d * 10) as i32, w) as usize;
                let desk_w = (w * 8 / 80).max(4);
                if ax >= desk_x_base && ax < desk_x_base + desk_w + 2 {
                    let mon_start = desk_x_base + desk_w / 4;
                    let mon_end = desk_x_base + desk_w * 3 / 4;
                    for mx in mon_start..mon_end {
                        if mx < scene[0].len() {
                            if mon_y0 < scene.len() { if let Some(ref mut px) = scene[mon_y0][mx] { *px = glow; } }
                            if mon_y1 < scene.len() { if let Some(ref mut px) = scene[mon_y1][mx] { *px = glow; } }
                        }
                    }
                }
            }
        }
        // Comms desk (top-right)
        if ay >= eng_y_lo && ay <= eng_y_hi && ax >= half_w {
            let cx0 = background::scale_x(50, w) as usize;
            let comms_w = (w * 12 / 80).max(4);
            if ax >= cx0 && ax < cx0 + comms_w + 2 {
                for off in [3, 4, 7, 8] {
                    let mx = cx0 + (off * w / 80).min(comms_w.saturating_sub(1));
                    if mx < scene[0].len() {
                        if mon_y0 < scene.len() { if let Some(ref mut px) = scene[mon_y0][mx] { *px = glow; } }
                        if mon_y1 < scene.len() { if let Some(ref mut px) = scene[mon_y1][mx] { *px = glow; } }
                    }
                }
            }
        }
        // Research desk (bottom-left)
        let res_y_lo = background::scale_y(28, h) as usize;
        let res_y_hi = background::scale_y(33, h) as usize;
        let res_mon_y0 = background::scale_y(26, h) as usize;
        let res_mon_y1 = background::scale_y(27, h) as usize;
        if ay >= res_y_lo && ay <= res_y_hi && ax < half_w {
            let rx0 = background::scale_x(8, w) as usize;
            let rx1 = background::scale_x(22, w) as usize;
            if ax >= rx0 && ax < rx1 {
                for off in [10, 11, 12] {
                    let mx = background::scale_x(off, w) as usize;
                    if mx < scene[0].len() {
                        if res_mon_y0 < scene.len() { if let Some(ref mut px) = scene[res_mon_y0][mx] { *px = glow; } }
                        if res_mon_y1 < scene.len() { if let Some(ref mut px) = scene[res_mon_y1][mx] { *px = glow; } }
                    }
                }
            }
        }
    }

    // Stamp agent sprites
    for agent in &state.agents {
        let sprite = make_sprite(&agent.sprite_colors, agent.frame);
        let sx = agent.x.round() as i32;
        let sy = agent.y.round() as i32 - sprites::SPRITE_H as i32;
        stamp_sprite(&mut scene, &sprite, sx, sy);
    }
    scene
}

/// Render the entire Office tab into the given area.
///
/// Self-healing: if `bg` or `state.scene_*` don't match the current `area`,
/// regenerates them in-place. This eliminates the dual-source-of-truth between
/// any external resize-poll loop and the actual render area, so the canvas is
/// always sized to the live terminal — including on the very first frame.
pub fn render(frame: &mut Frame, area: Rect, bg: &mut renderer::PixelGrid, state: &mut OfficeState) {
    // Main layout: office scene (flex) | sidebar (26 cols)
    let layout = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(26),
    ]).split(area);

    // Self-heal: if bg/state size doesn't match `area`, regenerate.
    // Cheap path: width comparison; only regenerates on actual mismatch.
    let sidebar_w = 26u16;
    let target_w = area.width.saturating_sub(sidebar_w + 2) as usize;
    let target_h = area.height.saturating_sub(2) as usize * 2;
    let target_w = target_w.max(20);
    let target_h = target_h.max(10);
    let cur_w = if bg.is_empty() { 0 } else { bg[0].len() };
    let cur_h = bg.len();
    if cur_w != target_w || cur_h != target_h {
        *bg = generate_bg_for_area(area);
        update_scene_dimensions(state, area);
    }

    render_scene(frame, layout[0], bg, state);
    render_sidebar(frame, layout[1], state);

    // Overlays (rendered last, on top)
    if state.show_memo {
        render_memo_overlay(frame, area, state);
    }
    if state.show_help {
        render_help_overlay(frame, area);
    }
}

/// Render the pixel art office scene with speech bubbles and zone labels.
fn render_scene(frame: &mut Frame, area: Rect, bg: &renderer::PixelGrid, state: &OfficeState) {
    let scene = compose_scene(bg, state);

    // Scene block
    let block = Block::default()
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette::MUTED))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(palette::BG));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Render half-block pixels
    render_halfblock(&scene, inner, frame.buffer_mut());

    // Zone labels (as Paragraph overlays in the corners of the scene)
    let zones = [
        ("ENGINEERING", 1, 0, palette::ACCENT),
        ("COMMS", inner.width.saturating_sub(10), 0, palette::BLUE),
        ("RESEARCH", 1, inner.height / 2, palette::CYAN),
        ("BREAK ROOM", inner.width.saturating_sub(12), inner.height.saturating_sub(3), palette::YELLOW),
    ];
    for (label, x_off, y_off, color) in zones {
        let label_area = Rect::new(
            inner.x + x_off,
            inner.y + y_off,
            label.len() as u16 + 1,
            1,
        );
        if label_area.x + label_area.width <= inner.x + inner.width
            && label_area.y < inner.y + inner.height
        {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    label,
                    Style::default().fg(color).add_modifier(Modifier::DIM),
                )),
                label_area,
            );
        }
    }

    // Speech bubbles above each agent
    for (i, agent) in state.agents.iter().enumerate() {
        let is_focused = state.focused_agent == Some(i);
        // S94 T3: show last_message in bubble if fresh (< 30 ticks), else fall back to state label
        let task_display = if !agent.last_message.is_empty() && agent.message_age < 30 {
            let msg = &agent.last_message;
            if msg.chars().count() > 22 { format!("{}...", msg.chars().take(19).collect::<String>()) } else { msg.clone() }
        } else if agent.task.len() > 22 {
            format!("{}...", agent.task.chars().take(19).collect::<String>())
        } else if agent.task.is_empty() {
            agent.state.label().to_string()
        } else {
            agent.task.clone()
        };

        // Calculate bubble position in terminal coords
        // Pixel x -> terminal x is 1:1 (each pixel = 1 char width)
        // Pixel y -> terminal y is 2:1 (each 2 pixel rows = 1 terminal row)
        let term_x = (agent.x.round() as u16).saturating_sub(2) + inner.x;
        let agent_pixel_y = agent.y.round() as i32;
        let bubble_pixel_y = (agent_pixel_y - sprites::SPRITE_H as i32 - 3).max(0) as u16;
        let term_y = (bubble_pixel_y / 2) + inner.y;

        let bubble_width = (agent.name.len() + task_display.len() + 5) as u16;
        let bubble_area = Rect::new(
            term_x.min(inner.x + inner.width - bubble_width - 1),
            term_y.max(inner.y),
            bubble_width.min(inner.width),
            1,
        );

        if bubble_area.y < inner.y + inner.height && bubble_area.x < inner.x + inner.width {
            let border_color = if is_focused { palette::ACCENT } else { palette::MUTED };
            // S94 T3: fade bubble text — bright when fresh, dim when old
            let bubble_fresh = !agent.last_message.is_empty() && agent.message_age < 30;
            let msg_color = if bubble_fresh {
                // Fade: age 0-10 = FG, 10-20 = ACCENT_DIM, 20-30 = DIM
                if agent.message_age < 10 { palette::FG }
                else if agent.message_age < 20 { palette::ACCENT_DIM }
                else { palette::MUTED }
            } else {
                palette::DIM
            };
            let line = Line::from(vec![
                Span::styled(
                    &agent.name,
                    Style::default().fg(agent.state.color()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" \u{2502} ", Style::default().fg(border_color)),
                Span::styled(&task_display, Style::default().fg(msg_color)),
            ]);
            frame.render_widget(Paragraph::new(line), bubble_area);
        }
    }
}

/// Render the right sidebar: agents list, zone summary, stats.
/// When an agent is focused, shows expanded detail instead of list.
fn render_sidebar(frame: &mut Frame, area: Rect, state: &OfficeState) {
    // If focused, show detail panel for that agent
    if let Some(idx) = state.focused_agent {
        if let Some(agent) = state.agents.get(idx) {
            render_agent_detail(frame, area, agent, state);
            return;
        }
    }

    let layout = Layout::vertical([
        Constraint::Length(1),  // header
        Constraint::Min(0),    // agent list
        Constraint::Length(8), // zone summary
        Constraint::Length(8), // stats
    ]).split(area);

    // Header — show count with local/channel breakdown
    let local_count = state.agents.iter().filter(|a| a.agent_type == "local").count();
    let channel_count = state.agents.iter().filter(|a| a.agent_type == "channel").count();
    let human_count = state.agents.iter().filter(|a| a.agent_type == "human").count();
    let header_text = if human_count > 0 || channel_count > 0 {
        let mut parts = vec![format!("{} local", local_count)];
        if channel_count > 0 { parts.push(format!("{} channel", channel_count)); }
        if human_count > 0 { parts.push(format!("{} human", human_count)); }
        format!(" AGENTS ({})", parts.join(", "))
    } else {
        format!(" AGENTS ({})", state.agents.len())
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            header_text,
            Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD),
        )).style(Style::default().bg(palette::WARM_BG)),
        layout[0],
    );

    // Agent list
    let agent_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(palette::MUTED))
        .style(Style::default().bg(palette::WARM_BG));
    let agent_inner = agent_block.inner(layout[1]);
    frame.render_widget(agent_block, layout[1]);

    if state.agents.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" (no agents)", Style::default().fg(palette::MUTED))),
            agent_inner,
        );
    } else {
        let items: Vec<ListItem> = state.agents.iter().enumerate().map(|(i, agent)| {
            let is_focused = state.focused_agent == Some(i);
            let badge = match agent.agent_type.as_str() {
                "human" => " (human)",
                "channel" => " (channel)",
                _ => " (local)",
            };
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        " \u{25CF} ",
                        Style::default().fg(agent.state.color()),
                    ),
                    Span::styled(
                        &agent.name,
                        if is_focused {
                            Style::default().fg(palette::WHITE).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(palette::FG)
                        },
                    ),
                    Span::raw(" "),
                    Span::styled(
                        agent.state.label(),
                        Style::default().fg(agent.state.color()),
                    ),
                    Span::styled(
                        badge,
                        Style::default().fg(palette::MUTED),
                    ),
                ]),
            ];
            // Task line
            if !agent.task.is_empty() {
                let task_trunc = if agent.task.len() > 20 {
                    format!("{}...", agent.task.chars().take(17).collect::<String>())
                } else {
                    agent.task.clone()
                };
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(task_trunc, Style::default().fg(palette::DIM)),
                ]));
            }
            // Model line
            if !agent.model.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(&agent.model, Style::default().fg(palette::MUTED)),
                ]));
            }
            ListItem::new(lines).style(
                if is_focused {
                    Style::default().bg(Color::Rgb(40, 20, 10))
                } else {
                    Style::default()
                }
            )
        }).collect();
        frame.render_widget(List::new(items), agent_inner);
    }

    // Zone summary
    let zone_block = Block::default()
        .title(Span::styled(" ZONES", Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD)))
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(Style::default().fg(palette::MUTED))
        .style(Style::default().bg(palette::WARM_BG));
    let zone_inner = zone_block.inner(layout[2]);
    frame.render_widget(zone_block, layout[2]);

    let zone_items: Vec<ListItem> = Zone::ALL.iter().map(|zone| {
        let count = state.zone_count(*zone);
        ListItem::new(Line::from(vec![
            Span::styled(" \u{25CF} ", Style::default().fg(zone.color())),
            Span::styled(
                format!("{:<12}", zone.label()),
                Style::default().fg(palette::DIM),
            ),
            Span::styled(
                count.to_string(),
                Style::default().fg(if count > 0 { zone.color() } else { palette::MUTED }),
            ),
        ]))
    }).collect();
    frame.render_widget(List::new(zone_items), zone_inner);

    // Stats
    let stats_block = Block::default()
        .title(Span::styled(" STATS", Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD)))
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(Style::default().fg(palette::MUTED))
        .style(Style::default().bg(palette::WARM_BG));
    let stats_inner = stats_block.inner(layout[3]);
    frame.render_widget(stats_block, layout[3]);

    let stats = [
        ("Ticks", state.tick.to_string(), palette::FG),
        ("Active", state.active_count().to_string(), palette::GREEN),
        ("Idle", (state.agents.len() - state.active_count()).to_string(), palette::DIM),
        ("Errors", state.error_count().to_string(), palette::RED),
    ];
    let stat_items: Vec<ListItem> = stats.iter().map(|(k, v, color)| {
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<10}", k), Style::default().fg(palette::DIM)),
            Span::styled(v, Style::default().fg(*color)),
        ]))
    }).collect();
    frame.render_widget(List::new(stat_items), stats_inner);
}

/// Focused agent detail panel (replaces sidebar agent list).
fn render_agent_detail(frame: &mut Frame, area: Rect, agent: &state::OfficeAgent, state: &OfficeState) {
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", agent.name),
            Style::default().fg(palette::ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette::ACCENT))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(palette::WARM_BG));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Status
    lines.push(Line::from(vec![
        Span::styled(" Status  ", Style::default().fg(palette::DIM)),
        Span::styled(
            format!("\u{25CF} {}", agent.state.label()),
            Style::default().fg(agent.state.color()).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Zone
    lines.push(Line::from(vec![
        Span::styled(" Zone    ", Style::default().fg(palette::DIM)),
        Span::styled(agent.zone.label(), Style::default().fg(agent.zone.color())),
    ]));

    // Model
    if !agent.model.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Model   ", Style::default().fg(palette::DIM)),
            Span::styled(&agent.model, Style::default().fg(palette::FG)),
        ]));
    }

    lines.push(Line::raw(""));

    // Current task (full, wrapped)
    lines.push(Line::styled(" Task", Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD)));
    if agent.task.is_empty() {
        lines.push(Line::styled("  (none)", Style::default().fg(palette::MUTED)));
    } else {
        // Word-wrap task to sidebar width
        let max_w = inner.width.saturating_sub(2) as usize;
        let mut remaining = agent.task.as_str();
        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(max_w);
            { let safe_end = remaining.char_indices().nth(chunk_len).map(|(i,_)| i).unwrap_or(remaining.len()); lines.push(Line::styled(format!("  {}", &remaining[..safe_end]), Style::default().fg(palette::FG))); }
            let safe_end = remaining.char_indices().nth(chunk_len).map(|(i,_)| i).unwrap_or(remaining.len()); remaining = &remaining[safe_end..];
        }
    }

    lines.push(Line::raw(""));

    // Position
    lines.push(Line::from(vec![
        Span::styled(" Pos     ", Style::default().fg(palette::DIM)),
        Span::styled(
            format!("({:.0}, {:.0})", agent.x, agent.y),
            Style::default().fg(palette::MUTED),
        ),
    ]));

    // ID
    lines.push(Line::from(vec![
        Span::styled(" ID      ", Style::default().fg(palette::DIM)),
        Span::styled(&agent.id, Style::default().fg(palette::MUTED)),
    ]));

    // Source type
    let (type_label, type_color) = match agent.agent_type.as_str() {
        "human" => ("(human)", palette::GREEN),
        "channel" => ("(channel)", palette::BLUE),
        _ => ("(local)", palette::ACCENT_DIM),
    };
    lines.push(Line::from(vec![
        Span::styled(" Source  ", Style::default().fg(palette::DIM)),
        Span::styled(type_label, Style::default().fg(type_color)),
    ]));

    lines.push(Line::raw(""));

    // Quick stats at bottom
    lines.push(Line::styled(
        format!(" Tick {} \u{2502} {} agents \u{2502} {} active",
            state.tick, state.agents.len(), state.active_count()),
        Style::default().fg(palette::DIM),
    ));

    lines.push(Line::raw(""));
    lines.push(Line::styled(" Esc unfocus \u{2502} F next", Style::default().fg(palette::MUTED)));

    let items: Vec<ListItem> = lines.into_iter().map(ListItem::new).collect();
    frame.render_widget(List::new(items), inner);
}

/// Yesterday's Memo overlay.
fn render_memo_overlay(frame: &mut Frame, area: Rect, state: &OfficeState) {
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = (state.memo_text.len() as u16 + 5).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + 2;
    let overlay = Rect::new(x, y, width, height);

    let title = if state.memo_date.is_empty() {
        " YESTERDAY'S MEMO ".to_string()
    } else {
        format!(" MEMO \u{2502} {} ", state.memo_date)
    };

    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(palette::ACCENT).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette::MUTED))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(palette::WARM_BG));
    let inner = block.inner(overlay);
    frame.render_widget(Clear, overlay);
    frame.render_widget(block, overlay);

    let lines: Vec<Line> = if state.memo_text.is_empty() {
        vec![Line::styled(" No memo available", Style::default().fg(palette::MUTED))]
    } else {
        state.memo_text.iter().map(|line| {
            // Agent names at start of line get bright styling
            let style = if line.starts_with("Zeus") || line.starts_with("zeus")
                || line.starts_with("Hermes") || line.starts_with("Athena")
                || line.starts_with("fbsd") || line.starts_with("molty")
            {
                Style::default().fg(palette::FG)
            } else {
                Style::default().fg(palette::DIM)
            };
            Line::styled(format!(" {}", line), style)
        }).collect()
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Help overlay with keyboard shortcuts.
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let width = 36u16.min(area.width.saturating_sub(4));
    let height = 10u16.min(area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(width + 28);
    let y = area.y + 2;
    let overlay = Rect::new(x, y, width, height);

    let block = Block::default()
        .title(Span::styled(" KEYBOARD ", Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette::MUTED))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(palette::WARM_BG));
    let inner = block.inner(overlay);
    frame.render_widget(Clear, overlay);
    frame.render_widget(block, overlay);

    let keys = [
        ("M", "Toggle yesterday's memo"),
        ("F", "Cycle focus between agents"),
        ("R", "Force reconnect"),
        ("?", "Toggle this help"),
        ("Esc", "Close overlays / unfocus"),
    ];
    let items: Vec<ListItem> = keys.iter().map(|(k, desc)| {
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<5}", k), Style::default().fg(palette::ACCENT_DIM).add_modifier(Modifier::BOLD)),
            Span::styled(*desc, Style::default().fg(palette::DIM)),
        ]))
    }).collect();
    frame.render_widget(List::new(items), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::state::OfficeState;

    #[test]
    fn test_compose_scene_no_agents() {
        let bg = background::generate(80, 40);
        let state = OfficeState::new();
        let scene = compose_scene(&bg, &state);
        assert_eq!(scene.len(), 40);
    }

    #[test]
    fn test_compose_scene_with_agents() {
        let bg = background::generate(80, 40);
        let mut state = OfficeState::new();
        state.agents.push(state::OfficeAgent::new("zeus100", "Zeus100"));
        let scene = compose_scene(&bg, &state);
        assert_eq!(scene.len(), 40);
        assert_eq!(scene[0].len(), 80);
    }

    /// Self-heal: render() must regenerate bg+state when area doesn't match.
    /// Repro for the fullscreen-regression bug — without self-heal, a stale
    /// 80×40 bg would render only ~30% of a larger terminal.
    #[test]
    fn test_render_self_heals_on_size_mismatch() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut bg = background::generate(80, 40); // stale default
        let mut state = OfficeState::new();
        assert_eq!(state.scene_width, 80);
        assert_eq!(state.scene_height, 40);

        // Simulate a 200×60 terminal area (much larger than 80×40 default).
        let area = Rect::new(0, 0, 200, 60);
        let mut buffer = Buffer::empty(area);
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, area, &mut bg, &mut state))
            .unwrap();
        let _ = buffer; // keep import alive for clarity

        // After render, bg and state should match the new area.
        // sidebar = 26 cols, borders = 2 → scene_w = 200 - 28 = 172
        assert_eq!(state.scene_width, 172);
        // height halfblock: (60 - 2) * 2 = 116
        assert_eq!(state.scene_height, 116);
        assert_eq!(bg.len(), 116);
        assert_eq!(bg[0].len(), 172);
    }

    /// Self-heal must be a no-op when sizes already match (fast path).
    #[test]
    fn test_render_no_regen_when_size_matches() {
        use ratatui::layout::Rect;

        // Pre-size bg+state for a 200×60 area.
        let area = Rect::new(0, 0, 200, 60);
        let mut bg = generate_bg_for_area(area);
        let mut state = OfficeState::new();
        update_scene_dimensions(&mut state, area);
        assert_eq!(state.scene_width, 172);

        // Capture pointer-identity-ish via length checks before/after.
        let before_h = bg.len();
        let before_w = bg[0].len();

        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, area, &mut bg, &mut state))
            .unwrap();

        // Sizes unchanged (fast path took the no-regen branch).
        assert_eq!(bg.len(), before_h);
        assert_eq!(bg[0].len(), before_w);
        assert_eq!(state.scene_width, 172);
        assert_eq!(state.scene_height, 116);
    }
}
