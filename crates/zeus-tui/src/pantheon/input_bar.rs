//! Pantheon IRC-style input bar widget.
//!
//! Renders a single-line prompt at the bottom of the chat layout:
//!
//!   [#general] > hello world█
//!
//! Key bindings handled upstream (in the main event loop):
//!   Enter       — submit (send message or dispatch command)
//!   Backspace   — delete last char
//!   Esc         — clear input
//!   Any char    — append to buffer (via `PantheonApp.input`)
//!
//! This module owns only the *rendering* half. State lives in
//! `PantheonApp.input` (a plain `String`).

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};

use super::app::PantheonApp;

/// Ratatui widget that draws the IRC input bar.
///
/// Borrows app state immutably — no mutation happens inside `render`.
pub struct InputBar<'a> {
    /// The agent's current nick (for the prompt prefix).
    pub nick: &'a str,
    /// Current channel name (e.g. "#general").
    pub channel: &'a str,
    /// Current content of the input buffer.
    pub input: &'a str,
    /// Whether the input bar currently has keyboard focus.
    pub focused: bool,
}

impl<'a> InputBar<'a> {
    /// Construct from the relevant slices of `PantheonApp`.
    pub fn from_app(app: &'a PantheonApp) -> Self {
        let channel = app
            .active_channel()
            .map(|c| c.name.as_str())
            .unwrap_or("#general");
        Self {
            nick: &app.nick,
            channel,
            input: &app.input,
            focused: true, // input bar is always focused when visible
        }
    }
}

impl<'a> Widget for InputBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Border style: bright cyan when focused, dark gray when not.
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(
                format!(" {} ", self.channel),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));

        // Inner area for the text line.
        let inner = block.inner(area);
        block.render(area, buf);

        // Detect whether input is a command draft (starts with /).
        let is_cmd = self.input.starts_with('/');

        // Prompt: `> ` in cyan.
        let prompt = Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        // Input text colour: yellow for commands, white for normal text.
        let input_style = if is_cmd {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        // Block cursor rendered as a highlighted space at end of text.
        let cursor = Span::styled(
            " ",
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black),
        );

        let line = Line::from(vec![
            prompt,
            Span::styled(self.input, input_style),
            cursor,
        ]);

        Paragraph::new(line).render(inner, buf);
    }
}

/// Submit the current input buffer.
///
/// Returns the raw string that was in the buffer and clears `app.input`.
/// The caller is responsible for routing the result to `commands::parse`
/// (for `/` commands) or constructing an `IrcMessage` (for normal text).
pub fn submit(app: &mut PantheonApp) -> Option<String> {
    let text = app.input.trim().to_string();
    app.input.clear();
    if text.is_empty() { None } else { Some(text) }
}

/// Handle a printable character keystroke — append to the input buffer.
pub fn type_char(app: &mut PantheonApp, c: char) {
    app.input.push(c);
}

/// Handle Backspace — remove the last character from the input buffer.
pub fn backspace(app: &mut PantheonApp) {
    app.input.pop();
}

/// Handle Esc — clear the entire input buffer.
pub fn clear_input(app: &mut PantheonApp) {
    app.input.clear();
}
