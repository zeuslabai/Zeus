//! UI — Chat-first layout with slim sidebar
//!
//! Left: full chat + input (like Claude Code)
//! Right: slim agents + channels sidebar (collapsible info)

use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::app::{App, Tab};
use crate::theme;
use crate::markdown;
use crate::office;

pub fn render(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    let layout = Layout::vertical([
        Constraint::Length(1),  // status bar
        Constraint::Length(1),  // tab bar
        Constraint::Min(0),    // body
    ]).split(size);

    render_status_bar(frame, layout[0], app);
    render_tab_bar(frame, layout[1], app);

    match app.active_tab {
        Tab::Chat => {
            // Chat takes most space, slim sidebar on right
            let body = Layout::horizontal([
                Constraint::Min(0),        // chat (fills remaining)
                Constraint::Length(24),    // sidebar (slim)
            ]).split(layout[2]);

            render_chat_area(frame, body[0], app);
            render_sidebar(frame, body[1], app);
        }
        Tab::Office => {
            office::render(frame, layout[2], &mut app.office_bg, &mut app.office);
        }
        Tab::Pantheon => {
            render_pantheon(frame, layout[2], app);
        }
        Tab::Settings => {
            render_settings(frame, layout[2], app);
        }
    }

    // Keybinding overlay — floats on top of whatever tab is active
    if app.show_keybind_overlay {
        render_keybind_overlay(frame, size);
    }
}

/// Center a rect of `width × height` within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn render_keybind_overlay(frame: &mut Frame, area: Rect) {
    // Dim the background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    let overlay = centered_rect(62, 30, area);
    frame.render_widget(bg.clone(), overlay);

    let block = Block::default()
        .title(Span::styled("  [ ? ] KEYBINDINGS  ", theme::title()))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(theme::border_active());

    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    // Two-column layout inside the modal
    let cols = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).split(inner);

    // ---- Left column: Global + Chat ----
    let left_lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(" GLOBAL", theme::title())),
        keybind_line("Tab / 1-3", "Cycle tabs"),
        keybind_line("?",         "Toggle this overlay"),
        keybind_line("F10 / C-c", "Quit"),
        Line::from(""),
        Line::from(Span::styled(" CHAT", theme::title())),
        keybind_line("Enter",     "Send message"),
        keybind_line("Esc",       "Close overlay"),
        keybind_line("↑ / ↓",    "Input history"),
        keybind_line("PgUp/PgDn","Scroll messages"),
        keybind_line("← / →",    "Move cursor"),
        keybind_line("Home/End",  "Cursor start/end"),
        keybind_line("Backspace", "Delete char"),
        Line::from(""),
        Line::from(Span::styled(" SLASH COMMANDS", theme::title())),
        keybind_line("/clear",    "Clear session"),
        keybind_line("/compact",  "Compact context"),
    ];

    // ---- Right column: Pantheon + Office ----
    let right_lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(" PANTHEON", theme::title())),
        keybind_line("← / →",    "Switch panel"),
        keybind_line("↑ / ↓",    "Navigate list"),
        keybind_line("Enter",     "Drill into room/mission"),
        keybind_line("Esc",       "Back to rooms"),
        keybind_line("n",         "New war room"),
        keybind_line("a / p / c", "Approve/Pause/Cancel mission"),
        Line::from(""),
        Line::from(Span::styled(" OFFICE", theme::title())),
        keybind_line("f / F",     "Cycle agent focus"),
        keybind_line("m",         "Toggle memo overlay"),
        keybind_line("?",         "Office help"),
        keybind_line("Enter",     "Open agent in Pantheon"),
        keybind_line("Esc",       "Clear focus"),
        Line::from(""),
        Line::from(Span::styled(
            "  Press ? or Esc to close",
            theme::muted(),
        )),
    ];

    let left_para = Paragraph::new(left_lines)
        .block(Block::default().borders(Borders::RIGHT).border_style(theme::border()));
    let right_para = Paragraph::new(right_lines);

    frame.render_widget(left_para, cols[0]);
    frame.render_widget(right_para, cols[1]);
}

#[inline]
fn keybind_line(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:14}", key), theme::accent()),
        Span::styled(desc, theme::text()),
    ])
}

