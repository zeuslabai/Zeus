use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Stub tab panel — clean intentional placeholder for tabs not yet built.
/// Matches the spec: `<tab> — Phase 2 in progress`
pub struct StubTab {
    pub name: &'static str,
    pub glyph: &'static str,
}

impl Widget for StubTab {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height < 3 || area.width < 20 {
            return;
        }

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(theme::BG));
            }
        }

        // Center the stub message
        let center_y = area.y + area.height / 2;
        let msg = format!("{} {} — Phase 2 in progress", self.glyph, self.name);
        let center_x = area.x + (area.width.saturating_sub(msg.len() as u16)) / 2;

        buf.set_string(
            center_x,
            center_y,
            &msg,
            Style::default()
                .fg(theme::DIM)
                .add_modifier(Modifier::ITALIC),
        );

        // Subtle border hint
        let sub_msg = "this tab will be replaced as it's built";
        let sub_x = area.x + (area.width.saturating_sub(sub_msg.len() as u16)) / 2;
        buf.set_string(
            sub_x,
            center_y + 1,
            sub_msg,
            Style::default().fg(theme::MUTED),
        );
    }
}
