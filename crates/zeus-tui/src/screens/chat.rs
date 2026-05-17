#![allow(dead_code)]
//! Chat screen — pre-wrapped text, virtual scroll, NO Paragraph::wrap
//! Owner: mikes-Mac-mini (feat/s68-tui-chat)
//!
//! Rules enforced:
//! - NO Paragraph::wrap — all wrapping is manual, done before render
//! - Virtual scroll: only visible lines are rendered
//! - Under 300 lines

use crossterm::event::{KeyCode, KeyEvent};
use unicode_width::UnicodeWidthStr;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::{App, Role};
use crate::diff_viewer;
use crate::markdown::render_markdown_with_width;
use crate::theme;
use super::{Action, Screen};

pub struct ChatScreen;

// ── Pre-wrap helpers ──────────────────────────────────────────────────────────

/// Wrap a string to `width` columns, returning owned lines.
/// This is the ONLY place text wrapping happens — no Paragraph::wrap anywhere.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut col = 0usize;
        for word in raw_line.split_whitespace() {
            // Use display width (handles CJK, emoji, multi-byte codepoints)
            let wlen = UnicodeWidthStr::width(word);
            if col == 0 {
                current.push_str(word);
                col = wlen;
            } else if col + 1 + wlen <= width {
                current.push(' ');
                current.push_str(word);
                col += 1 + wlen;
            } else {
                lines.push(current.clone());
                current = word.to_string();
                col = wlen;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

/// Render a single chat message into a list of `ListItem`s (pre-wrapped).
/// When `msg.streaming` is true, appends a `▌` cursor to the last content
/// line so the user can see tokens arriving in real-time.
/// `search_highlight`: 0 = no match, 1 = match, 2 = focused match
fn message_to_items(msg: &crate::app::ChatMessage, width: usize, search_highlight: u8) -> Vec<ListItem<'static>> {
    // Search highlight background: focused = bright yellow bg, match = dim yellow bg
    let highlight_bg = match search_highlight {
        2 => Some(ratatui::style::Color::Rgb(80, 70, 0)),   // focused match — amber
        1 => Some(ratatui::style::Color::Rgb(40, 35, 0)),   // non-focused match — dim
        _ => None,
    };

    let (role_label, role_style) = match msg.role {
        Role::User      => (" YOU ", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
        Role::Assistant => (" AI  ", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
        Role::System    => (" SYS ", Style::default().fg(theme::YELLOW)),
        Role::Tool      => (" TOOL", Style::default().fg(theme::PURPLE)),
    };

    // While streaming, show "[streaming]" badge next to role label
    let streaming_badge = if msg.streaming { " ▶" } else { "" };

    let agent = msg.agent_name.as_deref().unwrap_or("");
    let ts    = &msg.timestamp;

    // Header line: [ROLE] agentname  HH:MM:SS  [streaming badge]
    let header = Line::from(vec![
        Span::styled(role_label, role_style),
        Span::raw(" "),
        Span::styled(agent.to_string(), theme::bright()),
        Span::raw("  "),
        Span::styled(ts.clone(), theme::label()),
        Span::styled(streaming_badge.to_string(), Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)),
    ]);

    let mut items = vec![ListItem::new(header)];

    // Content lines — rendered with markdown, indent by 2 spaces
    let indent = "  ";
    let wrap_width = width.saturating_sub(indent.len());

    // If this message has a live stream_state, split flushed (rendered) from
    // pending (raw, cursor-suffixed).  Otherwise render the full content.
    let (flushed_text, pending_tail) = if let Some(ref ss) = msg.stream_state {
        (ss.flushed().to_string(), Some(ss.pending().to_string()))
    } else {
        (msg.content.clone(), None)
    };

    // Render the flushed portion — diff blocks get syntax highlighting,
    // everything else goes through the markdown renderer.
    if diff_viewer::contains_diff(&flushed_text) {
        for line in diff_viewer::render_diff_block(&flushed_text, indent) {
            items.push(ListItem::new(line));
        }
    } else {
        for line in render_markdown_with_width(&flushed_text, indent, wrap_width) {
            items.push(ListItem::new(line));
        }
    }

    // Pending tail: raw text + streaming cursor
    if let Some(mut tail) = pending_tail {
        if msg.streaming {
            tail.push('▌');
        }
        if !tail.is_empty() {
            for raw_line in wrap_text(&tail, wrap_width) {
                let padded = format!("{indent}{raw_line}");
                items.push(ListItem::new(Line::from(Span::styled(padded, theme::text()))));
            }
        }
    } else if msg.streaming {
        // No stream_state but still streaming — append cursor to last content line
        // (legacy path, keeps backwards compat)
        if let Some(last) = items.last_mut() {
            // Append cursor span to whatever's already rendered
            *last = ListItem::new(Line::from(vec![
                Span::raw("  ▌"),
            ]));
        }
    }

    // Blank separator
    items.push(ListItem::new(Line::from(Span::raw(""))));

    // Apply search highlight background to all items in this message
    if let Some(bg) = highlight_bg {
        items = items
            .into_iter()
            .map(|item| {
                let lines: Vec<Line> = item
                    .content()
                    .lines
                    .iter()
                    .map(|line| {
                        let spans: Vec<Span> = line
                            .spans
                            .iter()
                            .map(|s| {
                                Span::styled(
                                    s.content.clone().into_owned(),
                                    s.style.bg(bg),
                                )
                            })
                            .collect();
                        Line::from(spans)
                    })
                    .collect();
                ListItem::new(lines)
            })
            .collect();
    }

    items
}

// ── Screen impl ───────────────────────────────────────────────────────────────

impl Screen for ChatScreen {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App) {
        // Split into messages area + optional search bar + input bar
        let constraints = if app.search_active {
            vec![Constraint::Min(1), Constraint::Length(3), Constraint::Length(3)]
        } else {
            vec![Constraint::Min(1), Constraint::Length(3)]
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let msg_area   = chunks[0];
        let search_area = if app.search_active { Some(chunks[1]) } else { None };
        let input_area = if app.search_active { chunks[2] } else { chunks[1] };

        // ── Build all pre-wrapped items ──────────────────────────────────────
        // Pass the inner area width (post-border) to message_to_items so it
        // can correctly subtract only the indent, not the border again.
        // Block borders consume 1 cell on each side → inner_width = outer - 2.
        let inner_width = msg_area.width.saturating_sub(2) as usize;
        let focused_match_idx = app.search_matches.get(app.search_match_idx).copied();
        let all_items: Vec<ListItem> = app
            .messages
            .iter()
            .enumerate()
            .flat_map(|(i, m)| {
                let hl = if app.search_active && !app.search_query.is_empty() {
                    if Some(i) == focused_match_idx {
                        2
                    } else if app.search_matches.contains(&i) {
                        1
                    } else {
                        0
                    }
                } else {
                    0
                };
                message_to_items(m, inner_width, hl)
            })
            .collect();

        let total_lines = all_items.len();
        let visible     = msg_area.height as usize;

        // Virtual scroll: clamp offset, then slice
        let max_offset = total_lines.saturating_sub(visible);
        let offset     = app.scroll_offset.min(max_offset);
        let end        = (offset + visible).min(total_lines);
        let visible_items = all_items[offset..end].to_vec();

        // Scroll indicator in title
        let scroll_info = if total_lines > visible {
            format!(" [{}/{}] ↑↓ scroll ", offset + 1, total_lines)
        } else {
            String::new()
        };

        let msg_block = Block::default()
            .title(format!("◈ Chat{scroll_info}"))
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(theme::border_active())
            .style(Style::default().bg(theme::BG));

        let list = List::new(visible_items).block(msg_block);
        frame.render_widget(list, msg_area);

        // ── Search bar (S100 #13) ─────────────────────────────────────────────
        if let Some(sa) = search_area {
            let match_info = if app.search_query.is_empty() {
                String::new()
            } else if app.search_matches.is_empty() {
                " [no matches]".to_string()
            } else {
                format!(" [{}/{}]  n/N to jump", app.search_match_idx + 1, app.search_matches.len())
            };
            // Build cursor-bearing query display
            let q_chars: Vec<char> = app.search_query.chars().collect();
            let before: String = q_chars[..app.search_cursor].iter().collect();
            let after: String  = q_chars[app.search_cursor..].iter().collect();
            let search_text = format!("{before}█{after}{match_info}");
            let search_widget = Paragraph::new(search_text)
                .style(theme::bright())
                .block(
                    Block::default()
                        .title(" 🔍 Search  [Esc] exit  [n/N] next/prev ")
                        .borders(Borders::ALL).border_type(BorderType::Rounded)
                        .border_style(ratatui::style::Style::default()
                            .fg(ratatui::style::Color::Yellow))
                        .style(ratatui::style::Style::default()
                            .bg(crate::theme::BG_PANEL)),
                );
            frame.render_widget(search_widget, sa);
        }

        // ── Input bar ────────────────────────────────────────────────────────
        let input_text = format!("{}█", app.input);

        let input_widget = Paragraph::new(input_text)
            .style(theme::bright())
            .block(
                Block::default()
                    .title(" Type a message… [Enter] send  [Tab] switch screen ")
                    .borders(Borders::ALL).border_type(BorderType::Rounded)
                    .border_style(theme::border_active())
                    .style(Style::default().bg(theme::BG_PANEL)),
            );

        frame.render_widget(input_widget, input_area);
    }

    fn handle_input(&self, key: KeyEvent, app: &mut App) -> Action {
        // Chat input is handled in main.rs event loop — always active
        match key.code {
            KeyCode::Up   => { app.scroll_up();   Action::Continue }
            KeyCode::Down => { app.scroll_down(); Action::Continue }
            _ => Action::Continue,
        }
    }
}
