//! login.rs — Pantheon login screen widget.
//!
//! Renders the Zeus ASCII logo, four credential fields, and an auth status
//! line. Keyboard handling is delegated to the caller (TUI event loop) which
//! mutates `LoginForm` and `AuthState` directly.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::pantheon::app::{AuthState, LoginField, LoginForm};

/// Zeus ASCII logo lines — displayed at the top of the login screen.
const ZEUS_LOGO: &[&str] = &[
    r"  ███████╗███████╗██╗   ██╗███████╗",
    r"  ╚══███╔╝██╔════╝██║   ██║██╔════╝",
    r"    ███╔╝ █████╗  ██║   ██║███████╗",
    r"   ███╔╝  ██╔══╝  ██║   ██║╚════██║",
    r"  ███████╗███████╗╚██████╔╝███████║",
    r"  ╚══════╝╚══════╝ ╚═════╝ ╚══════╝",
    r"",
    r"      ⚡  Pantheon Agent Network  ⚡",
];

/// The login screen widget. Borrow `LoginForm` + `AuthState` from app state
/// and pass them here each frame.
pub struct LoginScreen<'a> {
    pub form:       &'a LoginForm,
    pub auth_state: &'a AuthState,
}

impl<'a> LoginScreen<'a> {
    pub fn new(form: &'a LoginForm, auth_state: &'a AuthState) -> Self {
        Self { form, auth_state }
    }
}

impl<'a> Widget for LoginScreen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first so remnants from other screens don't bleed through.
        Clear.render(area, buf);

        // Outer block with border
        let outer = Block::default()
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Pantheon ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = outer.inner(area);
        outer.render(area, buf);

        // Vertical layout: logo | spacer | fields | spacer | status
        let logo_height = ZEUS_LOGO.len() as u16;
        let fields_height = 4 * 3; // 4 fields × (label + input + gap)
        let status_height = 1u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(logo_height),
                Constraint::Length(1),              // spacer
                Constraint::Length(fields_height),
                Constraint::Length(1),              // spacer
                Constraint::Length(status_height),
                Constraint::Min(0),                 // leftover
            ])
            .split(inner);

        render_logo(chunks[0], buf);
        render_fields(self.form, chunks[2], buf);
        render_status(self.auth_state, chunks[4], buf);
    }
}

// ── Logo ─────────────────────────────────────────────────────────────────────

fn render_logo(area: Rect, buf: &mut Buffer) {
    let lines: Vec<Line> = ZEUS_LOGO
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                *l,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .render(area, buf);
}

// ── Credential fields ─────────────────────────────────────────────────────────

struct FieldDef {
    label:    &'static str,
    field:    LoginField,
    secret:   bool,
    placeholder: &'static str,
}

const FIELDS: &[FieldDef] = &[
    FieldDef { label: "Nick",        field: LoginField::Nick,       secret: false, placeholder: "your IRC nick" },
    FieldDef { label: "Channel Key", field: LoginField::ChannelKey, secret: true,  placeholder: "from config.toml" },
    FieldDef { label: "Gateway URL", field: LoginField::GatewayUrl, secret: false, placeholder: "ws://localhost:8080 (optional)" },
    FieldDef { label: "Agent Name",  field: LoginField::AgentName,  secret: false, placeholder: "human-readable agent label (optional)" },
];

fn render_fields(form: &LoginForm, area: Rect, buf: &mut Buffer) {
    // Centre a 50-char-wide column inside `area`.
    let col_width = area.width.min(60);
    let col_x = area.x + (area.width.saturating_sub(col_width)) / 2;

    let row_height = 3u16; // label + input box + 1 gap
    for (i, def) in FIELDS.iter().enumerate() {
        let row_y = area.y + (i as u16) * row_height;
        if row_y + 2 > area.y + area.height {
            break;
        }

        let is_focused = form.focused == def.field;

        // Label line
        let label_style = if is_focused {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let label_area = Rect {
            x: col_x,
            y: row_y,
            width: col_width,
            height: 1,
        };
        Paragraph::new(def.label)
            .style(label_style)
            .render(label_area, buf);

        // Input box
        let raw_value = match def.field {
            LoginField::Nick       => &form.nick,
            LoginField::ChannelKey => &form.channel_key,
            LoginField::GatewayUrl => &form.gateway_url,
            LoginField::AgentName  => &form.agent_name,
        };
        let display_value: String = if def.secret && !raw_value.is_empty() {
            "•".repeat(raw_value.len())
        } else {
            raw_value.clone()
        };
        let display_text = if display_value.is_empty() {
            Span::styled(
                def.placeholder,
                Style::default().fg(Color::DarkGray),
            )
        } else {
            Span::styled(
                display_value,
                Style::default().fg(Color::White),
            )
        };

        let border_style = if is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(border_style);

        let input_area = Rect {
            x: col_x,
            y: row_y + 1,
            width: col_width,
            height: 2,
        };
        let inner_area = block.inner(input_area);
        block.render(input_area, buf);
        Paragraph::new(Line::from(display_text)).render(inner_area, buf);
    }
}

// ── Auth status line ──────────────────────────────────────────────────────────

fn render_status(auth_state: &AuthState, area: Rect, buf: &mut Buffer) {
    let (text, style) = match auth_state {
        AuthState::Idle => (
            "[ Tab ] next field   [ Enter ] connect   [ Esc ] cancel".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        AuthState::Connecting => (
            "⟳ Connecting to Pantheon gateway…".to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        AuthState::Authenticated => (
            "✓ Authenticated — entering Pantheon…".to_string(),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        AuthState::Failed(msg) => (
            format!("✗ Auth failed: {}", msg),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    };

    Paragraph::new(text)
        .style(style)
        .alignment(Alignment::Center)
        .render(area, buf);
}
