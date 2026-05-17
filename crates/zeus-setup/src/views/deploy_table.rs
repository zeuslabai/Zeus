//! Deploy summary table view

use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Row, Table},
};

pub fn render(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();

    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // Title
    let ok_count = app
        .deploy_results
        .iter()
        .filter(|r| r.status.contains("OK") || r.status.contains("ok"))
        .count();
    let total = app.deploy_results.len();
    let title = format!(" Deploy Summary — {}/{} succeeded ", ok_count, total);

    let block = Block::default()
        .title(Span::styled(
            &title,
            Style::default()
                .fg(if ok_count == total {
                    theme.success
                } else {
                    theme.warning
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));
    frame.render_widget(block, chunks[0]);

    // Table
    let header = Row::new(vec!["Host", "IP", "OS", "Status", "Time"]).style(
        Style::default()
            .fg(theme.title)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .deploy_results
        .iter()
        .map(|r| {
            let status_style = if r.status.contains("OK") || r.status.contains("ok") {
                Style::default().fg(theme.success)
            } else {
                Style::default().fg(theme.error)
            };
            Row::new(vec![
                r.host.clone(),
                r.ip.clone(),
                r.os.clone(),
                r.status.clone(),
                format!("{:.1}s", r.duration.as_secs_f64()),
            ])
            .style(status_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(theme.border))
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.bg)),
    );

    frame.render_widget(table, chunks[1]);

    // Hint
    let hint = Paragraph::new(Line::from(Span::styled(
        "  Enter Return to menu  q Quit",
        Style::default().fg(theme.muted),
    )));
    frame.render_widget(hint, chunks[2]);
}
