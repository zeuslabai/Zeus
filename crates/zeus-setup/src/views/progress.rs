//! Progress view — step list + scrolling log + gauge bar

use crate::app::{App, AppView, StepStatus};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Padding, Paragraph, Wrap},
};

pub fn render(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();

    // Background
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    // Layout: header + steps + log + gauge + hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title bar
            Constraint::Min(6),    // Steps
            Constraint::Min(8),    // Log output
            Constraint::Length(3), // Gauge bar
            Constraint::Length(1), // Hint
        ])
        .split(area);

    // Title bar
    let elapsed = app.operation_start.map(|s| s.elapsed()).unwrap_or_default();
    let title = format!(" {} — {:.1}s ", app.operation_name, elapsed.as_secs_f64());
    let title_block = Block::default()
        .title(Line::from(vec![
            Span::styled("⚡", Style::default().fg(theme.warning)),
            Span::styled(
                &title,
                Style::default()
                    .fg(theme.title)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));
    frame.render_widget(title_block, chunks[0]);

    // Steps list
    render_steps(frame, app, chunks[1]);

    // Log output
    render_log(frame, app, chunks[2]);

    // Gauge bar
    render_gauge(frame, app, chunks[3]);

    // Hint line
    let hint_text = if app.view == AppView::Finished {
        "  Enter Return to menu  q Quit"
    } else {
        "  ↑/↓ Scroll log  q Cancel"
    };
    let hint = Paragraph::new(Line::from(Span::styled(
        hint_text,
        Style::default().fg(theme.muted),
    )));
    frame.render_widget(hint, chunks[4]);
}

fn render_steps(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Steps ",
            Style::default()
                .fg(theme.title)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(theme.border))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    let spinner = format!("{}", app.spinner_char());
    for step in &app.steps {
        let (icon, icon_style) = match &step.status {
            StepStatus::Pending => ("·", Style::default().fg(theme.muted)),
            StepStatus::Running => (
                spinner.as_str(),
                Style::default()
                    .fg(theme.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            StepStatus::Done => (
                "✓",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
            StepStatus::Failed => (
                "✗",
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ),
            StepStatus::Warning => (
                "!",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
        };

        let name_style = match &step.status {
            StepStatus::Running => Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            StepStatus::Failed => Style::default().fg(theme.error),
            StepStatus::Done => Style::default().fg(theme.fg),
            _ => Style::default().fg(theme.muted),
        };

        let mut spans = vec![
            Span::styled(format!("  {} ", icon), icon_style),
            Span::styled(&step.name, name_style),
        ];

        if !step.message.is_empty() {
            spans.push(Span::styled(
                format!(" — {}", step.message),
                Style::default().fg(theme.muted),
            ));
        }

        lines.push(Line::from(spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Preparing...",
            Style::default().fg(theme.muted),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_log(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let log_title = format!(" Output ({} lines) ", app.log_buffer.len());
    let block = Block::default()
        .title(Span::styled(log_title, Style::default().fg(theme.muted)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let total = app.log_buffer.len();
    let start = if total > visible_height {
        app.log_scroll
            .saturating_sub(visible_height)
            .min(total - visible_height)
    } else {
        0
    };
    let end = (start + visible_height).min(total);

    let lines: Vec<Line> = app.log_buffer[start..end]
        .iter()
        .map(|l| {
            let style = if l.contains("error") || l.contains("Error") || l.contains("FAILED") {
                Style::default().fg(theme.error)
            } else if l.contains("warning") || l.contains("Warning") {
                Style::default().fg(theme.warning)
            } else if l.contains("Compiling") || l.contains("Downloading") {
                Style::default().fg(theme.highlight)
            } else if l.contains("Finished") || l.contains("✓") {
                Style::default().fg(theme.success)
            } else {
                Style::default().fg(theme.muted)
            };
            Line::from(Span::styled(l.as_str(), style))
        })
        .collect();

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn render_gauge(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let label = if app.view == AppView::Finished {
        if app.finish_success {
            format!(
                "✓ {} ({:.1}s)",
                app.finish_summary,
                app.finish_elapsed.as_secs_f64()
            )
        } else {
            format!("✗ {}", app.finish_summary)
        }
    } else {
        format!("{}%  {}", app.overall_progress, app.progress_message)
    };

    let gauge_color = if app.view == AppView::Finished {
        if app.finish_success {
            theme.success
        } else {
            theme.error
        }
    } else {
        theme.highlight
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.bg)),
        )
        .gauge_style(Style::default().fg(gauge_color).bg(theme.border))
        .percent(app.overall_progress as u16)
        .label(Span::styled(
            label,
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(gauge, area);
}
