use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::prod::draw::BufferClampExt;
use crate::theme;

/// Settings subsystem groups from `docs/zeus-tui-production.jsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Llm,
    Channels,
    Memory,
    Security,
    Tools,
    Display,
    System,
}

impl SettingsSection {
    pub const ALL: &'static [SettingsSection] = &[
        Self::Llm,
        Self::Channels,
        Self::Memory,
        Self::Security,
        Self::Tools,
        Self::Display,
        Self::System,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Channels => "channels",
            Self::Memory => "memory",
            Self::Security => "security",
            Self::Tools => "tools",
            Self::Display => "display",
            Self::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Llm => "LLM",
            Self::Channels => "Channels",
            Self::Memory => "Memory",
            Self::Security => "Security",
            Self::Tools => "Tools",
            Self::Display => "Display",
            Self::System => "System",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Llm => "◇",
            Self::Channels => "⇌",
            Self::Memory => "▤",
            Self::Security => "🛡",
            Self::Tools => "⚙",
            Self::Display => "▦",
            Self::System => "⊕",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Llm => theme::ACCENT,
            Self::Channels => theme::GREEN,
            Self::Memory => theme::CYAN,
            Self::Security => theme::RED,
            Self::Tools => theme::AMBER,
            Self::Display => theme::PURPLE,
            Self::System => theme::DIM,
        }
    }

    pub fn rows(self) -> &'static [SettingRow] {
        match self {
            Self::Llm => LLM_ROWS,
            Self::Channels => CHANNEL_ROWS,
            Self::Memory => MEMORY_ROWS,
            Self::Security => SECURITY_ROWS,
            Self::Tools => TOOL_ROWS,
            Self::Display => DISPLAY_ROWS,
            Self::System => SYSTEM_ROWS,
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SettingRow {
    pub key: &'static str,
    pub value: &'static str,
    pub help: &'static str,
    pub dirty: bool,
    pub locked: bool,
    pub action: bool,
}

impl SettingRow {
    const fn edit(key: &'static str, value: &'static str, help: &'static str) -> Self {
        Self {
            key,
            value,
            help,
            dirty: false,
            locked: false,
            action: false,
        }
    }

    const fn dirty(key: &'static str, value: &'static str, help: &'static str) -> Self {
        Self {
            key,
            value,
            help,
            dirty: true,
            locked: false,
            action: false,
        }
    }

    const fn locked(key: &'static str, value: &'static str, help: &'static str) -> Self {
        Self {
            key,
            value,
            help,
            dirty: false,
            locked: true,
            action: false,
        }
    }

    const fn action(key: &'static str, value: &'static str, help: &'static str) -> Self {
        Self {
            key,
            value,
            help,
            dirty: false,
            locked: false,
            action: true,
        }
    }
}

/// Prototype-faithful Settings tab. `config` is the sanitized live
/// `GET /v1/config` payload; when present it overlays matching field values.
pub struct SettingsTab<'a> {
    pub active: SettingsSection,
    pub config: Option<&'a serde_json::Value>,
}

impl Default for SettingsTab<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SettingsTab<'a> {
    pub fn new() -> Self {
        Self {
            active: SettingsSection::Llm,
            config: None,
        }
    }

    pub fn with_config(config: Option<&'a serde_json::Value>) -> Self {
        Self {
            active: SettingsSection::Llm,
            config,
        }
    }

    pub fn with_active(mut self, active: SettingsSection) -> Self {
        self.active = active;
        self
    }

    fn live_value(
        config: Option<&serde_json::Value>,
        section: SettingsSection,
        row: SettingRow,
    ) -> String {
        let Some(config) = config else {
            return row.value.to_string();
        };

        let section_obj = config
            .get(section.id())
            .or_else(|| config.get(section.label()))
            .or_else(|| config.get(section.label().to_ascii_lowercase()));
        let Some(obj) = section_obj else {
            return row.value.to_string();
        };

        let snake_key = field_key(row.key);
        let value = obj
            .get(row.key)
            .or_else(|| obj.get(snake_key.as_str()))
            .or_else(|| obj.get(row.key.to_ascii_lowercase()));

        match value {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Bool(true)) => "✓ true".to_string(),
            Some(serde_json::Value::Bool(false)) => "○ false".to_string(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::Array(items)) => format!("{} entries", items.len()),
            Some(serde_json::Value::Object(items)) => format!("{} entries", items.len()),
            _ => row.value.to_string(),
        }
    }
}

impl Widget for SettingsTab<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        if area.height < 6 || area.width < 50 {
            return;
        }

        fill_bg(area, buf, theme::BG);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(area);

        render_group_list(cols[0], buf, self.active);
        render_fields(cols[1], buf, self.active, self.config);
    }
}

