use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::prod::draw::BufferClampExt;
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySubTab {
    Workspace,
    Sessions,
    Mnemosyne,
}

impl MemorySubTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Sessions => "Sessions",
            Self::Mnemosyne => "Mnemosyne",
        }
    }

    pub fn count_label(self, live: MemoryLive<'_>) -> String {
        let live_count = match self {
            Self::Workspace => live.files.map(<[_]>::len),
            Self::Sessions => live.sessions.map(<[_]>::len),
            Self::Mnemosyne => live.search.map(<[_]>::len),
        };
        if let Some(count) = live_count {
            let unit = match self {
                Self::Workspace => "files",
                Self::Sessions => "sessions",
                Self::Mnemosyne => "facts",
            };
            return format!("{count} {unit}");
        }

        match self {
            Self::Workspace => "847 files".to_string(),
            Self::Sessions => "147 sessions".to_string(),
            Self::Mnemosyne => "12,847 facts".to_string(),
        }
    }

    pub fn all() -> &'static [MemorySubTab] {
        &[Self::Workspace, Self::Sessions, Self::Mnemosyne]
    }
}

pub struct FileEntry {
    pub label: &'static str,
    pub color: Color,
    pub dirty: bool,
    pub current: bool,
}

pub const WORKSPACE_FILES: &[FileEntry] = &[
    FileEntry {
        label: "AGENTS.md",
        color: theme::FIRE_ORANGE,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "SOUL.md",
        color: theme::FIRE_ORANGE,
        dirty: true,
        current: false,
    },
    FileEntry {
        label: "USER.md",
        color: theme::TEXT,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "HEARTBEAT.md",
        color: theme::TEXT,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "├ journals/",
        color: theme::AMBER,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ ├ 2026-05-03.md",
        color: theme::TEXT,
        dirty: false,
        current: true,
    },
    FileEntry {
        label: "│ ├ 2026-05-02.md",
        color: theme::DIM,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ ├ 2026-05-01.md",
        color: theme::DIM,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ └ ...",
        color: theme::MUTED,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "├ projects/",
        color: theme::AMBER,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ ├ zeus-tui-onboarding.md",
        color: theme::TEXT,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ ├ pantheon-impl.md",
        color: theme::DIM,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ └ deploy-fixes.md",
        color: theme::DIM,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "├ contexts/",
        color: theme::AMBER,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "│ └ fleet-2026-05.md",
        color: theme::DIM,
        dirty: false,
        current: false,
    },
    FileEntry {
        label: "└ scratch.md",
        color: theme::DIM,
        dirty: true,
        current: false,
    },
];

pub struct SessionEntry {
    pub id: &'static str,
    pub time: &'static str,
    pub duration: &'static str,
    pub tools: u32,
    pub msgs: u32,
    pub active: bool,
    pub topic: &'static str,
}

pub const SESSIONS: &[SessionEntry] = &[
    SessionEntry {
        id: "s_2847",
        time: "14:30",
        duration: "12m",
        tools: 47,
        msgs: 23,
        active: true,
        topic: "TUI prototype design",
    },
    SessionEntry {
        id: "s_2846",
        time: "14:00",
        duration: "28m",
        tools: 89,
        msgs: 41,
        active: false,
        topic: "Onboarding impl PRD review",
    },
    SessionEntry {
        id: "s_2845",
        time: "13:15",
        duration: "45m",
        tools: 142,
        msgs: 67,
        active: false,
        topic: "Comprehensive wizard prototype",
    },
    SessionEntry {
        id: "s_2844",
        time: "11:30",
        duration: "1h 12m",
        tools: 234,
        msgs: 98,
        active: false,
        topic: "Voice / image gen PRDs",
    },
    SessionEntry {
        id: "s_2843",
        time: "yesterday 18:45",
        duration: "23m",
        tools: 56,
        msgs: 34,
        active: false,
        topic: "Fleet shakedown audit",
    },
    SessionEntry {
        id: "s_2842",
        time: "yesterday 16:20",
        duration: "55m",
        tools: 178,
        msgs: 72,
        active: false,
        topic: "Pitch deck v5",
    },
];

pub struct SearchResult {
    pub text: &'static str,
    pub score: f64,
    pub age: &'static str,
    pub source: &'static str,
}

pub const SEARCH_RESULTS: &[SearchResult] = &[
    SearchResult { text: "Mike confirmed Track C blockers ship in Phase 0 — [talos] always-write, [images]→[talos.image], heartbeat persistence", score: 0.94, age: "8m ago", source: "session 2847" },
    SearchResult { text: "ChanConfig forms must be stacked (all visible) not sequential per merakizzz directive 2026-05-03", score: 0.91, age: "30m ago", source: "session 2846" },
    SearchResult { text: "Z-Image Turbo on DGX requires steps=1 — multi-step inference returns black PNG", score: 0.88, age: "1h ago", source: "session 2845" },
    SearchResult { text: "Mac Studio M5 Ultra release tracked — 256GB RAM target for AI inference workloads", score: 0.85, age: "yesterday", source: "session 2843" },
];

