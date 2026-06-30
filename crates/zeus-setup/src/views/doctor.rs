//! Doctor results view

use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
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
    let pass_count = app.doctor_checks.iter().filter(|c| c.ok).count();
    let fail_count = app.doctor_checks.len() - pass_count;
    let title = format!(
        " Diagnostics — {} passed, {} failed ",
        pass_count, fail_count
    );

    let block = Block::default()
        .title(Span::styled(
            &title,
            Style::default()
                .fg(if fail_count > 0 {
                    theme.warning
                } else {
                    theme.success
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));
    frame.render_widget(block, chunks[0]);

    // Check results
    let results_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border))
        .padding(Padding::horizontal(2))
        .style(Style::default().bg(theme.bg));

    let inner = results_block.inner(chunks[1]);
    frame.render_widget(results_block, chunks[1]);

    let lines: Vec<Line> = app
        .doctor_checks
        .iter()
        .map(|check| {
            let (icon, color) = if check.ok {
                ("✓", theme.success)
            } else {
                ("✗", theme.error)
            };
            Line::from(vec![
                Span::styled(
                    format!("  {} ", icon),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<25}", check.name), Style::default().fg(theme.fg)),
                Span::styled(&check.detail, Style::default().fg(theme.muted)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);

    // Hint
    let hint = Paragraph::new(Line::from(Span::styled(
        "  Enter Return to menu  q Quit",
        Style::default().fg(theme.muted),
    )));
    frame.render_widget(hint, chunks[2]);
}
