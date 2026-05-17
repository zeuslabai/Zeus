//! Main menu view — interactive arrow-key navigation

use crate::app::{App, AppView};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

pub fn render(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();

    // Background
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    // Center the menu
    let menu_width = 56u16;
    let items = app.menu_items();
    let menu_height = items.len() as u16 + 8; // items + header + footer + padding

    let centered = centered_rect(menu_width, menu_height, area);

    // Title
    let title = match &app.view {
        AppView::MainMenu => format!(" Zeus Setup  v{} ", App::version()),
        AppView::InstallMenu => " Install Zeus ".to_string(),
        AppView::BuildMenu => " Build from Source ".to_string(),
        AppView::DeployMenu => " Deploy to Fleet ".to_string(),
        AppView::McpMenu => " Configure MCP ".to_string(),
        AppView::ServiceMenu => " Manage Services ".to_string(),
        _ => String::new(),
    };

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled("⚡", Style::default().fg(theme.warning)),
            Span::styled(
                &title,
                Style::default()
                    .fg(theme.title)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(centered);
    frame.render_widget(block, centered);

    // Layout: items + hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Menu items
            Constraint::Length(1), // Hint line
        ])
        .split(inner);

    // Menu items
    let mut lines = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let is_selected = i == app.menu_cursor;
        let line = if is_selected {
            Line::from(vec![
                Span::styled(
                    "  ▸ ",
                    Style::default()
                        .fg(theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    *item,
                    Style::default()
                        .fg(theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(vec![
                Span::raw("    "),
                Span::styled(*item, Style::default().fg(theme.fg)),
            ])
        };
        lines.push(line);
    }

    let menu = Paragraph::new(lines);
    frame.render_widget(menu, chunks[0]);

    // Hint line
    let back_hint = if app.view != AppView::MainMenu {
        "  Esc Back  "
    } else {
        ""
    };
    let hint = Paragraph::new(Line::from(vec![Span::styled(
        format!("  ↑/↓ Navigate  Enter Select  q Quit{}", back_hint),
        Style::default().fg(theme.muted),
    )]))
    .alignment(Alignment::Left);
    frame.render_widget(hint, chunks[1]);
}

/// Helper to create a centered rectangle
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
