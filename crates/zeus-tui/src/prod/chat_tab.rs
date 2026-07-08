use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme;
use crate::widgets::{FaceState, face_frame};

/// Chat message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    ToolCall,
    System,
}

/// A single chat message.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    pub tool_name: Option<String>,
}

/// Streaming state indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    Streaming,
    Queued,
}

/// A live tool-usage feed item displayed during a streaming cook.
#[derive(Debug, Clone)]
pub struct ToolFeedItem {
    /// Tool name (e.g. "shell", "read_file").
    pub name: String,
    /// Short input summary (first ~60 chars of the tool input).
    pub input_summary: String,
    /// Whether this tool has completed.
    pub done: bool,
    /// Short output summary (first ~60 chars), set on completion.
    pub output_summary: String,
}

/// Chat tab — matches JSX ChatTab (line 156).
/// Message list (user/assistant, role colors), input line, `/`-slash overlay,
/// streaming/cooking indicator.
pub struct ChatTab<'a> {
    pub messages: &'a [ChatMessage],
    pub input: &'a str,
    pub stream_state: StreamState,
    pub scroll_offset: usize,
    pub slash_open: bool,
    /// Blink phase from `App::cursor_visible()` — drives the composer's
    /// insertion caret (set by the caller each frame).
    pub cursor_on: bool,
    /// Monotonic animation tick from the app run-loop; drives the prototype
    /// ZeusFace cursor at the bottom-left of the composer.
    pub anim_tick: u64,
    /// Live tool-usage feed items during a streaming cook.
    pub tool_feed: &'a [ToolFeedItem],
    /// Current cook iteration count (from `iter` SSE events).
    pub iter_count: u32,
    /// Live active agent tasks from `GET /v1/tasks/active` (#280). `None` until
    /// the first poll lands; an empty slice means no active tasks. When there
    /// are active tasks, a Claude-Code-style tracker panel renders at the top
    /// of the message area (status glyph + content per row).
    pub active_tasks: Option<&'a [crate::api::TaskResponse]>,
    /// Current model/provider badge for the chat header and assistant rows.
    pub model_badge: Option<&'a str>,
    /// Context-window usage percent for the prototype ctx gauge.
    pub ctx_percent: u8,
}

// Map a task status string to its Claude-Code-style glyph + color.
// `pending` → `☐` dim, `active`/`in_progress` → `◐` accent (highlighted),
// `completed` → `☑` green, anything else (paused/failed) → `☒` muted.
fn trunc_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn wrap_plain_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let width = width.max(1) as usize;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let words: Vec<&str> = raw.split_whitespace().collect();
        if words.is_empty() {
            lines.push(Line::default());
            continue;
        }
        let mut line_buf = String::new();
        for word in words {
            let word_len = word.chars().count();
            if word_len > width {
                if !line_buf.is_empty() {
                    lines.push(Line::from(Span::styled(
                        std::mem::take(&mut line_buf),
                        style,
                    )));
                }

                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if chunk.chars().count() == width {
                        lines.push(Line::from(Span::styled(std::mem::take(&mut chunk), style)));
                    }
                }
                if !chunk.is_empty() {
                    line_buf = chunk;
                }
                continue;
            }

            let sep = usize::from(!line_buf.is_empty());
            if line_buf.chars().count() + word_len + sep > width && !line_buf.is_empty() {
                lines.push(Line::from(Span::styled(
                    std::mem::take(&mut line_buf),
                    style,
                )));
            }
            if !line_buf.is_empty() {
                line_buf.push(' ');
            }
            line_buf.push_str(word);
        }
        if !line_buf.is_empty() {
            lines.push(Line::from(Span::styled(line_buf, style)));
        }
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn line_text(line: Line<'static>) -> String {
    line.spans
        .into_iter()
        .map(|span| span.content.to_string())
        .collect::<String>()
}

fn line_cell_width(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum()
}

struct ChatBox<'a> {
    label: &'a str,
    label_color: ratatui::style::Color,
    body: Vec<Line<'static>>,
}

