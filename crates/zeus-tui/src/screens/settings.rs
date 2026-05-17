#![allow(dead_code)]
//! Settings screen — config viewer
//! Owner: zeus107 (feat/s68-tui-core)

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use crate::app::App;
use crate::theme;
use super::{Action, Screen};

pub struct SettingsScreen;

struct ConfigEntry {
    label: &'static str,
    key: &'static str,
}

const NETWORK_ENTRIES: &[ConfigEntry] = &[
    ConfigEntry { label: "Gateway URL",    key: "gateway_url" },
    ConfigEntry { label: "Connected",      key: "connected" },
    ConfigEntry { label: "Poll Interval",  key: "5s" },
    ConfigEntry { label: "Model",          key: "model" },
    ConfigEntry { label: "Provider",       key: "provider" },
    ConfigEntry { label: "Auth Method",    key: "auth_method" },
    ConfigEntry { label: "Version",        key: "gateway_version" },
];

const DISPLAY_ENTRIES: &[ConfigEntry] = &[
    ConfigEntry { label: "Theme",          key: "cyberpunk-dark-red" },
    ConfigEntry { label: "Border Style",   key: "rounded" },
    ConfigEntry { label: "Refresh Rate",   key: "100ms" },
];

const AGENT_ENTRIES: &[ConfigEntry] = &[
    ConfigEntry { label: "Agent Count",    key: "agents" },
    ConfigEntry { label: "Channel Count",  key: "channels" },
    ConfigEntry { label: "Tools",          key: "tools_count" },
    ConfigEntry { label: "Sessions",       key: "sessions_count" },
    ConfigEntry { label: "Log Buffer",     key: "1000 entries" },
];

fn render_section<'a>(
    title: &'static str,
    entries: &'static [ConfigEntry],
    app: &App,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!(" {} ", title),
        theme::title(),
    )));

    for entry in entries {
        let value = resolve_value(entry.key, app);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:20}", entry.label),
                theme::label(),
            ),
            Span::styled(value, theme::accent()),
        ]));
    }

    lines.push(Line::from(""));
    lines
}

fn resolve_value(key: &str, app: &App) -> String {
    match key {
        "gateway_url"     => app.gateway_url.clone(),
        "connected"       => if app.connected { "YES".into() } else { "NO".into() },
        "agents"          => app.agents.len().to_string(),
        "channels"        => app.channels.len().to_string(),
        "model"           => if app.model.is_empty() { "—".into() } else { app.model.clone() },
        "provider"        => if app.provider.is_empty() { "—".into() } else { app.provider.clone() },
        "auth_method"     => if app.auth_method.is_empty() { "—".into() } else { app.auth_method.clone() },
        "gateway_version" => if app.gateway_version.is_empty() { "—".into() } else { app.gateway_version.clone() },
        "tools_count"     => if app.tools_count == 0 { "—".into() } else { app.tools_count.to_string() },
        "sessions_count"  => app.sessions_count.to_string(),
        other             => other.to_string(),
    }
}

impl Screen for SettingsScreen {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App) {
        let outer = Block::default()
            .title(Span::styled("[ SETTINGS ]", theme::title()))
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(theme::border_active());

        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        // Two columns
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        // Left: Network + Agent
        let mut left: Vec<Line> = vec![Line::from("")];
        left.extend(render_section("NETWORK", NETWORK_ENTRIES, app));
        left.extend(render_section("AGENTS", AGENT_ENTRIES, app));

        let left_widget = Paragraph::new(left)
            .block(Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme::border()));
        frame.render_widget(left_widget, cols[0]);

        // Right: Display + footer
        let mut right: Vec<Line> = vec![Line::from("")];
        right.extend(render_section("DISPLAY", DISPLAY_ENTRIES, app));

        right.push(Line::from(""));
        right.push(Line::from(Span::styled(
            "  Config: ~/.zeus/config.toml",
            theme::muted(),
        )));
        right.push(Line::from(Span::styled(
            "  Press Esc to close",
            theme::muted(),
        )));

        let right_widget = Paragraph::new(right);
        frame.render_widget(right_widget, cols[1]);
    }

    fn handle_input(&self, key: crossterm::event::KeyEvent, _app: &mut App) -> Action {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Action::SwitchTab(0),
            _ => Action::Continue,
        }
    }
}
