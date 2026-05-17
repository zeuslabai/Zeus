#![allow(dead_code)]
//! Help screen — keybindings reference
//! Owner: mikes-Mac-mini (feat/s68-tui-core)

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use crate::app::App;
use crate::theme;
use super::{Action, Screen};

pub struct HelpScreen;

struct KeyBind {
    key: &'static str,
    desc: &'static str,
}

const GLOBAL_KEYS: &[KeyBind] = &[
    KeyBind { key: "1-4",     desc: "Switch tab" },
    KeyBind { key: "?",       desc: "Open help" },
    KeyBind { key: "q / Q",   desc: "Quit" },
    KeyBind { key: "Tab",     desc: "Next tab" },
];

const CHAT_KEYS: &[KeyBind] = &[
    KeyBind { key: "i",       desc: "Enter input mode" },
    KeyBind { key: "Enter",   desc: "Send message" },
    KeyBind { key: "Esc",     desc: "Exit input mode" },
    KeyBind { key: "j / ↓",   desc: "Scroll down" },
    KeyBind { key: "k / ↑",   desc: "Scroll up" },
    KeyBind { key: "g",       desc: "Jump to top" },
    KeyBind { key: "G",       desc: "Jump to bottom" },
];

const STATUS_KEYS: &[KeyBind] = &[
    KeyBind { key: "j / ↓",   desc: "Next agent" },
    KeyBind { key: "k / ↑",   desc: "Previous agent" },
    KeyBind { key: "Enter",   desc: "Select agent" },
    KeyBind { key: "r",       desc: "Refresh" },
];

const MEMORY_KEYS: &[KeyBind] = &[
    KeyBind { key: "j / ↓",   desc: "Scroll down" },
    KeyBind { key: "k / ↑",   desc: "Scroll up" },
    KeyBind { key: "f",       desc: "Filter by kind" },
    KeyBind { key: "g",       desc: "Jump to top" },
    KeyBind { key: "G",       desc: "Jump to bottom" },
    KeyBind { key: "Esc",     desc: "Clear filter" },
];

fn render_section(title: &str, keys: &[KeyBind]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Section header
    lines.push(Line::from(Span::styled(
        format!(" {} ", title),
        theme::title(),
    )));

    // Key rows
    for bind in keys {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:12}", bind.key),
                theme::accent(),
            ),
            Span::styled(
                bind.desc.to_string(),
                theme::text(),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines
}

impl Screen for HelpScreen {
    fn render(&self, frame: &mut Frame, area: Rect, _app: &App) {
        let outer = Block::default()
            .title(Span::styled("[ HELP — KEYBINDINGS ]", theme::title()))
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(theme::border_active());

        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        // Split into two columns
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        // Left column: Global + Chat
        let mut left_lines: Vec<Line<'static>> = Vec::new();
        left_lines.push(Line::from(""));
        left_lines.extend(render_section("GLOBAL", GLOBAL_KEYS));
        left_lines.extend(render_section("CHAT", CHAT_KEYS));

        let left = Paragraph::new(left_lines)
            .block(Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme::border()));
        frame.render_widget(left, cols[0]);

        // Right column: Status + Memory
        let mut right_lines: Vec<Line<'static>> = Vec::new();
        right_lines.push(Line::from(""));
        right_lines.extend(render_section("STATUS", STATUS_KEYS));
        right_lines.extend(render_section("MEMORY", MEMORY_KEYS));

        // Footer hint
        right_lines.push(Line::from(Span::styled(
            "  Press ? or Esc to close",
            theme::muted(),
        )));

        let right = Paragraph::new(right_lines);
        frame.render_widget(right, cols[1]);
    }

    fn handle_input(&self, key: crossterm::event::KeyEvent, _app: &mut App) -> Action {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => Action::SwitchTab(0),
            _ => Action::Continue,
        }
    }
}
