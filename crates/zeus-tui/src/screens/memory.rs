#![allow(dead_code)]
//! Memory screen — log viewer with kind filtering
//! Owner: mikes-Mac-mini (feat/s68-tui-core)

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use crate::app::{App, LogEntry, LogKind};
use crate::theme;
use super::{Action, Screen};

pub struct MemoryScreen;

impl Screen for MemoryScreen {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        render_filter_bar(frame, chunks[0]);
        render_log(frame, chunks[1], app);
    }

    fn handle_input(&self, key: crossterm::event::KeyEvent, app: &mut App) -> Action {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up   | KeyCode::Char('k') => app.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
            KeyCode::Char('g') => app.scroll_offset = 0,
            KeyCode::Char('G') => app.scroll_offset = app.log.len().saturating_sub(1),
            KeyCode::Esc => return Action::SwitchTab(0),
            _ => {}
        }
        Action::Continue
    }
}

fn render_filter_bar(frame: &mut Frame, area: Rect) {
    let kinds = ["ALL", "OUTPUT", "SHELL", "SEARCH", "SPAWN", "MEM", "SYS"];
    let spans: Vec<Span> = kinds.iter().enumerate().map(|(i, k)| {
        let style = if i == 0 {
            Style::default().fg(theme::HIGHLIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        Span::styled(format!(" [{k}] "), style)
    }).collect();

    let block = Block::default()
        .title(" Filter ")
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER));

    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let height = area.height.saturating_sub(2) as usize;
    let total = app.log.len();
    let offset = app.scroll_offset.min(total.saturating_sub(1));
    let visible = &app.log[offset..total.min(offset + height)];

    let items: Vec<ListItem> = visible.iter().map(|entry| {
        ListItem::new(format_entry(entry))
    }).collect();

    let scroll_info = if total > 0 {
        format!(" {}/{} ", offset + 1, total)
    } else {
        " empty ".to_string()
    };

    let block = Block::default()
        .title(format!(" ◎ Memory Log {scroll_info}"))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER));

    frame.render_widget(List::new(items).block(block), area);
}

fn format_entry(entry: &LogEntry) -> Line<'static> {
    let (sym, color) = kind_style(&entry.kind);

    let ts = Span::styled(
        format!("{} ", entry.timestamp),
        Style::default().fg(theme::DIM),
    );
    let kind_span = Span::styled(
        format!("{sym} "),
        Style::default().fg(color),
    );
    let content = Span::styled(
        entry.content.clone(),
        Style::default().fg(theme::TEXT),
    );

    if let Some(detail) = &entry.detail {
        let det = Span::styled(
            format!(" — {detail}"),
            Style::default().fg(theme::DIM),
        );
        Line::from(vec![ts, kind_span, content, det])
    } else {
        Line::from(vec![ts, kind_span, content])
    }
}

fn kind_style(kind: &LogKind) -> (&'static str, ratatui::style::Color) {
    match kind {
        LogKind::AgentOutput => ("◈", theme::ACCENT),
        LogKind::Shell       => ("$", theme::SUCCESS),
        LogKind::WebSearch   => ("⌕", theme::HIGHLIGHT),
        LogKind::Spawn       => ("⊕", theme::WARNING),
        LogKind::Memory      => ("◎", theme::ACCENT),
        LogKind::System      => ("·", theme::DIM),
    }
}
