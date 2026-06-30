use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Production TopBar — matches JSX TopBar (line 49).
/// Renders: `ZEUS │ host:port │ ○ conn │ [ctx bar] % │ Ctrl+K palette │ Ctrl+C quit`
pub struct ProdTopBar {
    pub hostname: String,
    pub port: u16,
    pub conn_state: ConnState,
    pub ctx_percent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connected,
    Connecting,
    Disconnected,
}

impl ConnState {
    pub fn glyph(self) -> &'static str {
        match self {
            ConnState::Connected => "●",
            ConnState::Connecting => "◐",
            ConnState::Disconnected => "○",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ConnState::Connected => "connected",
            ConnState::Connecting => "connecting",
            ConnState::Disconnected => "disconnected",
        }
    }

    pub fn color(self) -> ratatui::style::Color {
        match self {
            ConnState::Connected => theme::GREEN,
            ConnState::Connecting => theme::YELLOW,
            ConnState::Disconnected => theme::RED,
        }
    }
}

impl Widget for ProdTopBar {
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

        // ZEUS logo. Keep the glyph flush-left; a leading padding cell makes
        // narrow/cropped renders read as `EUS` at the left edge.
        spans.push(Span::styled(
            "ZEUS ",
            Style::default()
                .fg(theme::FIRE_ORANGE)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));

        // host:port
        spans.push(Span::styled(
            format!("{}:{}", self.hostname, self.port),
            Style::default().fg(theme::DIM),
        ));
        spans.push(Span::styled(" │ ", Style::default().fg(theme::MUTED)));

        // Connection state
        spans.push(Span::styled(
            format!("{} ", self.conn_state.glyph()),
            Style::default().fg(self.conn_state.color()),
        ));
        spans.push(Span::styled(
            self.conn_state.label(),
            Style::default().fg(self.conn_state.color()),
        ));
        spans.push(Span::styled(" │ ", Style::default().fg(theme::MUTED)));

        // Context bar
        spans.push(Span::styled(
            "[",
            Style::default().fg(theme::MUTED),
        ));
        spans.push(Span::styled(
            format!("{}%", self.ctx_percent),
            Style::default()
                .fg(if self.ctx_percent > 80 { theme::YELLOW } else { theme::DIM })
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "]",
            Style::default().fg(theme::MUTED),
        ));

        // Right side: keybind hints
        spans.push(Span::raw("  "));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "Ctrl+K",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" palette ", Style::default().fg(theme::DIM)));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "Ctrl+C",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" quit", Style::default().fg(theme::DIM)));

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}
