use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;
use super::top_bar::STEPS;

/// StepHeader widget — matches JSX StepHeader (line 270).
/// Renders: `STEP 01/19` + `WLCM` + title + subtitle
pub struct StepHeader {
    pub step_idx: usize,
    pub title: &'static str,
    pub subtitle: &'static str,
}

impl Widget for StepHeader {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 {
            return;
        }

        let code = STEPS.get(self.step_idx).map(|(_, c)| *c).unwrap_or("????");
        let step_num = format!("{:02}", self.step_idx + 1);

        // Line 1: STEP 01/19  CODE
        if area.height >= 1 {
            let line1 = Line::from(vec![
                Span::styled(
                    format!("STEP {}/{} ", step_num, STEPS.len()),
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    code,
                    Style::default()
                        .fg(theme::DIM)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            buf.set_line(area.x, area.y, &line1, area.width);
        }

        // Line 2: title
        if area.height >= 2 {
            let line2 = Line::from(Span::styled(
                self.title,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ));
            buf.set_line(area.x, area.y + 1, &line2, area.width);
        }

        // Line 3: subtitle
        if area.height >= 3 {
            let line3 = Line::from(Span::styled(self.subtitle, Style::default().fg(theme::DIM)));
            buf.set_line(area.x, area.y + 2, &line3, area.width);
        }
    }
}