fn render_tab_bar(frame: &mut Frame, area: Rect, app: &App) {
    let tab_style = |active: bool| {
        if active {
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(theme::DIM)
        }
    };
    let tabs = Line::from(vec![
        Span::raw(" "),
        Span::styled(" chat ", tab_style(app.active_tab == Tab::Chat)),
        Span::raw(" "),
        Span::styled(" \u{26A1} the office ", tab_style(app.active_tab == Tab::Office)),
        Span::raw(" "),
        Span::styled(" pantheon ", tab_style(app.active_tab == Tab::Pantheon)),
        Span::raw(" "),
        Span::styled(" settings ", tab_style(app.active_tab == Tab::Settings)),
        Span::styled("  Tab to switch ", theme::muted()),
    ]);
    frame.render_widget(
        Paragraph::new(tabs).style(Style::default().bg(theme::BG_PANEL)),
        area,
    );
}

fn render_pantheon(frame: &mut Frame, area: Rect, app: &App) {
    crate::pantheon::chat::render_chat(frame, area, &app.pantheon_irc);
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let (status_icon, status_text, status_style) = if app.connected {
        ("●", " connected".to_string(), theme::success())
    } else if app.reconnect_attempts > 0 {
        ("○", format!(" gateway offline — reconnecting ({})", app.reconnect_attempts), Style::default().fg(theme::RED))
    } else {
        ("○", " gateway offline".to_string(), Style::default().fg(theme::RED))
    };

    let model_info = if app.connected && !app.model.is_empty() {
        format!(" {} ", app.model)
    } else {
        String::new()
    };

    let token_info = if app.session_input_tokens > 0 || app.session_output_tokens > 0 {
        format!(" │ {} tokens | {} ", app.format_tokens(), app.format_cost())
    } else {
        String::new()
    };

    // Context window pressure bar
    let pressure = app.context_pressure();
    let (bar_color, bar_label) = if pressure >= 0.8 {
        (theme::RED, "ctx")
    } else if pressure >= 0.5 {
        (theme::YELLOW, "ctx")
    } else {
        (theme::GREEN, "ctx")
    };
    let filled = (pressure * 8.0).round() as usize;
    let empty = 8usize.saturating_sub(filled);
    let ctx_bar = format!(" {} [{}{}] {:.0}% ", bar_label, "█".repeat(filled), "░".repeat(empty), pressure * 100.0);

    let budget_warn = app.budget_warning().map(|w| format!(" │ {} ", w)).unwrap_or_default();
    let budget_style = if app.budget_warning().is_some() {
        Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)
    } else {
        theme::muted()
    };

    // Cooking/thinking indicator — shows when agent is working.
    // T20: append " (N queued)" when there are pending inputs waiting.
    let queue_suffix = if !app.pending_inputs.is_empty() {
        format!(" ({} queued)", app.pending_inputs.len())
    } else {
        String::new()
    };
    let cooking_indicator = if let Some(ref thinking) = app.thinking_text {
        format!(" │ ⚡ {}{} ", thinking, queue_suffix)
    } else if app.messages.last().map(|m| m.streaming).unwrap_or(false) {
        format!(" │ ⚡ Thinking...{} ", queue_suffix)
    } else {
        String::new()
    };
    let cooking_style = Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD);

    let bar = Line::from(vec![
        Span::styled(format!(" {} ", app.self_name), Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
        Span::styled("│ ", theme::muted()),
        Span::styled(status_icon, status_style),
        Span::styled(status_text, status_style),
        Span::styled(&model_info, theme::label()),
        Span::styled(&cooking_indicator, cooking_style),
        Span::styled(&token_info, theme::muted()),
        Span::styled(&budget_warn, budget_style),
        Span::styled("│", theme::muted()),
        Span::styled(ctx_bar, Style::default().fg(bar_color)),
        Span::styled("│ Ctrl+C quit ", theme::label()),
    ]);
    frame.render_widget(
        Paragraph::new(bar).style(Style::default().bg(theme::BG_PANEL)),
        area,
    );
}

