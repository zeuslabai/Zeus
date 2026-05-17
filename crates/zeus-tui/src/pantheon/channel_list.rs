//! Channel sidebar widget for Pantheon.
//!
//! Renders the left-hand column showing all joined channels with:
//! - Active channel highlighting (inverted style)
//! - Unread message counts as `(N)` suffix
//! - Channel type prefix: `#` for standard channels

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem};

use super::app::PantheonApp;

/// Render the channel list sidebar into the given area.
pub fn render_channel_list(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let items: Vec<ListItem> = app
        .channels
        .iter()
        .enumerate()
        .map(|(idx, channel)| {
            let is_active = idx == app.active_channel;

            // Build label: channel name + unread count
            let mut label = channel.name.clone();
            if channel.unread > 0 {
                label.push_str(&format!(" ({})", channel.unread));
            }

            // Style: active channel gets inverted colors + bold,
            // channels with unreads get bold, others are dim.
            let style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if channel.unread > 0 {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            ListItem::new(Span::styled(label, style))
        })
        .collect();

    let title = if app.connected {
        "Channels"
    } else {
        "Channels [offline]"
    };

    let list = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::super::app::*;

    #[test]
    fn test_channel_list_builds_without_panic() {
        // Smoke test: PantheonApp::new constructs with default channels,
        // and the channel list should reference them without out-of-bounds.
        let app = PantheonApp::new("test".to_string());
        assert!(!app.channels.is_empty());
        assert_eq!(app.active_channel, 0);
        assert!(app.active_channel().is_some());
    }

    #[test]
    fn test_unread_count_on_channels() {
        let mut app = PantheonApp::new("test".to_string());
        app.channels[0].unread = 5;
        assert_eq!(app.channels[0].unread, 5);
        assert_eq!(app.channels[1].unread, 0);
    }
}
