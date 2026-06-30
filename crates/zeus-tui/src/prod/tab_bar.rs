use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Primary tabs — matches JSX PRIMARY_TABS (line 20).
pub const PRIMARY_TABS: &[TabDef] = &[
    TabDef { id: "chat", name: "chat", glyph: "▸" },
    TabDef { id: "office", name: "office", glyph: "◇" },
    TabDef { id: "pantheon", name: "pantheon", glyph: "◈" },
    TabDef { id: "tools", name: "tools", glyph: "⚙" },
    TabDef { id: "memory", name: "memory", glyph: "▤" },
    TabDef { id: "channels", name: "channels", glyph: "⇌" },
    TabDef { id: "wallet", name: "wallet", glyph: "⊟" },
    TabDef { id: "approvals", name: "approvals", glyph: "✓" },
    TabDef { id: "settings", name: "settings", glyph: "⊕" },
    TabDef { id: "advanced", name: "more…", glyph: "▸▸" },
];

pub struct TabDef {
    pub id: &'static str,
    pub name: &'static str,
    pub glyph: &'static str,
}

/// Production TabBar — matches JSX TabBar (line 93).
/// Active tab highlighted with accent pill (bg + bold), not just underline.
pub struct ProdTabBar {
    pub active_idx: usize,
}

impl Widget for ProdTabBar {
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
        spans.push(Span::raw(" "));

        for (i, tab) in PRIMARY_TABS.iter().enumerate() {
            let is_active = i == self.active_idx;

            if is_active {
                // Active tab: accent glyph + bright name on highlight bg
                spans.push(Span::styled(
                    format!(" {} ", tab.glyph),
                    Style::default()
                        .fg(theme::ACCENT)
                        .bg(theme::BG_HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{} ", tab.name),
                    Style::default()
                        .fg(theme::TEXT_BRIGHT)
                        .bg(theme::BG_HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                // Inactive tab
                spans.push(Span::styled(
                    format!(" {} ", tab.glyph),
                    Style::default().fg(theme::MUTED),
                ));
                spans.push(Span::styled(
                    format!("{} ", tab.name),
                    Style::default().fg(theme::DIM),
                ));
            }
        }

        // Right side: nav hints
        spans.push(Span::raw("  "));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "Tab",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" to switch  ·  ", Style::default().fg(theme::DIM)));
        spans.push(Span::styled(
            "⇧Tab",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" back  ·  ", Style::default().fg(theme::DIM)));
        spans.push(Span::styled(
            ":",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" palette", Style::default().fg(theme::DIM)));

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}
