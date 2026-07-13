use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceTarget {
    Default,
    Named,
}

#[derive(Debug, Clone)]
pub struct InstanceScreen {
    pub target: InstanceTarget,
    pub name: String,
    pub focused_field: usize,
}

impl InstanceScreen {
    pub fn new() -> Self {
        Self {
            target: InstanceTarget::Default,
            name: String::new(),
            focused_field: 0,
        }
    }

    pub fn from_config() -> Self {
        // Phase 1 intentionally hydrates to the current/default Zeus home. The
        // named-instance target is a UI preview until the later sessions/fleet
        // slices introduce durable multi-instance config schema.
        Self::new()
    }

    pub fn field_count(&self) -> usize {
        if self.target == InstanceTarget::Named {
            2
        } else {
            1
        }
    }

    pub fn focus_next(&mut self) {
        let count = self.field_count().max(1);
        self.focused_field = (self.focused_field + 1) % count;
    }

    pub fn focus_prev(&mut self) {
        let count = self.field_count().max(1);
        self.focused_field = if self.focused_field == 0 {
            count - 1
        } else {
            self.focused_field - 1
        };
    }

    pub fn handle_char(&mut self, c: char) {
        if self.target == InstanceTarget::Named && self.focused_field == 1 {
            if is_instance_name_char(c) {
                self.name.push(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.target == InstanceTarget::Named && self.focused_field == 1 {
            self.name.pop();
        }
    }

    pub fn toggle_target(&mut self) {
        self.target = match self.target {
            InstanceTarget::Default => InstanceTarget::Named,
            InstanceTarget::Named => InstanceTarget::Default,
        };
        self.focused_field = 0;
    }

    pub fn move_up(&mut self) {
        self.target = InstanceTarget::Default;
        self.focused_field = 0;
    }

    pub fn move_down(&mut self) {
        self.target = InstanceTarget::Named;
        self.focused_field = 0;
    }

    pub fn selected_path(&self) -> String {
        match self.target {
            InstanceTarget::Default => "~/.zeus".to_string(),
            InstanceTarget::Named => self.named_path_preview(),
        }
    }

    fn named_path_preview(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            "~/.zeus/instances/<name>".to_string()
        } else {
            format!("~/.zeus/instances/{name}")
        }
    }

    pub fn render_with_cursor(
        &self,
        area: Rect,
        buf: &mut ratatui::buffer::Buffer,
        cursor_visible: bool,
    ) {
        self.render_inner(area, buf, cursor_visible);
    }

    fn render_inner(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_visible: bool) {
        Clear.render(area, buf);
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(" Instance target ");
        let inner = outer.inner(area);
        outer.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Min(3),
            ])
            .split(inner);

        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "Choose where this Zeus instance lives. ",
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    "Default is safe for existing boxes.",
                    Style::default().fg(theme::GREEN),
                ),
            ]),
            Line::from(Span::styled(
                "Named instances are previewed here; later slices add sessions/fleet persistence.",
                Style::default().fg(theme::DIM),
            )),
        ])
        .render(chunks[0], buf);

        self.render_option(
            chunks[1],
            buf,
            0,
            "Default instance",
            "Use current ~/.zeus/config.toml",
            "~/.zeus",
        );
        self.render_option(
            chunks[2],
            buf,
            1,
            "Named instance",
            "Preview isolated instance home",
            &self.named_path_preview(),
        );

        let name_line = if self.target == InstanceTarget::Named {
            let mut value = if self.name.is_empty() {
                "<type name>".to_string()
            } else {
                self.name.clone()
            };
            if self.focused_field == 1 && cursor_visible {
                value.push('█');
            }
            Line::from(vec![
                Span::styled("Name: ", Style::default().fg(theme::DIM)),
                Span::styled(
                    value,
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(Span::styled(
                "Press Space/Enter to switch target. Tab reaches footer controls.",
                Style::default().fg(theme::DIM),
            ))
        };

        Paragraph::new(vec![
            name_line,
            Line::from(vec![
                Span::styled("Launch preview: ", Style::default().fg(theme::DIM)),
                Span::styled(self.launch_preview(), Style::default().fg(theme::ACCENT)),
            ]),
        ])
        .render(chunks[3], buf);
    }

    fn render_option(
        &self,
        area: Rect,
        buf: &mut ratatui::buffer::Buffer,
        index: usize,
        title: &str,
        sub: &str,
        path: &str,
    ) {
        let selected = match (&self.target, index) {
            (InstanceTarget::Default, 0) => true,
            (InstanceTarget::Named, 1) => true,
            _ => false,
        };
        let focused = self.focused_field == 0 && selected;
        let border = if focused {
            theme::ACCENT
        } else if selected {
            theme::GREEN
        } else {
            theme::BORDER
        };
        let marker = if selected { "●" } else { "○" };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border));
        let inner = block.inner(area);
        block.render(area, buf);
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    Style::default().fg(if selected { theme::GREEN } else { theme::MUTED }),
                ),
                Span::styled(
                    title,
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" — {sub}"), Style::default().fg(theme::DIM)),
            ]),
            Line::from(Span::styled(
                path.to_string(),
                Style::default().fg(theme::ACCENT_DIM),
            )),
        ])
        .render(inner, buf);
    }

    fn launch_preview(&self) -> String {
        match self.target {
            InstanceTarget::Default => "zeus gateway".to_string(),
            InstanceTarget::Named => {
                let name = self.name.trim();
                if name.is_empty() {
                    "zeus gateway --instance <name>".to_string()
                } else {
                    format!("zeus gateway --instance {name}")
                }
            }
        }
    }
}

impl Default for InstanceScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for InstanceScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.render_inner(area, buf, false);
    }
}

fn is_instance_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
}