fn render_chat_area(frame: &mut Frame, area: Rect, app: &App) {
    let layout = Layout::vertical([
        Constraint::Min(0),    // messages
        Constraint::Length(3), // input
    ]).split(area);

    render_messages(frame, layout[0], app);
    render_input(frame, layout[1], app);
}

fn render_messages(frame: &mut Frame, area: Rect, app: &App) {
    // Reserve bottom row for scroll indicator (when scrolled up)
    let show_indicator = !app.is_at_bottom();
    let msg_area = if show_indicator {
        Rect { height: area.height.saturating_sub(1), ..area }
    } else {
        area
    };
    let indicator_area = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };

    let width = msg_area.width as usize;
    let mut all_lines: Vec<Line> = Vec::new();

    if app.messages.is_empty() {
        all_lines.push(Line::raw(""));
        all_lines.push(Line::styled(
            "  Welcome to Zeus.",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ));
        all_lines.push(Line::raw(""));
        all_lines.push(Line::styled("  Type a message and press Enter.", theme::text()));
        if !app.connected {
            all_lines.push(Line::raw(""));
            all_lines.push(Line::styled("  Gateway offline — run: zeus gateway", theme::muted()));
        }
    } else {
        for msg in &app.messages {
            let (role_label, role_style) = match msg.role {
                crate::app::Role::User => (
                    "you",
                    Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD),
                ),
                crate::app::Role::Assistant => (
                    msg.agent_name.as_deref().unwrap_or(&app.self_name),
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                ),
                crate::app::Role::System => ("system", Style::default().fg(theme::YELLOW)),
                crate::app::Role::Tool => ("tool", Style::default().fg(theme::PURPLE)),
            };

            // Build header line: optional [channel] badge + role label + timestamp
            let mut header_spans: Vec<Span> = Vec::new();
            if let Some(source) = &msg.channel_source {
                let (badge_text, badge_style) = channel_badge(source);
                header_spans.push(Span::styled(badge_text, badge_style));
                header_spans.push(Span::raw(" "));
            }
            header_spans.push(Span::styled(format!(" {} ", role_label), role_style));
            header_spans.push(Span::styled(format!(" {}", msg.timestamp), theme::muted()));
            all_lines.push(Line::from(header_spans));

            // Render message content with word wrapping at terminal width.
            // For streaming messages, append a blinking block cursor so the
            // user can see tokens arriving token-by-token in real time.
            // Layer 3: show iter/tool badge and thinking snippet on the
            // active streaming placeholder when content is empty.
            if msg.streaming {
                if msg.content.is_empty() && (app.cooking_iter > 0 || app.cooking_tools > 0) {
                    // Show live cooking badge: "iter 3 • tool 7"
                    let badge = format!(
                        "iter {} • tool {}",
                        app.cooking_iter,
                        app.cooking_tools,
                    );
                    let placeholder = if let Some(ref thought) = app.thinking_text {
                        format!("🧠 {}  ┆  {}", badge, thought)
                    } else {
                        format!("⚙ {}…", badge)
                    };
                    let rendered = markdown::render_markdown_with_width(&placeholder, " ", width);
                    all_lines.extend(rendered);
                } else if let Some(ref state) = msg.stream_state {
                    // Split render: flushed blocks → styled markdown, pending tail → raw text + cursor
                    let flushed = state.flushed();
                    if !flushed.is_empty() {
                        let rendered = markdown::render_markdown_with_width(flushed, " ", width);
                        all_lines.extend(rendered);
                    }
                    let cursor = if app.tick_count % 8 < 4 { "▋" } else { "" };
                    let tail = format!("{}{}", state.pending(), cursor);
                    if !tail.is_empty() {
                        all_lines.push(ratatui::text::Line::raw(format!(" {}", tail)));
                    }
                } else {
                    // Fallback: no stream_state, render full content with cursor
                    let cursor = if app.tick_count % 8 < 4 { "▋" } else { "" };
                    let content_with_cursor = format!("{}{}", msg.content, cursor);
                    let rendered = markdown::render_markdown_with_width(&content_with_cursor, " ", width);
                    all_lines.extend(rendered);
                }
            } else {
                let rendered = markdown::render_markdown_with_width(&msg.content, " ", width);
                all_lines.extend(rendered);
            };
            all_lines.push(Line::raw(""));
        }
    }

    // Scroll: offset from bottom, clamped so we never go past the top
    let visible = msg_area.height as usize;
    let total = all_lines.len();
    let max_offset = if total > visible { total - visible } else { 0 };
    let offset = app.scroll_offset.min(max_offset);
    let start = if total > visible { max_offset - offset } else { 0 };

    let visible_lines: Vec<ListItem> = all_lines[start..]
        .iter()
        .take(visible)
        .map(|l| ListItem::new(l.clone()))
        .collect();

    let list = List::new(visible_lines).style(Style::default().bg(theme::BG));
    frame.render_widget(list, msg_area);

    // Scroll position indicator
    if show_indicator {
        let pct = if max_offset == 0 { 100 } else {
            ((max_offset - offset) * 100 / max_offset).min(100)
        };
        let indicator = Line::from(vec![
            Span::styled(
                format!(" ↑ scrolled up — {}% — scroll or Esc to jump to bottom ", pct),
                theme::label(),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().bg(theme::BG_PANEL)),
            indicator_area,
        );
    }
}

