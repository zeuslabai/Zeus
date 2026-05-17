//! User list sidebar widget for Pantheon.
//!
//! Renders the right-hand column showing users in the active channel with:
//! - Mode prefixes: `@` for ops, `+` for voiced, space for normal
//! - Online/idle status indicator: `●` online, `○` offline/idle
//! - Sorted: ops first, then voiced, then normal; alphabetical within tiers
//! - Nick coloring via `nick_color::nick_color()`

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem};

use super::app::{IrcUser, PantheonApp};
use super::nick_color::nick_color;

/// Render the user list sidebar into the given area.
pub fn render_user_list(f: &mut Frame<'_>, area: Rect, app: &PantheonApp) {
    let items: Vec<ListItem> = match app.active_channel() {
        Some(channel) => {
            // Sort users: ops (@) first, then voiced (+), then normal.
            // Alphabetical within each tier (case-insensitive).
            let mut sorted: Vec<&IrcUser> = channel.users.iter().collect();
            sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

            sorted
                .iter()
                .map(|user| render_user_item(user))
                .collect()
        }
        None => vec![ListItem::new(Span::styled(
            "(no channel)",
            Style::default().fg(Color::DarkGray),
        ))],
    };

    let user_count = app
        .active_channel()
        .map(|ch| ch.users.len())
        .unwrap_or(0);

    let title = format!("Users ({})", user_count);

    let list = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(list, area);
}

/// Render a single user entry as a styled `ListItem`.
fn render_user_item(user: &IrcUser) -> ListItem<'static> {
    // Status dot: ● online, ○ offline/idle
    let status_dot = if user.is_online { "● " } else { "○ " };
    let status_color = if user.is_online {
        Color::Green
    } else {
        Color::DarkGray
    };

    // Mode prefix: @ for ops, + for voiced, space for normal
    let mode_prefix = match user.mode {
        Some(m) => format!("{}", m),
        None => " ".to_string(),
    };

    let color = nick_color(&user.nick);

    let mut spans = vec![
        Span::styled(status_dot, Style::default().fg(status_color)),
        Span::styled(
            mode_prefix,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(user.nick.clone(), Style::default().fg(color)),
    ];

    // Append role tag if present (e.g. " [coordinator]")
    if let Some(ref role) = user.role {
        spans.push(Span::styled(
            format!(" [{}]", role),
            Style::default().fg(Color::DarkGray),
        ));
    }

    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::super::app::*;

    #[test]
    fn test_user_sort_key_ops_first() {
        let op = IrcUser::op("alice");
        let voiced = IrcUser::voiced("bob");
        let normal = IrcUser::new("charlie");

        assert!(op.sort_key() < voiced.sort_key());
        assert!(voiced.sort_key() < normal.sort_key());
    }

    #[test]
    fn test_user_sort_key_alphabetical_within_tier() {
        let a = IrcUser::new("alice");
        let b = IrcUser::new("bob");
        let z = IrcUser::new("Zeus100");

        assert!(a.sort_key() < b.sort_key());
        // Case-insensitive: "zeus100" < everything starting with z...
        assert!(b.sort_key() < z.sort_key());
    }

    #[test]
    fn test_user_mode_constructors() {
        let op = IrcUser::op("zeus100");
        assert_eq!(op.mode, Some('@'));
        assert!(op.is_online);

        let voiced = IrcUser::voiced("zeus107");
        assert_eq!(voiced.mode, Some('+'));

        let normal = IrcUser::new("random");
        assert_eq!(normal.mode, None);
    }
}
