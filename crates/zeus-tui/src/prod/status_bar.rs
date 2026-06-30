use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Production StatusBar — matches JSX HintBar (line 396).
/// Bottom bar: `● ready │ queue: N │ ↑↓ scroll │ Tab switch │ / commands │ ? help`
pub struct ProdStatusBar {
    pub queue_count: usize,
    pub is_streaming: bool,
}

impl Widget for ProdStatusBar {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height == 0 {
            return;
        }

        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(theme::BG_PANEL));
        }

        let mut spans = Vec::new();

        // Status indicator
        if self.is_streaming {
            spans.push(Span::styled(
                " ◉ ",
                Style::default().fg(theme::FIRE_ORANGE),
            ));
            spans.push(Span::styled(
                "streaming ",
                Style::default().fg(theme::FIRE_ORANGE),
            ));
        } else {
            spans.push(Span::styled(
                " ● ",
                Style::default().fg(theme::GREEN),
            ));
            spans.push(Span::styled("ready ", Style::default().fg(theme::DIM)));
        }
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));

        // Queue count
        if self.queue_count > 0 {
            spans.push(Span::styled(
                format!("queue: {} ", self.queue_count),
                Style::default().fg(theme::YELLOW),
            ));
            spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        }

        // Navigation hints
        let hints: &[(&str, &str)] = &[
            ("↑↓", "scroll"),
            ("Tab", "switch"),
            ("/", "commands"),
            ("?", "help"),
        ];

        for (i, (key, label)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
            }
            spans.push(Span::styled(
                *key,
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(theme::DIM),
            ));
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}