fn push_chat_box(all: &mut Vec<(u16, Line<'static>)>, x: u16, width: u16, box_: ChatBox<'_>) {
    let inner = width.saturating_sub(4).max(8) as usize;
    let border_w = inner + 2;
    all.push((
        x,
        Line::from(Span::styled(
            format!("╭{}╮", "─".repeat(border_w)),
            Style::default().fg(theme::MUTED),
        )),
    ));
    all.push((
        x,
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme::MUTED)),
            Span::styled(
                box_.label.to_string(),
                Style::default()
                    .fg(box_.label_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ));
    for line in box_.body {
        let mut spans = Vec::with_capacity(line.spans.len() + 1);
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.extend(line.spans);
        all.push((x, Line::from(spans)));
    }
    all.push((
        x,
        Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(border_w)),
            Style::default().fg(theme::MUTED),
        )),
    ));
}

struct ToolCard<'a> {
    name: &'a str,
    args_or_input: &'a str,
    output: Option<&'a str>,
    done: bool,
    error: bool,
}

fn push_tool_card(all: &mut Vec<(u16, Line<'static>)>, x: u16, width: u16, card: ToolCard<'_>) {
    let ToolCard {
        name,
        args_or_input,
        output,
        done,
        error,
    } = card;
    let inner = width.saturating_sub(4).max(8) as usize;
    let border_w = inner + 2;
    let (glyph, status, status_style) = if error {
        (
            "✗",
            "error",
            Style::default()
                .fg(theme::FIRE_ORANGE)
                .add_modifier(Modifier::BOLD),
        )
    } else if done {
        (
            "✓",
            "success",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "◉",
            "running",
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )
    };
    all.push((
        x,
        Line::from(Span::styled(
            format!("╭{}╮", "─".repeat(border_w)),
            Style::default().fg(theme::MUTED),
        )),
    ));
    all.push((
        x,
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "⚙ tool_call",
                Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(theme::DIM)),
            Span::styled(
                trunc_chars(name, inner / 2),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(theme::DIM)),
            Span::styled(format!("{glyph} {status}"), status_style),
        ]),
    ));
    if !args_or_input.trim().is_empty() {
        all.push((
            x,
            Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme::MUTED)),
                Span::styled("args ", Style::default().fg(theme::DIM)),
                Span::styled(
                    trunc_chars(args_or_input.trim(), inner.saturating_sub(7)),
                    Style::default().fg(theme::DIM),
                ),
            ]),
        ));
    }
    if let Some(output) = output.filter(|o| !o.trim().is_empty()) {
        for (idx, line) in wrap_plain_lines(
            output.trim(),
            inner.saturating_sub(9) as u16,
            Style::default().fg(theme::TEXT),
        )
        .into_iter()
        .take(6)
        .enumerate()
        {
            let prefix = if idx == 0 {
                "│ result "
            } else {
                "│        "
            };
            let prefix_style = if idx == 0 {
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            all.push((
                x,
                Line::from(vec![
                    Span::styled(prefix, prefix_style),
                    Span::styled(line_text(line), Style::default().fg(theme::TEXT)),
                ]),
            ));
        }
    }
    all.push((
        x,
        Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(border_w)),
            Style::default().fg(theme::MUTED),
        )),
    ));
}

fn task_glyph(status: &str) -> (&'static str, ratatui::style::Color) {
    match status {
        "completed" => ("\u{2611}", theme::GREEN),             // ☑
        "active" | "in_progress" => ("\u{25d0}", theme::CYAN), // ◐
        "failed" => ("\u{2612}", theme::FIRE_ORANGE),          // ☒
        "paused" => ("\u{25d0}", theme::MUTED),                // ◐ dim
        _ => ("\u{2610}", theme::DIM),                         // ☐ pending
    }
}

fn composer_face_state(stream_state: StreamState, input: &str) -> FaceState {
    match stream_state {
        StreamState::Streaming => FaceState::Working,
        StreamState::Queued => FaceState::Queued,
        StreamState::Idle if !input.is_empty() => FaceState::Listening,
        StreamState::Idle => FaceState::Ready,
    }
}