fn render_group_list(area: Rect, buf: &mut Buffer, active: SettingsSection) {
    fill_bg(area, buf, theme::BG_PANEL);
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme::BORDER));
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 2 {
        return;
    }

    buf.set_string_clamped(
        inner.x + 1,
        inner.y,
        "SUBSYSTEM",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );

    let mut y = inner.y + 2;
    for group in SettingsSection::ALL {
        if y >= inner.bottom() {
            break;
        }
        let selected = *group == active;
        let style = if selected {
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .bg(theme::BG_HIGHLIGHT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT).bg(theme::BG_PANEL)
        };
        paint_row(Rect::new(inner.x, y, inner.width, 1), buf, style);
        if selected {
            buf.set_string_clamped(
                inner.x,
                y,
                "┃",
                Style::default().fg(group.color()).bg(theme::BG_HIGHLIGHT),
            );
        }
        buf.set_string_clamped(
            inner.x + 2,
            y,
            group.glyph(),
            Style::default()
                .fg(if selected {
                    group.color()
                } else {
                    theme::MUTED
                })
                .bg(if selected {
                    theme::BG_HIGHLIGHT
                } else {
                    theme::BG_PANEL
                })
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(inner.x + 5, y, group.label(), style);
        if group.rows().iter().any(|row| row.dirty) {
            buf.set_string_clamped(
                inner.right().saturating_sub(5),
                y,
                "●",
                Style::default().fg(theme::AMBER).bg(if selected {
                    theme::BG_HIGHLIGHT
                } else {
                    theme::BG_PANEL
                }),
            );
        }
        let count = group.rows().len().to_string();
        let count_x = inner.right().saturating_sub(count.len() as u16 + 1);
        buf.set_string_clamped(
            count_x,
            y,
            count,
            Style::default().fg(theme::MUTED).bg(if selected {
                theme::BG_HIGHLIGHT
            } else {
                theme::BG_PANEL
            }),
        );
        y += 2;
    }
}

fn render_fields(
    area: Rect,
    buf: &mut Buffer,
    active: SettingsSection,
    config: Option<&serde_json::Value>,
) {
    fill_bg(area, buf, theme::BG);
    if area.height < 4 || area.width < 20 {
        return;
    }

    let header_h = 3.min(area.height);
    let header = Rect::new(area.x, area.y, area.width, header_h);
    fill_bg(header, buf, theme::BG_PANEL);
    if header.height > 0 {
        buf.set_string_clamped(
            header.x + 2,
            header.y + 1.min(header.height.saturating_sub(1)),
            active.glyph(),
            Style::default().fg(active.color()).bg(theme::BG_PANEL),
        );
        buf.set_string_clamped(
            header.x + 5,
            header.y + 1.min(header.height.saturating_sub(1)),
            active.label(),
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .bg(theme::BG_PANEL)
                .add_modifier(Modifier::BOLD),
        );
        let count = format!("{} settings", active.rows().len());
        buf.set_string_clamped(
            header.x + 18,
            header.y + 1.min(header.height.saturating_sub(1)),
            count,
            Style::default().fg(theme::DIM).bg(theme::BG_PANEL),
        );
        let hint = "changes save on Enter · Esc to discard";
        let hint_x = header.right().saturating_sub(hint.len() as u16 + 2);
        if hint_x > header.x + 28 {
            buf.set_string_clamped(
                hint_x,
                header.y + 1.min(header.height.saturating_sub(1)),
                hint,
                Style::default().fg(theme::MUTED).bg(theme::BG_PANEL),
            );
        }
    }

    let divider_y = area.y + header_h.saturating_sub(1);
    for x in area.x..area.right() {
        buf[(x, divider_y)]
            .set_char('─')
            .set_style(Style::default().fg(theme::BORDER).bg(theme::BG_PANEL));
    }

    let mut y = area.y + header_h + 1;
    for row in active.rows() {
        if y >= area.bottom() {
            break;
        }
        render_field_row(area, buf, y, active, *row, config);
        y = y.saturating_add(3);
    }
}

fn render_field_row(
    area: Rect,
    buf: &mut Buffer,
    y: u16,
    active: SettingsSection,
    row: SettingRow,
    config: Option<&serde_json::Value>,
) {
    let bg = if row.dirty {
        theme::BG_HIGHLIGHT
    } else {
        theme::BG
    };
    let row_area = Rect::new(
        area.x,
        y,
        area.width,
        2.min(area.bottom().saturating_sub(y)),
    );
    fill_bg(row_area, buf, bg);

    let key_style = Style::default()
        .fg(theme::TEXT)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let value_style = Style::default()
        .fg(if row.action {
            active.color()
        } else if row.locked {
            theme::DIM
        } else {
            theme::TEXT_BRIGHT
        })
        .bg(bg);
    let help_style = Style::default()
        .fg(theme::DIM)
        .bg(bg)
        .add_modifier(Modifier::ITALIC);

    let key = if row.dirty {
        format!("● {}", row.key)
    } else if row.locked {
        format!("🔒 {}", row.key)
    } else {
        row.key.to_string()
    };
    buf.set_string_clamped(area.x + 2, y, key, key_style);
    let value = SettingsTab::live_value(config, active, row);
    let value_x = area.x + 27.min(area.width.saturating_sub(1));
    buf.set_string_clamped(value_x, y, value, value_style);
    if y + 1 < area.bottom() {
        buf.set_string_clamped(value_x, y + 1, row.help, help_style);
    }

    let button = if row.action {
        " RUN "
    } else if row.locked {
        " LOCKED "
    } else {
        " EDIT "
    };
    let button_x = area.right().saturating_sub(button.len() as u16 + 2);
    if button_x > value_x + 12 {
        let button_style = if row.action {
            Style::default()
                .fg(theme::BG)
                .bg(active.color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED).bg(bg)
        };
        buf.set_string_clamped(button_x, y, button, button_style);
    }

    let sep_y = y.saturating_add(2);
    if sep_y < area.bottom() {
        for x in area.x..area.right() {
            buf[(x, sep_y)]
                .set_char('─')
                .set_style(Style::default().fg(theme::BORDER).bg(theme::BG));
        }
    }
}