#[derive(Default, Clone, Copy)]
pub struct MemoryLive<'a> {
    pub files: Option<&'a [crate::api::MemoryFileEntry]>,
    pub sessions: Option<&'a [crate::api::SessionSummary]>,
    pub search: Option<&'a [crate::api::MemorySearchHit]>,
}

pub fn render_memory_tab(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    active_sub: MemorySubTab,
    scroll: u16,
    live: MemoryLive<'_>,
) {
    if area.is_empty() {
        return;
    }

    Clear.render(area, buf);
    fill_rect(buf, area, Style::default().bg(theme::BG));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    render_sub_tab_bar(chunks[0], buf, active_sub, live);

    match active_sub {
        MemorySubTab::Workspace => render_workspace(chunks[1], buf, scroll, live.files),
        MemorySubTab::Sessions => render_sessions(chunks[1], buf, scroll, live.sessions),
        MemorySubTab::Mnemosyne => render_mnemosyne(chunks[1], buf, scroll, live.search),
    }
}

fn render_sub_tab_bar(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    active: MemorySubTab,
    live: MemoryLive<'_>,
) {
    fill_rect(buf, area, Style::default().bg(theme::BG_PANEL));
    let mut x = area.x + 1;
    let label_y = area.y + 1.min(area.height.saturating_sub(1));
    let underline_y = area.bottom().saturating_sub(1);

    for tab in MemorySubTab::all() {
        let label = tab.label();
        let count = tab.count_label(live);
        let active_tab = *tab == active;
        let start = x;
        let label_style = if active_tab {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        let tab_text = format!("{label} ");
        buf.set_string_clamped(x, label_y, tab_text, label_style);
        x = x.saturating_add(label.len() as u16 + 1);
        buf.set_string_clamped(x, label_y, &count, Style::default().fg(theme::MUTED));
        x = x.saturating_add(count.len() as u16 + 4);

        let end = x.saturating_sub(3).min(area.right());
        if active_tab {
            for ux in start..end {
                buf.set_string_clamped(
                    ux,
                    underline_y,
                    "─",
                    Style::default().fg(theme::FIRE_ORANGE),
                );
            }
        }
    }

    for bx in area.x..area.right() {
        if buf[(bx, underline_y)].symbol() == " " {
            buf.set_string_clamped(bx, underline_y, "─", Style::default().fg(theme::MUTED));
        }
    }
}

fn render_workspace(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    scroll: u16,
    live: Option<&[crate::api::MemoryFileEntry]>,
) {
    let left_width = area.width.min(42);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(0)])
        .split(area);

    render_workspace_tree(chunks[0], buf, scroll, live);
    render_journal(chunks[1], buf);
}

fn render_workspace_tree(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    scroll: u16,
    live: Option<&[crate::api::MemoryFileEntry]>,
) {
    fill_rect(buf, area, Style::default().bg(theme::BG));
    draw_right_border(area, buf);
    let mut y = area.y + 1;

    buf.set_string_clamped(
        area.x + 2,
        y,
        "~/.zeus/workspace/",
        Style::default().fg(theme::DIM),
    );
    y += 2;

    if let Some(files) = live {
        for (i, file) in files.iter().enumerate().skip(scroll as usize) {
            if y >= area.bottom() {
                break;
            }
            let is_dir = file.path.ends_with('/');
            let current = i == 0;
            let style = file_style(if is_dir { theme::AMBER } else { theme::TEXT }, current);
            let marker = if current { " ◀" } else { "" };
            let row = format!("  ├ {}{}", file.path, marker);
            paint_current_row(area, buf, y, current);
            buf.set_string_clamped(area.x + 2, y, row, style);
            y += 1;
        }
        return;
    }

    for file in WORKSPACE_FILES.iter().skip(scroll as usize) {
        if y >= area.bottom() {
            break;
        }
        let dirty = if file.dirty { " ●" } else { "" };
        let marker = if file.current { " ◀" } else { "" };
        let row = format!("  {}{}{}", file.label, dirty, marker);
        paint_current_row(area, buf, y, file.current);
        buf.set_string_clamped(area.x + 2, y, row, file_style(file.color, file.current));
        y += 1;
    }
}