/// Returns (badge_text, badge_style) for a channel source string.
fn channel_badge(source: &str) -> (String, Style) {
    match source {
        "discord"  => (
            "[discord]".to_string(),
            Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
        ),
        "telegram" => (
            "[telegram]".to_string(),
            Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD),
        ),
        "tui"      => (
            "[tui]".to_string(),
            Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD),
        ),
        "slack"    => (
            "[slack]".to_string(),
            Style::default().fg(Color::Rgb(74, 172, 188)).add_modifier(Modifier::BOLD),
        ),
        "api"      => (
            "[api]".to_string(),
            Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD),
        ),
        other      => (
            format!("[{}]", other),
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        ),
    }
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    // Horizontal scroll: show a window of text around the cursor
    let visible_width = area.width.saturating_sub(4) as usize; // borders + " > " title
    let chars: Vec<char> = app.input.chars().collect();
    let cursor = app.cursor_pos.min(chars.len());

    // Calculate scroll offset to keep cursor visible
    let scroll_offset = if cursor > visible_width.saturating_sub(2) {
        cursor - visible_width.saturating_sub(2)
    } else {
        0
    };

    let end = (scroll_offset + visible_width).min(chars.len());
    let visible_chars: String = chars[scroll_offset..end].iter().collect();
    let cursor_in_view = cursor - scroll_offset;

    let before: String = visible_chars.chars().take(cursor_in_view).collect();
    let after: String = visible_chars.chars().skip(cursor_in_view).collect();

    let input_line = Line::from(vec![
        Span::raw(before),
        Span::styled("█", ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED)),
        Span::raw(after),
    ]);
    let input_widget = Paragraph::new(input_line)
        .style(theme::bright())
        .block(
            Block::default()
                .title(" > ")
                .borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(theme::border_active())
                .border_type(BorderType::Rounded)
                .style(Style::default().bg(theme::BG_PANEL)),
        );
    frame.render_widget(input_widget, area);
}

/// Settings entry definition: label, config JSON path, hint, and whether it's a toggle.
struct SettingsEntry {
    label: &'static str,
    path: &'static str, // dot-path into config JSON (e.g. "gateway.host")
    hint: &'static str,
    toggle: bool,       // if true, Enter toggles between true/false or enabled/disabled
    select: &'static [&'static str], // if non-empty, Enter cycles through these options
}

// No hardcoded model list — model field is free-text input.
// User types their provider/model string directly (e.g. "ollama/qwen3" or "anthropic/claude-sonnet-4-6").
const MODEL_OPTIONS: &[&str] = &[];

const VERBOSITY_OPTIONS: &[&str] = &["quiet", "normal", "verbose"];
const SECURITY_OPTIONS: &[&str] = &["minimal", "standard", "strict"];

/// Settings entries generated from the full settings_fields::ALL_FIELDS definition.
/// This gives the TUI access to all 45 config fields across 11 sections.
fn build_settings_entries() -> Vec<SettingsEntry> {
    use crate::screens::settings_fields::{ALL_FIELDS, FieldKind};
    ALL_FIELDS.iter().map(|f| {
        let (toggle, select) = match &f.kind {
            FieldKind::Toggle => (true, &[] as &[&str]),
            FieldKind::Select(opts) => (false, *opts),
            _ => (false, &[] as &[&str]),
        };
        SettingsEntry {
            label: f.label,
            path: f.key,
            hint: f.hint,
            toggle,
            select,
        }
    }).collect()
}

