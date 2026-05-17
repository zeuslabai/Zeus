//! Pantheon chat module — see PRD for specification.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use super::app::PantheonApp;
use super::message_view::render_message_view;

pub fn render_chat(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Min(40),
            Constraint::Length(24),
        ])
        .split(area);

    render_channel_list(f, layout[0], app);
    render_center_panel(f, layout[1], app);
    render_user_list(f, layout[2], app);
}

fn render_channel_list(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let items = app
        .channels
        .iter()
        .enumerate()
        .map(|(idx, channel)| {
            let mut label = channel.name.clone();
            if channel.unread > 0 {
                label.push_str(&format!(" ({})", channel.unread));
            }
            if idx == app.active_channel {
                label = format!("> {}", label);
            }
            ListItem::new(label)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).block(Block::default().title("Channels").borders(Borders::ALL));
    f.render_widget(list, area);
}

fn render_center_panel(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)])
        .split(area);

    if let Some(channel) = app.active_channel() {
        let header = Paragraph::new(channel.topic.clone().unwrap_or_default())
            .block(Block::default().title(channel.name.clone()).borders(Borders::ALL));
        f.render_widget(header, chunks[0]);
        render_message_view(f, chunks[0], &channel.messages);
    }

    let input = Paragraph::new("Type a message...")
        .block(Block::default().title("Input").borders(Borders::ALL));
    f.render_widget(input, chunks[1]);
}

fn render_user_list(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let users = app
        .active_channel()
        .map(|ch| {
            ch.users
                .iter()
                .map(|u| ListItem::new(format!("{}{}", u.mode.map(|m| m.to_string()).unwrap_or_default(), u.nick)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let list = List::new(users).block(Block::default().title("Users").borders(Borders::ALL));
    f.render_widget(list, area);
}
