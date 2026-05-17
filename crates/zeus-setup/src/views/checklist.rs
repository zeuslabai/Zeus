//! Multi-select checklist view (used for deploy targets, build options)

use crate::theme::Theme;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

/// Checklist item state
#[derive(Debug, Clone)]
pub struct ChecklistItem {
    pub label: String,
    pub checked: bool,
}

/// Render a checklist
pub fn render_checklist(
    items: &[ChecklistItem],
    cursor: usize,
    theme: &Theme,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) {
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == cursor;
            let check = if item.checked { "x" } else { " " };

            if is_selected {
                Line::from(vec![
                    Span::styled(
                        format!("  ▸ [{}] ", check),
                        Style::default()
                            .fg(theme.highlight)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &item.label,
                        Style::default()
                            .fg(theme.highlight)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(format!("    [{}] ", check), Style::default().fg(theme.fg)),
                    Span::styled(&item.label, Style::default().fg(theme.fg)),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    paragraph.render(area, buf);
}
