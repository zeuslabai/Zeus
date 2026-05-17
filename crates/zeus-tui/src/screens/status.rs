#![allow(dead_code)]
//! Status screen — fleet health + gateway connection info
//! Owner: mikes-Mac-mini (feat/s68-tui-core)

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use crate::app::{App, AgentStatus, ChannelStatus};
use crate::theme;
use super::{Action, Screen};

pub struct StatusScreen;

impl Screen for StatusScreen {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),  // gateway header
                Constraint::Min(0),     // agents + channels
            ])
            .split(area);

        render_gateway(frame, chunks[0], app);

        let lower = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        render_agents(frame, lower[0], app);
        render_channels(frame, lower[1], app);
    }

    fn handle_input(&self, key: crossterm::event::KeyEvent, app: &mut App) -> Action {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
            KeyCode::Esc => return Action::SwitchTab(0),
            _ => {}
        }
        Action::Continue
    }
}

fn render_gateway(frame: &mut Frame, area: Rect, app: &App) {
    let (status_sym, status_style) = if app.connected {
        ("● CONNECTED", Style::default().fg(theme::SUCCESS))
    } else {
        ("○ DISCONNECTED", Style::default().fg(theme::ERROR))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Gateway: ", Style::default().fg(theme::LABEL)),
            Span::styled(&app.gateway_url, Style::default().fg(theme::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("Status:  ", Style::default().fg(theme::LABEL)),
            Span::styled(status_sym, status_style),
        ]),
        Line::from(vec![
            Span::styled("Tick:    ", Style::default().fg(theme::LABEL)),
            Span::styled(app.tick_count.to_string(), Style::default().fg(theme::DIM)),
        ]),
    ];

    let block = Block::default()
        .title(" ◈ Gateway Status ")
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_agents(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.agents.iter().enumerate().map(|(i, agent)| {
        let (sym, style) = match agent.status {
            AgentStatus::Running   => ("▶", Style::default().fg(theme::SUCCESS)),
            AgentStatus::Idle      => ("◌", Style::default().fg(theme::DIM)),
            AgentStatus::Completed => ("✓", Style::default().fg(theme::ACCENT)),
            AgentStatus::Error     => ("✗", Style::default().fg(theme::ERROR)),
        };

        let sel = if i == app.selected_agent {
            Style::default().fg(theme::HIGHLIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT)
        };

        let line = Line::from(vec![
            Span::styled(format!("{sym} "), style),
            Span::styled(format!("{:<16}", agent.name), sel),
            Span::styled(
                format!("{:>3}%", agent.progress),
                Style::default().fg(theme::DIM),
            ),
        ]);
        ListItem::new(line)
    }).collect();

    let block = Block::default()
        .title(" ⚙ Agents ")
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER));

    frame.render_widget(List::new(items).block(block), area);
}

fn render_channels(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.channels.iter().map(|ch| {
        let style = match ch.status {
            ChannelStatus::Connected => Style::default().fg(theme::SUCCESS),
            ChannelStatus::Relay     => Style::default().fg(theme::ACCENT),
            ChannelStatus::Offline   => Style::default().fg(theme::ERROR),
        };

        let unread = if ch.unread > 0 {
            format!(" [{}]", ch.unread)
        } else {
            String::new()
        };

        let line = Line::from(vec![
            Span::styled(format!("{} ", ch.icon), style),
            Span::styled(format!("{:<14}", ch.name), Style::default().fg(theme::TEXT)),
            Span::styled(unread, Style::default().fg(theme::ACCENT)),
        ]);
        ListItem::new(line)
    }).collect();

    let block = Block::default()
        .title(" ⌁ Channels ")
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER));

    frame.render_widget(List::new(items).block(block), area);
}