fn render_journal(area: Rect, buf: &mut ratatui::buffer::Buffer) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    fill_rect(buf, area, Style::default().bg(theme::BG));
    let header = Rect::new(area.x, area.y, area.width, area.height.min(3));
    fill_rect(buf, header, Style::default().bg(theme::BG_PANEL));
    draw_bottom_border(header, buf);

    let mut x = area.x + 2;
    let y = area.y + 1;
    buf.set_string_clamped(
        x,
        y,
        "JOURNAL",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    x += 10;
    buf.set_string_clamped(x, y, "2026-05-03.md", Style::default().fg(theme::DIM));
    let meta = "last modified · 2 minutes ago";
    let meta_x = area.right().saturating_sub(meta.len() as u16 + 2);
    if meta_x > x + 16 {
        buf.set_string_clamped(meta_x, y, meta, Style::default().fg(theme::MUTED));
    }

    let mut cy = area.y + 4;
    put_line(
        area,
        buf,
        &mut cy,
        "# Journal · 2026-05-03",
        theme::FIRE_ORANGE,
        true,
    );
    put_line(
        area,
        buf,
        &mut cy,
        "Saturday · zeus.local",
        theme::DIM,
        false,
    );
    cy += 1;
    put_line(area, buf, &mut cy, "## Sessions", theme::AMBER, true);
    put_wrapped(area, buf, &mut cy, "Worked through the comprehensive onboarding wizard impl PRD with merakizzz. Walked all 19 steps. Locked the feature surface. Track C (Talos gate, [images] migration, heartbeat persistence) confirmed as pre-launch blockers.", theme::TEXT);
    cy += 1;
    put_line(area, buf, &mut cy, "## Decisions", theme::AMBER, true);
    put_wrapped(
        area,
        buf,
        &mut cy,
        "- Image gen routes to [talos.image], not [images]",
        theme::TEXT,
    );
    put_wrapped(
        area,
        buf,
        &mut cy,
        "- ChanConfig forms render stacked, not sequential",
        theme::TEXT,
    );
    put_wrapped(
        area,
        buf,
        &mut cy,
        "- Memory step pre-selects Ollama if detected at localhost:11434",
        theme::TEXT,
    );
    cy += 1;
    put_line(area, buf, &mut cy, "## Next", theme::AMBER, true);
    put_wrapped(area, buf, &mut cy, "- Ship Phase 0 blockers", theme::TEXT);
    put_wrapped(
        area,
        buf,
        &mut cy,
        "- Render-gate every prod tab against the prototype",
        theme::TEXT,
    );
}

fn render_sessions(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    scroll: u16,
    live: Option<&[crate::api::SessionSummary]>,
) {
    fill_rect(buf, area, Style::default().bg(theme::BG));
    let mut y = area.y + 1;

    if let Some(sessions) = live {
        for (i, session) in sessions.iter().enumerate().skip(scroll as usize) {
            if y >= area.bottom() {
                break;
            }
            let id: String = session.id.chars().take(8).collect();
            let stats = format!(
                "~{} tok · {} msgs",
                session.est_tokens, session.message_count
            );
            render_session_row(
                area,
                buf,
                y,
                SessionRow {
                    active: i == 0,
                    id: &id,
                    time: &session.created,
                    topic: &session.last_preview,
                    stats: &stats,
                },
            );
            y += 2;
        }
        return;
    }

    for session in SESSIONS.iter().skip(scroll as usize) {
        if y >= area.bottom() {
            break;
        }
        let stats = format!(
            "{} · {} tools · {} msgs",
            session.duration, session.tools, session.msgs
        );
        render_session_row(
            area,
            buf,
            y,
            SessionRow {
                active: session.active,
                id: session.id,
                time: session.time,
                topic: session.topic,
                stats: &stats,
            },
        );
        y += 2;
    }
}

fn render_mnemosyne(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    scroll: u16,
    live: Option<&[crate::api::MemorySearchHit]>,
) {
    fill_rect(buf, area, Style::default().bg(theme::BG));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    fill_rect(buf, chunks[0], Style::default().bg(theme::BG_PANEL));
    draw_bottom_border(chunks[0], buf);
    let prompt = "/  hybrid search · BM25 + vector embeddings";
    buf.set_string_clamped(
        chunks[0].x + 2,
        chunks[0].y + 1,
        prompt,
        Style::default().fg(theme::DIM),
    );
    let embedded = "● ollama embedded";
    let ex = chunks[0].right().saturating_sub(embedded.len() as u16 + 2);
    buf.set_string_clamped(
        ex,
        chunks[0].y + 1,
        embedded,
        Style::default().fg(theme::CYAN),
    );

    let count = live.map_or("12,847".to_string(), |hits| hits.len().to_string());
    let title = format!("RECENT FACTS · {count} indexed");
    let mut y = chunks[1].y + 1;
    buf.set_string_clamped(
        chunks[1].x + 2,
        y,
        title,
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 2;

    if let Some(hits) = live {
        for hit in hits.iter().skip(scroll as usize) {
            if y + 3 >= chunks[1].bottom() {
                break;
            }
            let source = hit
                .memory_type
                .as_deref()
                .or(hit.path.as_deref())
                .or(hit.session_id.as_deref())
                .unwrap_or("memory");
            let meta = format!("{:.2} · {source} · live", hit.score);
            render_fact_card(chunks[1], buf, y, &meta, &hit.content);
            y += 5;
        }
        return;
    }

    for result in SEARCH_RESULTS.iter().skip(scroll as usize) {
        if y + 3 >= chunks[1].bottom() {
            break;
        }
        let meta = format!("{:.2} · {} · {}", result.score, result.source, result.age);
        render_fact_card(chunks[1], buf, y, &meta, result.text);
        y += 5;
    }
}

struct SessionRow<'a> {
    active: bool,
    id: &'a str,
    time: &'a str,
    topic: &'a str,
    stats: &'a str,
}

