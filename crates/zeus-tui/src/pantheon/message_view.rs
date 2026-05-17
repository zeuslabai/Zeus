//! Pantheon message_view module — see PRD for specification.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::app::{IrcMessage, MessageKind};

pub fn render_message_view(f: &mut Frame<'_>, area: Rect, messages: &[IrcMessage]) {
    let block = Block::default().title("Messages").borders(Borders::ALL);

    if messages.is_empty() {
        let paragraph = Paragraph::new("No messages yet")
            .block(block)
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
        return;
    }

    let lines: Vec<Line> = messages
        .iter()
        .map(|msg| render_message_line(msg))
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn render_message_line(message: &IrcMessage) -> Line<'static> {
    let prefix = match message.kind {
        MessageKind::Action => format!("* {}", message.nick),
        MessageKind::Notice => format!("-{}-", message.nick),
        MessageKind::System => String::from("[system]"),
        MessageKind::Join => format!("→ {} joined", message.nick),
        MessageKind::Part => format!("← {} left", message.nick),
        MessageKind::Topic => format!("[topic] {}", message.nick),
        MessageKind::Normal => format!("<{}>", message.nick),
    };

    let mut spans = vec![Span::styled(prefix, Style::default().fg(color_for_nick(&message.nick)).add_modifier(Modifier::BOLD)), Span::raw(" ")];
    spans.push(Span::raw(message.content.clone()));
    Line::from(spans)
}

fn color_for_nick(nick: &str) -> Color {
    let mut hash: u64 = 0;
    for byte in nick.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
    }

    match hash % 6 {
        0 => Color::Cyan,
        1 => Color::Green,
        2 => Color::Yellow,
        3 => Color::Magenta,
        4 => Color::Blue,
        _ => Color::LightRed,
    }
}