fn paint_row(area: Rect, buf: &mut Buffer, style: Style) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_style(style);
        }
    }
}

fn fill_bg(area: Rect, buf: &mut Buffer, color: Color) {
    for y in area.top()..area.bottom().min(buf.area.bottom()) {
        for x in area.left()..area.right().min(buf.area.right()) {
            buf[(x, y)]
                .set_char(' ')
                .set_style(Style::default().bg(color));
        }
    }
}

fn field_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

const LLM_ROWS: &[SettingRow] = &[
    SettingRow::edit("Provider", "anthropic", "Primary LLM provider"),
    SettingRow::edit(
        "Model",
        "claude-opus-4-7",
        "Specific model from provider catalog",
    ),
    SettingRow::edit("Temperature", "0.7", "Sampling temperature 0.0–2.0"),
    SettingRow::edit("Max iterations", "200", "Cooking loop iteration cap"),
    SettingRow::edit(
        "Fallback chain",
        "openai/gpt-4o, groq/llama-3.3-70b",
        "Comma-separated fallback providers",
    ),
];

const CHANNEL_ROWS: &[SettingRow] = &[
    SettingRow::edit("Discord", "✓ enabled", "Discord bot adapter"),
    SettingRow::edit("Telegram", "✓ enabled", "Telegram MTProto adapter"),
    SettingRow::edit("Slack", "✓ enabled", "Slack Socket Mode adapter"),
    SettingRow::edit("Email", "✓ enabled", "SMTP + IMAP IDLE adapter"),
    SettingRow::edit("iMessage", "✓ enabled (macOS)", "AppleScript bridge"),
    SettingRow::edit("WhatsApp", "○ disabled", "Cloud API adapter"),
    SettingRow::edit("Signal", "○ disabled", "signal-cli adapter"),
    SettingRow::edit("Matrix", "○ disabled", "matrix-sdk adapter"),
];

const MEMORY_ROWS: &[SettingRow] = &[
    SettingRow::edit(
        "DB path",
        "~/.zeus/mnemosyne.db",
        "SQLite + vector store location",
    ),
    SettingRow::dirty("Embedding provider", "ollama", "Embedding model provider"),
    SettingRow::edit(
        "Embedding model",
        "nomic-embed-text",
        "Specific embedding model",
    ),
    SettingRow::edit("FTS enabled", "✓ true", "SQLite FTS5 full-text index"),
    SettingRow::edit("Auto-prune", "30 days", "Old session cleanup threshold"),
];

const SECURITY_ROWS: &[SettingRow] = &[
    SettingRow::edit("Aegis level", "standard", "Sandbox aggressiveness"),
    SettingRow::edit("Approval mode", "interactive", "How approvals are surfaced"),
    SettingRow::edit("Command allowlist", "47 entries", "Approved shell commands"),
    SettingRow::edit("URL allowlist", "12 entries", "Approved web_fetch URLs"),
    SettingRow::edit("Audit log", "~/.zeus/audit.jsonl", "Audit trail location"),
];

const TOOL_ROWS: &[SettingRow] = &[
    SettingRow::locked(
        "Talos enabled",
        "✓ FORCE-ON (macOS)",
        "macOS automation crate",
    ),
    SettingRow::edit("Browser", "✓ enabled", "Chrome CDP automation"),
    SettingRow::edit("MCP servers", "3 connected", "Active MCP server count"),
    SettingRow::edit("Tool timeout", "30s", "Per-tool execution timeout"),
];

const DISPLAY_ROWS: &[SettingRow] = &[
    SettingRow::edit("Theme", "dark", "Color theme"),
    SettingRow::edit("Accent color", "fire-orange", "UI accent color"),
    SettingRow::edit("Vim mode", "✓ true", "Vim-style keybinds"),
    SettingRow::edit("High contrast", "○ false", "Accessibility mode"),
    SettingRow::edit("Animations", "✓ true", "Enable streaming animations"),
];

const SYSTEM_ROWS: &[SettingRow] = &[
    SettingRow::action("Re-run onboarding", "→", "Launch zeus onboard --resume"),
    SettingRow::action("Daemon status", "→", "View / restart gateway daemon"),
    SettingRow::action("Export config", "→", "Save config.toml to file"),
    SettingRow::edit("Build version", "0.4.7-rc.3 (a1c4f29)", "Current build"),
    SettingRow::edit(
        "Workspace path",
        "~/.zeus/workspace",
        "Agent workspace location",
    ),
];