fn render_session_row(area: Rect, buf: &mut ratatui::buffer::Buffer, y: u16, row: SessionRow<'_>) {
    let dot = if row.active { theme::GREEN } else { theme::DIM };
    buf.set_string_clamped(area.x + 2, y, "●", Style::default().fg(dot));
    buf.set_string_clamped(area.x + 5, y, row.id, Style::default().fg(theme::DIM));
    buf.set_string_clamped(area.x + 15, y, row.time, Style::default().fg(theme::MUTED));
    buf.set_string_clamped(area.x + 30, y, row.topic, Style::default().fg(theme::TEXT));
    let stats_x = area.right().saturating_sub(row.stats.len() as u16 + 2);
    buf.set_string_clamped(stats_x, y, row.stats, Style::default().fg(theme::DIM));
    draw_hline(
        buf,
        area.x,
        y + 1,
        area.width,
        Style::default().fg(theme::MUTED),
    );
}

fn render_fact_card(area: Rect, buf: &mut ratatui::buffer::Buffer, y: u16, meta: &str, text: &str) {
    let card = Rect::new(area.x + 2, y, area.width.saturating_sub(4), 4);
    if card.width < 8 || card.height < 3 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::MUTED))
        .style(Style::default().bg(theme::BG_PANEL));
    let inner = block.inner(card);
    block.render(card, buf);
    for by in card.y..card.bottom() {
        buf[(card.x, by)].set_style(Style::default().fg(theme::CYAN).bg(theme::BG_PANEL));
    }
    buf.set_string_clamped(
        inner.x + 1,
        inner.y,
        meta,
        Style::default().fg(theme::GREEN),
    );
    let max = inner.width.saturating_sub(2) as usize;
    let line: String = text.chars().take(max).collect();
    buf.set_string_clamped(
        inner.x + 1,
        inner.y + 1,
        line,
        Style::default().fg(theme::TEXT),
    );
}

fn file_style(color: Color, current: bool) -> Style {
    let style = Style::default().fg(color);
    if current {
        style.bg(theme::BG_HIGHLIGHT).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn paint_current_row(area: Rect, buf: &mut ratatui::buffer::Buffer, y: u16, current: bool) {
    if current {
        for x in area.x..area.right().saturating_sub(1) {
            buf[(x, y)].set_style(Style::default().bg(theme::BG_HIGHLIGHT));
        }
        buf.set_string_clamped(
            area.x,
            y,
            "│",
            Style::default()
                .fg(theme::FIRE_ORANGE)
                .bg(theme::BG_HIGHLIGHT),
        );
    }
}

fn put_line(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    y: &mut u16,
    text: &str,
    color: Color,
    bold: bool,
) {
    if *y >= area.bottom() {
        return;
    }
    let style = if bold {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    buf.set_string_clamped(area.x + 2, *y, text, style);
    *y += 1;
}

fn put_wrapped(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    y: &mut u16,
    text: &str,
    color: Color,
) {
    let width = area.width.saturating_sub(4) as usize;
    if width == 0 {
        return;
    }
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + word.len() + 1 > width {
            put_line(area, buf, y, &line, color, false);
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        put_line(area, buf, y, &line, color, false);
    }
}

fn fill_rect(buf: &mut ratatui::buffer::Buffer, area: Rect, style: Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf[(x, y)].set_style(style);
        }
    }
}

fn draw_right_border(area: Rect, buf: &mut ratatui::buffer::Buffer) {
    if area.width == 0 {
        return;
    }
    let x = area.right().saturating_sub(1);
    for y in area.y..area.bottom() {
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::MUTED));
    }
}

fn draw_bottom_border(area: Rect, buf: &mut ratatui::buffer::Buffer) {
    if area.height == 0 {
        return;
    }
    draw_hline(
        buf,
        area.x,
        area.bottom().saturating_sub(1),
        area.width,
        Style::default().fg(theme::MUTED),
    );
}

fn draw_hline(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, width: u16, style: Style) {
    for dx in 0..width {
        buf.set_string_clamped(x + dx, y, "─", style);
    }
}