/// Number of settings entries (used for cursor bounds in key handlers).
pub fn settings_count() -> usize {
    crate::screens::settings_fields::ALL_FIELDS.len()
}

/// Return the config paths for each settings entry (used by key handlers).
pub fn settings_entry_paths() -> Vec<&'static str> {
    build_settings_entries().iter().map(|e| e.path).collect()
}

/// Return whether each settings entry is a toggle (used by key handlers).
pub fn settings_entry_toggles() -> Vec<bool> {
    build_settings_entries().iter().map(|e| e.toggle).collect()
}

/// Return the select options for each settings entry (empty slice = free text).
pub fn settings_entry_selects() -> Vec<&'static [&'static str]> {
    build_settings_entries().iter().map(|e| e.select).collect()
}

/// Public wrapper around resolve_config for use from lib.rs key handlers.
pub fn resolve_config_pub(config: &serde_json::Value, path: &str) -> String {
    let v = resolve_config(config, path);
    if v.is_empty() { "not set".into() } else { v }
}

/// Build a JSON object for PUT /v1/config from dirty edits.
/// Maps dot-paths back into the nested ConfigUpdateRequest shape.
pub fn build_config_update(dirty: &std::collections::HashMap<String, String>) -> serde_json::Value {
    let mut update = serde_json::Map::new();
    for (path, value) in dirty {
        match path.as_str() {
            "model" => { update.insert("model".into(), serde_json::Value::String(value.clone())); }
            "max_iterations" => {
                if let Ok(n) = value.parse::<u64>() {
                    update.insert("max_iterations".into(), serde_json::Value::Number(n.into()));
                }
            }
            "verbosity" => { update.insert("verbosity".into(), serde_json::Value::String(value.clone())); }
            p if p.starts_with("gateway.") => {
                let key = p.strip_prefix("gateway.").unwrap();
                let gw = update.entry("gateway".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(obj) = gw.as_object_mut() {
                    let val = match value.as_str() {
                        "enabled" | "true" => serde_json::Value::Bool(true),
                        "disabled" | "false" => serde_json::Value::Bool(false),
                        _ => {
                            if let Ok(n) = value.parse::<u64>() {
                                serde_json::Value::Number(n.into())
                            } else {
                                serde_json::Value::String(value.clone())
                            }
                        }
                    };
                    obj.insert(key.into(), val);
                }
            }
            p if p.starts_with("prometheus.") => {
                let key = p.strip_prefix("prometheus.").unwrap();
                let prom = update.entry("prometheus".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(obj) = prom.as_object_mut() {
                    let val = match value.as_str() {
                        "enabled" | "true" => serde_json::Value::Bool(true),
                        "disabled" | "false" => serde_json::Value::Bool(false),
                        _ => serde_json::Value::String(value.clone()),
                    };
                    obj.insert(key.into(), val);
                }
            }
            p if p.starts_with("aegis.") => {
                let key = p.strip_prefix("aegis.").unwrap();
                let aegis = update.entry("aegis".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(obj) = aegis.as_object_mut() {
                    obj.insert(key.into(), serde_json::Value::String(value.clone()));
                }
            }
            p if p.starts_with("mnemosyne.") => {
                let key = p.strip_prefix("mnemosyne.").unwrap();
                let mn = update.entry("mnemosyne".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(obj) = mn.as_object_mut() {
                    let val = match value.as_str() {
                        "enabled" | "true" => serde_json::Value::Bool(true),
                        "disabled" | "false" => serde_json::Value::Bool(false),
                        _ => serde_json::Value::String(value.clone()),
                    };
                    obj.insert(key.into(), val);
                }
            }
            p if p.starts_with("tui.") => {
                let key = p.strip_prefix("tui.").unwrap();
                let tui = update.entry("tui".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(obj) = tui.as_object_mut() {
                    let val = match value.as_str() {
                        "enabled" | "true" => serde_json::Value::Bool(true),
                        "disabled" | "false" => serde_json::Value::Bool(false),
                        _ => serde_json::Value::String(value.clone()),
                    };
                    obj.insert(key.into(), val);
                }
            }
            _ => {
                // Generic: just set as string at top level
                update.insert(path.clone(), serde_json::Value::String(value.clone()));
            }
        }
    }
    serde_json::Value::Object(update)
}

/// Resolve a dot-path value from the config JSON.
fn resolve_config(config: &serde_json::Value, path: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = config;
    for part in &parts {
        current = match current.get(part) {
            Some(v) => v,
            None => return String::new(),
        };
    }
    match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => if *b { "enabled".into() } else { "disabled".into() },
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(Span::styled(" Settings ", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(theme::border_active())
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(theme::BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let has_config = !app.settings_config.is_null();
    let has_dirty = !app.settings_dirty.is_empty();

    let entries = build_settings_entries();
    let items: Vec<ListItem> = entries.iter().enumerate().map(|(i, entry)| {
        let is_selected = app.settings_cursor == i;
        let is_editing = is_selected && app.settings_editing;

        // Value resolution: dirty edits > config > fallback
        let value = if is_editing {
            // Show edit buffer with cursor
            let buf = &app.settings_edit_value;
            let cursor = app.settings_edit_cursor.min(buf.len());
            let (before, after) = buf.split_at(cursor);
            format!("{}█{}", before, after)
        } else if let Some(dirty) = app.settings_dirty.get(entry.path) {
            dirty.clone()
        } else if has_config {
            let v = resolve_config(&app.settings_config, entry.path);
            if v.is_empty() { "not set".into() } else { v }
        } else {
            "loading...".into()
        };

        let label_style = if is_selected {
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        let value_style = if is_editing {
            Style::default().fg(Color::Rgb(255, 200, 100)).add_modifier(Modifier::BOLD)
        } else if app.settings_dirty.contains_key(entry.path) {
            Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
        } else {
            theme::text()
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!(" {:<18}", entry.label), label_style),
                Span::styled(value, value_style),
            ]),
        ];
        // Only show hint for selected row to keep it compact
        if is_selected {
            let hint_text = if is_editing && entry.toggle {
                format!("  {} — Enter to toggle, Esc to cancel", entry.hint)
            } else if is_editing {
                format!("  {} — Type value, Enter to confirm, Esc to cancel", entry.hint)
            } else {
                format!("  {}", entry.hint)
            };
            lines.push(Line::styled(hint_text, Style::default().fg(theme::MUTED)));
        }

        ListItem::new(lines).style(if is_selected { Style::default().bg(Color::Rgb(30, 15, 10)) } else { Style::default() })
    }).collect();

    // Layout: settings list + status + action bar
    let layout = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
    ]).split(inner);

    // Scroll the list to keep cursor visible
    let visible_rows = layout[0].height as usize;
    let scroll_offset = if app.settings_cursor >= visible_rows.saturating_sub(2) {
        app.settings_cursor.saturating_sub(visible_rows.saturating_sub(3))
    } else {
        0
    };
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(app.settings_cursor));
    *list_state.offset_mut() = scroll_offset;
    frame.render_stateful_widget(
        List::new(items).highlight_symbol(""),
        layout[0],
        &mut list_state,
    );

    // Status line
    let status_text = if !app.settings_status.is_empty() {
        app.settings_status.as_str()
    } else if has_dirty {
        " Unsaved changes (S to save, C to discard)"
    } else if !has_config {
        " Connecting to gateway..."
    } else {
        ""
    };
    let status_style = if status_text.contains("Saved") || status_text.contains("success") {
        Style::default().fg(theme::GREEN)
    } else if status_text.contains("Unsaved") || status_text.contains("Error") {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::MUTED)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(status_text, status_style)),
        layout[1],
    );

    // Action bar
    let actions = if app.settings_editing {
        Line::from(vec![
            Span::styled(" Type to edit ", Style::default().fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", theme::muted()),
            Span::styled(" Enter Confirm ", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", theme::muted()),
            Span::styled(" Esc Cancel ", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" [S]ave ", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", theme::muted()),
            Span::styled(" [C]ancel ", Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", theme::muted()),
            Span::styled(" [R]estart Gateway ", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", theme::muted()),
            Span::styled(" ↑↓ Navigate  Enter Edit  Esc Back ", theme::muted()),
        ])
    };
    let action_block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme::border())
        .style(Style::default().bg(theme::BG_PANEL));
    frame.render_widget(
        Paragraph::new(actions).block(action_block),
        layout[2],
    );
}

fn render_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let layout = Layout::vertical([
        Constraint::Percentage(50), // agents
        Constraint::Percentage(50), // channels
    ]).split(area);

    // Agents
    let agent_block = Block::default()
        .title(Line::styled(" agents ", theme::label()))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(theme::border())
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(theme::BG_PANEL));
    let agent_inner = agent_block.inner(layout[0]);
    frame.render_widget(agent_block, layout[0]);

    let agent_items: Vec<ListItem> = if app.agents.is_empty() {
        vec![ListItem::new(Line::styled("  (none)", theme::muted()))]
    } else {
        app.agents.iter().map(|agent| {
            let (icon, style) = match agent.status {
                crate::app::AgentStatus::Running => ("●", theme::success()),
                crate::app::AgentStatus::Idle => ("●", theme::warning()),
                crate::app::AgentStatus::Completed => ("○", theme::muted()),
                crate::app::AgentStatus::Error => ("✗", Style::default().fg(theme::RED)),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", icon), style),
                Span::styled(&agent.name, theme::text()),
            ]))
        }).collect()
    };
    frame.render_widget(List::new(agent_items), agent_inner);

    // Channels
    let ch_block = Block::default()
        .title(Line::styled(" channels ", theme::label()))
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(theme::border())
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(theme::BG_PANEL));
    let ch_inner = ch_block.inner(layout[1]);
    frame.render_widget(ch_block, layout[1]);

    let ch_items: Vec<ListItem> = if app.channels.is_empty() {
        vec![ListItem::new(Line::styled("  (none)", theme::muted()))]
    } else {
        app.channels.iter().map(|ch| {
            let (icon, style) = match ch.status {
                crate::app::ChannelStatus::Connected => ("●", theme::success()),
                crate::app::ChannelStatus::Relay => ("◉", Style::default().fg(theme::PURPLE)),
                crate::app::ChannelStatus::Offline => ("○", theme::muted()),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", icon), style),
                Span::styled(&ch.name, theme::text()),
            ]))
        }).collect()
    };
    frame.render_widget(List::new(ch_items), ch_inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Tab, PantheonSelection};

    #[test]
    fn test_active_tab_defaults_to_chat() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.active_tab, Tab::Chat);
    }

    #[test]
    fn test_tab_enum_pantheon_variant_exists() {
        let tab = Tab::Pantheon;
        assert_eq!(tab, Tab::Pantheon);
    }

    #[test]
    fn test_pantheon_selection_rooms_default() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.pantheon_selected, PantheonSelection::Rooms);
    }

    #[test]
    fn test_pantheon_rooms_empty_by_default() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_rooms.is_empty());
    }

    #[test]
    fn test_pantheon_missions_empty_by_default() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_missions.is_empty());
    }

    #[test]
    fn test_channel_badge_discord() {
        let (text, _style) = channel_badge("discord");
        assert_eq!(text, "[discord]");
    }

    #[test]
    fn test_channel_badge_telegram() {
        let (text, _style) = channel_badge("telegram");
        assert_eq!(text, "[telegram]");
    }

    #[test]
    fn test_channel_badge_tui() {
        let (text, _style) = channel_badge("tui");
        assert_eq!(text, "[tui]");
    }

    #[test]
    fn test_channel_badge_slack() {
        let (text, _style) = channel_badge("slack");
        assert_eq!(text, "[slack]");
    }

    #[test]
    fn test_channel_badge_api() {
        let (text, _style) = channel_badge("api");
        assert_eq!(text, "[api]");
    }

    #[test]
    fn test_channel_badge_unknown_uses_brackets() {
        let (text, _style) = channel_badge("matrix");
        assert_eq!(text, "[matrix]");
    }
}