impl<'a> Widget for ChatTab<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height < 3 {
            return;
        }

        let placeholder = if self.stream_state == StreamState::Streaming {
            "type to queue (input never blocks)…"
        } else {
            "message…"
        };
        let face_width = 8u16;
        let composer_prefix_width = face_width + 3; // face + ` │ ` separator
        let input_text_width = area.width.saturating_sub(composer_prefix_width + 1).max(1);
        let mut input_lines = if self.input.is_empty() {
            vec![Line::from(Span::styled(
                placeholder,
                Style::default().fg(theme::DIM),
            ))]
        } else {
            wrap_plain_lines(
                self.input,
                input_text_width,
                Style::default().fg(theme::TEXT),
            )
        };
        if !self.input.is_empty() && self.cursor_on {
            let last_width = input_lines.last().map(line_cell_width).unwrap_or_default();
            if last_width >= input_text_width as usize {
                input_lines.push(Line::from(Span::styled(
                    "▏",
                    Style::default().fg(theme::AMBER),
                )));
            } else if let Some(last) = input_lines.last_mut() {
                last.spans
                    .push(Span::styled("▏", Style::default().fg(theme::AMBER)));
            }
        }

        let max_input_rows = usize::from(area.height.saturating_sub(2).clamp(1, 5));
        let visible_input_rows = input_lines.len().max(1).min(max_input_rows);
        let input_height = (visible_input_rows as u16)
            .saturating_add(2)
            .min(area.height);

        // Reserve a growing bottom input bar: top border + wrapped input rows + hint/overlay row.
        let msg_height = area.height.saturating_sub(input_height);
        let msg_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: msg_height,
        };
        let input_area = Rect {
            x: area.x,
            y: area.y + msg_height,
            width: area.width,
            height: input_height,
        };

        // Fill message area background
        for y in msg_area.top()..msg_area.bottom() {
            for x in msg_area.left()..msg_area.right() {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(theme::BG));
            }
        }

        // Build the full flat line buffer first, THEN window by line from the
        // bottom. Message-granularity scrolling (the old `start = end -
        // visible_height` over message *indices*) clipped tall messages: a long
        // streaming reply overflowed `msg_area.bottom()` and the newest tokens
        // fell off-screen. Line-granularity bottom-anchoring fixes that —
        // `scroll_offset` is now *lines from the bottom*, so offset 0 always
        // pins the newest line on screen as content streams in (auto-follow).
        // Scrolling up (offset > 0) pauses the follow; jumping to bottom
        // (offset → 0) resumes it.
        let text_width = msg_area.width.saturating_sub(4);

        // Each entry is (x-origin, line) so prefixes (col 0) and wrapped body
        // text (col 4) keep their indents in the flat buffer.
        let mut all: Vec<(u16, Line<'static>)> = Vec::new();

        let ctx = self.ctx_percent.min(100);
        let filled = ((ctx as usize * 10) / 100).min(10);
        let gauge = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
        let ctx_style = if ctx < 60 {
            Style::default().fg(theme::GREEN)
        } else if ctx < 80 {
            Style::default().fg(theme::YELLOW)
        } else {
            Style::default().fg(theme::FIRE_ORANGE)
        };
        all.push((
            msg_area.x + 2,
            Line::from(vec![
                Span::styled("ctx ", Style::default().fg(theme::DIM)),
                Span::styled(gauge, ctx_style),
                Span::styled(format!(" {ctx}%"), ctx_style),
                Span::styled(" │ model ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    self.model_badge.unwrap_or("model:unknown").to_string(),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ));
        all.push((msg_area.x, Line::default()));

        // ── Task-tracker panel (#280) ──
        // Claude-Code-style live todo list, fed by `GET /v1/tasks/active`. Only
        // shown when there are active tasks, so an idle chat stays uncluttered.
        // One row per task: status glyph + content, in_progress highlighted.
        if let Some(tasks) = self.active_tasks
            && !tasks.is_empty()
        {
            let done = tasks.iter().filter(|t| t.status == "completed").count();
            all.push((
                msg_area.x + 2,
                Line::from(Span::styled(
                    format!(
                        "\u{2500}\u{2500} tasks ({done}/{}) \u{2500}\u{2500}",
                        tasks.len()
                    ),
                    Style::default().fg(theme::MUTED),
                )),
            ));
            for task in tasks {
                let (glyph, color) = task_glyph(&task.status);
                let highlight = task.status == "active" || task.status == "in_progress";
                let mut content_style =
                    Style::default().fg(if highlight { theme::TEXT } else { theme::DIM });
                if highlight {
                    content_style = content_style.add_modifier(Modifier::BOLD);
                }
                // Truncate long content to keep one row per task.
                let max_content = text_width.saturating_sub(4) as usize;
                let content = if task.description.chars().count() > max_content && max_content > 1 {
                    let mut c: String = task.description.chars().take(max_content - 1).collect();
                    c.push('\u{2026}');
                    c
                } else {
                    task.description.clone()
                };
                all.push((
                    msg_area.x + 4,
                    Line::from(vec![
                        Span::styled(format!("{glyph} "), Style::default().fg(color)),
                        Span::styled(content, content_style),
                    ]),
                ));
            }
            all.push((msg_area.x, Line::default()));
        }

        for msg in self.messages {
            let (label, label_color) = match msg.role {
                Role::User => ("▸ you", theme::ACCENT),
                Role::Assistant => ("◈ zeus", theme::CYAN),
                Role::ToolCall => ("⚙ tool", theme::YELLOW),
                Role::System => ("● sys", theme::DIM),
            };

            let text_color = match msg.role {
                Role::User => theme::TEXT,
                Role::Assistant => theme::TEXT,
                Role::ToolCall => theme::DIM,
                Role::System => theme::DIM,
            };

            // User/assistant/system messages render in bordered chat boxes.
            // Assistant/system content still goes through markdown; user input stays plain.
            // Tool calls keep their richer prototype-style bordered cards.
            match msg.role {
                Role::Assistant | Role::System => {
                    let mut body = Vec::new();
                    if msg.role == Role::Assistant {
                        let badge = self.model_badge.unwrap_or("model:unknown");
                        body.push(Line::from(vec![
                            Span::styled("model ", Style::default().fg(theme::DIM)),
                            Span::styled(
                                badge.to_string(),
                                Style::default()
                                    .fg(theme::ACCENT)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    }
                    body.extend(crate::prod::markdown::render_markdown(
                        &msg.text,
                        text_width.saturating_sub(4),
                        text_color,
                    ));
                    push_chat_box(
                        &mut all,
                        msg_area.x + 2,
                        text_width,
                        ChatBox {
                            label,
                            label_color,
                            body,
                        },
                    );
                }
                Role::ToolCall => {
                    let tool = msg.tool_name.as_deref().unwrap_or("tool");
                    let lower = msg.text.to_ascii_lowercase();
                    let error =
                        lower.contains("error") || lower.contains("failed") || lower.contains("✗");
                    push_tool_card(
                        &mut all,
                        msg_area.x + 2,
                        text_width,
                        ToolCard {
                            name: tool,
                            args_or_input: &msg.text,
                            output: Some(&msg.text),
                            done: true,
                            error,
                        },
                    );
                }
                Role::User => {
                    let body = wrap_plain_lines(
                        &msg.text,
                        text_width.saturating_sub(4),
                        Style::default().fg(text_color),
                    );
                    push_chat_box(
                        &mut all,
                        msg_area.x + 2,
                        text_width,
                        ChatBox {
                            label,
                            label_color,
                            body,
                        },
                    );
                }
            }

            // Blank line between messages.
            all.push((msg_area.x, Line::default()));
        }

        // Live tool-usage feed during streaming cook — replaces the old
        // static "◈ zeus is thinking..." with real-time tool activity.
        if matches!(
            self.stream_state,
            StreamState::Queued | StreamState::Streaming
        ) {
            if self.stream_state == StreamState::Queued || self.tool_feed.is_empty() {
                // Send-feedback P1: after Enter, paint an immediate pending state
                // until the assistant/tool stream produces visible output.
                let label = if self.stream_state == StreamState::Queued {
                    "◈ sending / working..."
                } else {
                    "◈ zeus is thinking..."
                };
                all.push((
                    msg_area.x + 4,
                    Line::from(Span::styled(
                        label.to_string(),
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::ITALIC),
                    )),
                ));
            } else {
                // Iteration header (shown once per iter boundary).
                if self.iter_count > 0 {
                    all.push((
                        msg_area.x + 2,
                        Line::from(Span::styled(
                            format!("── iter {} ──", self.iter_count),
                            Style::default().fg(theme::MUTED),
                        )),
                    ));
                }
                // Render each tool feed item as the same bordered card style used
                // for persisted tool-call messages.
                for item in self.tool_feed.iter() {
                    push_tool_card(
                        &mut all,
                        msg_area.x + 2,
                        text_width,
                        ToolCard {
                            name: &item.name,
                            args_or_input: &item.input_summary,
                            output: Some(&item.output_summary),
                            done: item.done,
                            error: false,
                        },
                    );
                }
            }
        }

        // Window the last `visible_height` lines, offset upward by
        // `scroll_offset` lines from the bottom. Clamp the offset so it can
        // never scroll past the top (no blank void above the first line).
        let visible_height = msg_area.height as usize;
        let total_lines = all.len();
        let max_offset = total_lines.saturating_sub(visible_height);
        let offset = self.scroll_offset.min(max_offset);
        let indicator = offset > 0 && msg_area.height > 0;
        let content_height = if indicator {
            visible_height.saturating_sub(1)
        } else {
            visible_height
        };
        let end = total_lines.saturating_sub(offset);
        let start = end.saturating_sub(content_height);

        for (row, (x, line)) in (msg_area.top()..msg_area.bottom()).zip(all[start..end].iter()) {
            buf.set_line(*x, row, line, msg_area.width);
        }

        if indicator {
            let pct = if max_offset == 0 {
                0
            } else {
                ((offset * 100) / max_offset).min(100)
            };
            let label = format!("↑ scrolled up — {pct}% — Esc to jump to bottom");
            let y = msg_area.bottom().saturating_sub(1);
            buf.set_line(
                msg_area.x + 2,
                y,
                &Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                )),
                msg_area.width.saturating_sub(4),
            );
        }

        // ── Input bar ──
        // Fill input background
        for y in input_area.top()..input_area.bottom() {
            for x in input_area.left()..input_area.right() {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(theme::BG_PANEL));
            }
        }

        // Top border
        let border_y = input_area.top();
        for x in input_area.left()..input_area.right() {
            buf[(x, border_y)]
                .set_symbol("─")
                .set_style(Style::default().fg(theme::MUTED));
        }

        // Wrapped input rows. The composer shows the newest wrapped rows when capped,
        // so the cursor/end of a long draft remains visible above the hint row.
        let input_y = input_area.top() + 1;
        let first_visible = input_lines.len().saturating_sub(visible_input_rows);
        for (idx, line) in input_lines
            .iter()
            .skip(first_visible)
            .take(visible_input_rows)
            .enumerate()
        {
            let row = input_y + idx as u16;
            if row >= input_area.bottom().saturating_sub(1) {
                break;
            }

            let mut spans = Vec::new();
            if idx == 0 {
                let state = composer_face_state(self.stream_state, self.input);
                let (face, color) = face_frame(state, self.anim_tick);
                spans.push(Span::styled(
                    format!(
                        " {face:<width$}",
                        width = face_width.saturating_sub(1) as usize
                    ),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(" │ ", Style::default().fg(theme::MUTED)));
            } else {
                spans.push(Span::raw(" ".repeat(composer_prefix_width as usize)));
            }
            spans.extend(line.spans.iter().cloned());
            buf.set_line(input_area.x, row, &Line::from(spans), input_area.width);
        }

        let hint_y = input_area.bottom().saturating_sub(1);
        let hint_width = input_area.width.saturating_sub(4).max(1);
        if self.slash_open {
            buf.set_line(
                input_area.x + 2,
                hint_y,
                &Line::from(Span::styled(
                    "/recall  /research  /exec  /browse  /memory  /help",
                    Style::default().fg(theme::ACCENT_DIM),
                )),
                hint_width,
            );
        } else {
            buf.set_line(
                input_area.x + 2,
                hint_y,
                &Line::from(Span::styled(
                    "↵ send  / commands",
                    Style::default().fg(theme::DIM),
                )),
                hint_width,
            );
        }

        let counter = format!("{}/4096", self.input.len());
        let counter_width = counter.chars().count() as u16;
        if counter_width + 1 < input_area.width {
            let counter_x = input_area.right().saturating_sub(counter_width + 1);
            buf.set_string(counter_x, hint_y, counter, Style::default().fg(theme::DIM));
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(input: &str, cursor_on: bool) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &[],
                input,
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: false,
                cursor_on,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn render_cursor(
        input: &str,
        cursor_on: bool,
        stream_state: StreamState,
        anim_tick: u64,
    ) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &[],
                input,
                stream_state,
                scroll_offset: 0,
                slash_open: false,
                cursor_on,
                anim_tick,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn caret_painted_with_input_during_blink() {
        assert!(
            render("hello", true).contains('\u{258f}'),
            "expected composer caret when typing during blink-on"
        );
    }

    #[test]
    fn caret_hidden_on_blink_off() {
        assert!(
            !render("hello", false).contains('\u{258f}'),
            "expected no caret during blink-off half-cycle"
        );
    }

    #[test]
    fn caret_hidden_on_empty_input() {
        // Empty composer shows the placeholder, not an edit caret.
        assert!(
            !render("", true).contains('\u{258f}'),
            "expected no caret on empty composer (placeholder shown)"
        );
    }

    #[test]
    fn composer_bottom_left_face_cursor_animates_and_tracks_state() {
        let ready_tick0 = render_cursor("", false, StreamState::Idle, 0);
        let ready_tick4 = render_cursor("", false, StreamState::Idle, 4);
        assert!(
            ready_tick0.contains("(◉‿◉)"),
            "ready face missing at composer bottom-left:
{ready_tick0}"
        );
        assert!(
            ready_tick4.contains("(-‿-)"),
            "ready face should animate with app tick:
{ready_tick4}"
        );

        let listening = render_cursor("hello", false, StreamState::Idle, 2);
        assert!(
            listening.contains("(-_◉)"),
            "typing should switch the composer face to listening frames:
{listening}"
        );

        let streaming = render_cursor("", false, StreamState::Streaming, 1);
        assert!(
            streaming.contains("(◢_◣)"),
            "streaming should switch the composer face to working frames:
{streaming}"
        );
    }

    #[test]
    fn long_input_wraps_into_growing_composer_without_edge_overflow() {
        let first = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ab";
        let second = "cdefghijklmnopqrstuvwxyz";
        let input = format!("{first}{second}");
        let mut term = Terminal::new(TestBackend::new(50, 12)).unwrap();

        term.draw(|f| {
            let chat = ChatTab {
                messages: &[],
                input: &input,
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: true,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();

        let buf = term.backend().buffer().clone();
        let mut rows = Vec::new();
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            rows.push(row);
        }
        let text = rows.join("\n");

        let first_row = rows
            .iter()
            .position(|row| row.contains(first))
            .unwrap_or_else(|| panic!("first wrapped input row missing:\n{text}"));
        let second_row = rows
            .iter()
            .position(|row| row.contains(second))
            .unwrap_or_else(|| panic!("second wrapped input row missing:\n{text}"));

        assert_ne!(
            first_row, second_row,
            "long input should wrap onto multiple composer rows:\n{text}"
        );
        for row in [first_row, second_row] {
            assert_eq!(
                buf[(buf.area.width - 1, row as u16)].symbol(),
                " ",
                "input row should leave the right edge unclobbered:\n{text}"
            );
        }
        assert!(text.contains("/recall"), "slash overlay missing:\n{text}");
        assert!(text.contains("62/4096"), "length counter missing:\n{text}");
    }
}

#[cfg(test)]
mod autoscroll_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_scroll(messages: &[ChatMessage], scroll_offset: usize, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(60, h)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages,
                input: "",
                stream_state: StreamState::Idle,
                scroll_offset,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// A tall message whose body exceeds the visible height: at offset 0
    /// (follow), the NEWEST line must be on screen — the old message-granular
    /// windowing clipped it off the bottom. This is the core auto-scroll fix.
    #[test]
    fn newest_line_visible_at_offset_zero() {
        // 40 distinct body lines, viewport only ~7 tall (10 - 3 input bar).
        let body: String = (0..40).map(|i| format!("line{i}\n\n")).collect();
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            text: body,
            tool_name: None,
        }];
        let out = render_scroll(&msgs, 0, 10);
        assert!(
            out.contains("line39"),
            "newest line must be visible at offset 0 (auto-follow), got:\n{out}"
        );
        assert!(
            !out.contains("line0\n") || !out.contains("line1\n"),
            "oldest lines should be scrolled off at offset 0"
        );
    }

    /// Scrolling up (offset > 0) reveals older lines and drops the newest —
    /// manual scroll pauses the follow.
    #[test]
    fn scrolling_up_shows_resume_hint() {
        let body: String = (0..40).map(|i| format!("line{i}\n\n")).collect();
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            text: body,
            tool_name: None,
        }];
        let out = render_scroll(&msgs, 12, 10);
        assert!(
            out.contains("↑ scrolled up"),
            "scrolling up should show the resume hint, got:\n{out}"
        );
        assert!(
            out.contains("Esc to jump to bottom"),
            "resume hint should advertise Esc-to-bottom, got:\n{out}"
        );
    }

    #[test]
    fn scrolling_up_reveals_older_lines() {
        let body: String = (0..40).map(|i| format!("line{i}\n\n")).collect();
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            text: body,
            tool_name: None,
        }];
        // Scroll up 30 lines from the bottom.
        let out = render_scroll(&msgs, 30, 10);
        assert!(
            out.contains("line9") || out.contains("line8"),
            "scrolling up should reveal older lines, got:\n{out}"
        );
        assert!(
            !out.contains("line39"),
            "newest line should be off-screen when scrolled up"
        );
    }

    /// The offset is clamped: an absurd scroll value can't push content into a
    /// blank void above the first line — the oldest line stays visible.
    #[test]
    fn offset_clamped_to_top() {
        let body: String = (0..40).map(|i| format!("line{i}\n\n")).collect();
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            text: body,
            tool_name: None,
        }];
        let out = render_scroll(&msgs, 9999, 10);
        assert!(
            out.contains("line0"),
            "over-scroll must clamp to the top (oldest line visible), got:\n{out}"
        );
    }
}

#[cfg(test)]
mod markdown_bleed_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_msgs(messages: &[ChatMessage]) -> ratatui::buffer::Buffer {
        let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages,
                input: "",
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        term.backend().buffer().clone()
    }

    fn render_with_tasks(tasks: &[crate::api::TaskResponse]) -> ratatui::buffer::Buffer {
        let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &[],
                input: "",
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: Some(tasks),
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        term.backend().buffer().clone()
    }

    fn buf_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn queued_send_feedback_renders_working_indicator() {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let messages = vec![ChatMessage {
            role: Role::User,
            text: "hello".to_string(),
            tool_name: None,
        }];
        term.draw(|f| {
            let chat = ChatTab {
                messages: &messages,
                input: "",
                stream_state: StreamState::Queued,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();

        let text = buf_text(term.backend().buffer());
        assert!(
            text.contains("hello"),
            "user echo missing:
{text}"
        );
        assert!(
            text.contains("sending / working"),
            "queued send-feedback indicator missing:
{text}"
        );
    }

    #[test]
    fn task_panel_hidden_when_no_active_tasks() {
        // Empty task list → no panel header, no clutter on an idle chat.
        let buf = render_with_tasks(&[]);
        assert!(
            !buf_text(&buf).contains("tasks ("),
            "task panel must be hidden when there are no active tasks"
        );
    }

    #[test]
    fn task_panel_shows_glyphs_and_content() {
        // Mixed statuses → header count + per-row content all render.
        let tasks = vec![
            crate::api::TaskResponse {
                id: "1".into(),
                description: "Wire the widget".into(),
                status: "completed".into(),
            },
            crate::api::TaskResponse {
                id: "2".into(),
                description: "Render the panel".into(),
                status: "active".into(),
            },
            crate::api::TaskResponse {
                id: "3".into(),
                description: "Push the branch".into(),
                status: "pending".into(),
            },
        ];
        let text = buf_text(&render_with_tasks(&tasks));
        assert!(
            text.contains("tasks (1/3)"),
            "header must show done/total, got:\n{text}"
        );
        assert!(
            text.contains("Wire the widget"),
            "completed row content missing"
        );
        assert!(
            text.contains("Render the panel"),
            "active row content missing"
        );
        assert!(
            text.contains("Push the branch"),
            "pending row content missing"
        );
        assert!(text.contains('\u{2611}'), "completed glyph ☑ missing");
        assert!(text.contains('\u{25d0}'), "active glyph ◐ missing");
        assert!(text.contains('\u{2610}'), "pending glyph ☐ missing");
    }

    /// Find the first cell whose symbol starts the given needle word, return its
    /// style modifier.
    fn modifier_of(buf: &ratatui::buffer::Buffer, needle: char) -> Option<Modifier> {
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol().starts_with(needle) {
                    return Some(buf[(x, y)].style().add_modifier);
                }
            }
        }
        None
    }

    #[test]
    fn assistant_markdown_styled_user_plain() {
        // Assistant bubble with bold markdown → the bold word renders BOLD.
        let msgs = vec![
            ChatMessage {
                role: Role::User,
                text: "Wabc plain".to_string(),
                tool_name: None,
            },
            ChatMessage {
                role: Role::Assistant,
                text: "**Zbold**".to_string(),
                tool_name: None,
            },
        ];
        let buf = render_msgs(&msgs);

        // Assistant's bold word 'Z' → BOLD set (markdown applied).
        let zmod = modifier_of(&buf, 'Z').expect("assistant bold word rendered");
        assert!(
            zmod.contains(Modifier::BOLD),
            "assistant markdown bold should render BOLD, got {zmod:?}"
        );

        // User's plain word 'W' → no BOLD (plain path, markdown not applied).
        let wmod = modifier_of(&buf, 'W').expect("user plain word rendered");
        assert!(
            !wmod.contains(Modifier::BOLD),
            "user bubble must stay plain (no markdown), got {wmod:?}"
        );
    }

    #[test]
    fn p2_header_renders_context_gauge_and_model_badge() {
        let mut term = Terminal::new(TestBackend::new(96, 24)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &[],
                input: "",
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: Some("claude-opus-4-7"),
                ctx_percent: 73,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let text = buf_text(term.backend().buffer());
        assert!(text.contains("ctx"), "ctx label missing:\n{text}");
        assert!(text.contains("73%"), "ctx percent missing:\n{text}");
        assert!(
            text.contains("claude-opus-4-7"),
            "model badge missing:\n{text}"
        );
    }

    #[test]
    fn p2_assistant_renders_markdown_with_model_badge() {
        let messages = vec![ChatMessage {
            role: Role::Assistant,
            text: "**Zed** result".to_string(),
            tool_name: None,
        }];
        let mut term = Terminal::new(TestBackend::new(96, 24)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &messages,
                input: "",
                stream_state: StreamState::Idle,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: Some("sonnet-test"),
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let text = buf_text(&buf);
        assert!(
            text.contains("sonnet-test"),
            "assistant badge missing:\n{text}"
        );
        let zmod = modifier_of(&buf, 'Z').expect("assistant bold markdown rendered");
        assert!(
            zmod.contains(Modifier::BOLD),
            "assistant markdown should stay rendered, got {zmod:?}"
        );
    }

    #[test]
    fn user_assistant_and_system_messages_render_in_boxes() {
        let msgs = vec![
            ChatMessage {
                role: Role::User,
                text: "hello from user".to_string(),
                tool_name: None,
            },
            ChatMessage {
                role: Role::Assistant,
                text: "**boxed** assistant".to_string(),
                tool_name: None,
            },
            ChatMessage {
                role: Role::System,
                text: "system notice".to_string(),
                tool_name: None,
            },
        ];
        let buf = render_msgs(&msgs);
        let text = buf_text(&buf);

        for label in ["▸ you", "◈ zeus", "● sys"] {
            assert!(
                text.contains(label),
                "chat box label {label:?} missing:
{text}"
            );
        }
        assert!(
            text.matches('╭').count() >= 3,
            "each non-tool message should have a top border:
{text}"
        );
        assert!(
            text.matches('╰').count() >= 3,
            "each non-tool message should have a bottom border:
{text}"
        );

        let boxed = modifier_of(&buf, 'b').expect("assistant markdown rendered inside box");
        assert!(
            boxed.contains(Modifier::BOLD),
            "assistant markdown styling should survive boxed rendering, got {boxed:?}"
        );
    }

    #[test]
    fn p2_tool_calls_render_bordered_result_cards_not_plain_json() {
        let messages = vec![ChatMessage {
            role: Role::ToolCall,
            text: r#"{"path":"/tmp/a"} -> Line 1: ok"#.to_string(),
            tool_name: Some("read_file".to_string()),
        }];
        let mut term = Terminal::new(TestBackend::new(120, 28)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &messages,
                input: "",
                stream_state: StreamState::Streaming,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[ToolFeedItem {
                    name: "shell".to_string(),
                    input_summary: "cargo test".to_string(),
                    done: true,
                    output_summary: "ok".to_string(),
                }],
                iter_count: 1,
                active_tasks: None,
                model_badge: None,
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let text = buf_text(term.backend().buffer());
        assert!(text.contains("╭"), "tool card top border missing:\n{text}");
        assert!(
            text.contains("╰"),
            "tool card bottom border missing:\n{text}"
        );
        assert!(
            text.contains("⚙ tool_call"),
            "tool card label missing:\n{text}"
        );
        assert!(
            text.contains("read_file"),
            "persisted tool name missing:\n{text}"
        );
        assert!(text.contains("shell"), "live tool name missing:\n{text}");
        assert!(
            text.contains("✓ success"),
            "success status missing:\n{text}"
        );
        assert!(
            text.contains("result"),
            "rendered result label missing:\n{text}"
        );
    }

    #[test]
    fn user_literal_asterisks_not_consumed() {
        // A user message with markdown syntax stays literal (plain path).
        let msgs = vec![ChatMessage {
            role: Role::User,
            text: "**Qkeep**".to_string(),
            tool_name: None,
        }];
        let buf = render_msgs(&msgs);
        let mut found_star = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() == "*" {
                    found_star = true;
                }
            }
        }
        assert!(
            found_star,
            "user bubble should keep literal '*' (no markdown parsing)"
        );
    }
}
