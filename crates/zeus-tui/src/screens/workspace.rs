use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Clear, Block, Borders, Padding, Widget};

use crate::theme;

/// Truncate `text` to `max_w` chars, appending `…` if clipped. Char-based
/// (paths may contain non-ASCII). Same idiom as the other #271 screens.
fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if text.chars().count() <= max_w {
        return text.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let keep: String = text.chars().take(max_w - 1).collect();
    format!("{keep}…")
}

/// Workspace screen — step 11 of onboarding.
/// Matches JSX WorkspaceStep component (line 1309).
pub struct WorkspaceScreen {
    /// Workspace path value
    pub workspace_path: String,
    /// Sessions path value
    pub sessions_path: String,
    /// Mnemosyne DB path value
    pub mnemosyne_path: String,
    /// Whether an existing workspace was detected
    pub existing_detected: bool,
    /// Number of memory facts found (if existing)
    pub memory_facts: usize,
    /// Number of sessions found (if existing)
    pub session_count: usize,
    /// Human-readable last-modified time of the existing workspace (static
    /// example for now; FOLLOW-UP: derive from real dir mtime scan).
    pub existing_mtime: String,
    /// Which field is focused: 0=workspace, 1=sessions, 2=mnemosyne
    pub focused_field: usize,
    /// Blink phase from `App::cursor_visible()` — drives the insertion cursor
    /// on the focused field (set by the caller each frame).
    pub cursor_on: bool,
}

impl Default for WorkspaceScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceScreen {
    pub fn new() -> Self {
        Self {
            workspace_path: "~/.zeus/workspace".to_string(),
            sessions_path: "~/.zeus/sessions".to_string(),
            mnemosyne_path: "~/.zeus/mnemosyne.db".to_string(),
            existing_detected: false,
            memory_facts: 0,
            session_count: 0,
            existing_mtime: "2 minutes ago".to_string(),
            focused_field: 0,
            cursor_on: false,
        }
    }
}

impl Widget for WorkspaceScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height < 5 || area.width < 20 {
            return;
        }

        let inner = Block::default()
            .padding(Padding::horizontal(2))
            .inner(area);

        let mut cy = inner.y;

        // Existing workspace warning (if detected)
        if self.existing_detected {
            let warn_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::AMBER))
                .style(Style::default().fg(theme::AMBER));

            let warn_area = Rect {
                x: inner.x,
                y: cy,
                width: inner.width,
                height: 4,
            };
            warn_block.render(warn_area, buf);

            // Header: "↻ EXISTING WORKSPACE FOUND" (amber, bold) — JSX 1319
            let hdr_w = inner.width.saturating_sub(4) as usize;
            let hdr = clamp_ellipsis("↻ EXISTING WORKSPACE FOUND", hdr_w);
            buf.set_string(
                inner.x + 2,
                cy + 1,
                &hdr,
                Style::default().fg(theme::AMBER).add_modifier(ratatui::style::Modifier::BOLD),
            );

            // Body: "~/.zeus/workspace contains {N} memory facts, {N} sessions,
            // last modified {time}." — JSX 1320. Counts/time are a STATIC example
            // here per dispatch; FOLLOW-UP: real dir-scan for fact/session counts
            // + mtime (spec calls for live scan, deferred to avoid blocking 1:1).
            let info = format!(
                "~/.zeus/workspace contains {} memory facts, {} sessions, last modified {}",
                self.memory_facts, self.session_count, self.existing_mtime
            );
            let info_w = inner.width.saturating_sub(4) as usize;
            let info_clamped = clamp_ellipsis(&info, info_w);
            buf.set_string(inner.x + 2, cy + 2, &info_clamped, Style::default().fg(theme::TEXT));

            // Buttons: USE EXISTING (accent fill) · START FRESH (BACKUP OLD) (outline) — JSX 1322-1325
            let use_label = " USE EXISTING ";
            buf.set_string(
                inner.x + 2,
                cy + 3,
                use_label,
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::ACCENT)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );
            let fresh_label = " START FRESH (BACKUP OLD) ";
            let fresh_x = inner.x + 2 + use_label.chars().count() as u16 + 2;
            if fresh_x + fresh_label.chars().count() as u16 <= inner.x + inner.width {
                buf.set_string(
                    fresh_x,
                    cy + 3,
                    fresh_label,
                    Style::default().fg(theme::DIM).add_modifier(ratatui::style::Modifier::BOLD),
                );
            }

            cy += 6;
        }

        // PATHS section label — JSX 1329 (accentDim, bold, letter-spaced)
        buf.set_string(
            inner.x,
            cy,
            "PATHS",
            Style::default().fg(theme::ACCENT_DIM).add_modifier(ratatui::style::Modifier::BOLD),
        );
        cy += 1;

        // Workspace path field
        let ws_label = if self.focused_field == 0 {
            "▸ Workspace"
        } else {
            "  Workspace"
        };
        let ws_color = if self.focused_field == 0 {
            theme::FIRE_ORANGE
        } else {
            theme::DIM
        };
        buf.set_string(
            inner.x,
            cy,
            ws_label,
            Style::default().fg(ws_color).add_modifier(ratatui::style::Modifier::BOLD),
        );
        cy += 1;

        // Workspace path value box
        let ws_box = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.focused_field == 0 {
                theme::FIRE_ORANGE
            } else {
                theme::BORDER
            }))
            .style(Style::default().fg(theme::TEXT));

        let ws_area = Rect {
            x: inner.x,
            y: cy,
            width: inner.width.min(60),
            height: 3,
        };
        ws_box.render(ws_area, buf);
        let ws_text_w = ws_area.width.saturating_sub(4) as usize; // 2 border + 2 padding
        let ws_clamped = clamp_ellipsis(&self.workspace_path, ws_text_w);
        buf.set_string(inner.x + 2, cy + 1, &ws_clamped, Style::default().fg(theme::TEXT));
        // Insertion cursor — focused field only, blink-gated. Char-count (not
        // byte) so a non-ASCII path can't panic; clamped inside the box border.
        if self.cursor_on && self.focused_field == 0 && !ws_clamped.is_empty() {
            let cursor_col = inner.x + 2 + ws_clamped.chars().count() as u16;
            if cursor_col < inner.x + ws_area.width - 1 {
                buf.set_string(cursor_col, cy + 1, "▏", Style::default().fg(theme::AMBER));
            }
        }

        // Hint — only render if there's room to the right of the box
        let hint_x = inner.x + ws_area.width + 2;
        if hint_x < inner.x + inner.width {
            let hint_w = (inner.x + inner.width - hint_x) as usize;
            let hint = clamp_ellipsis("AGENTS.md, SOUL.md, journals, daily notes", hint_w);
            buf.set_string(hint_x, cy + 1, &hint, Style::default().fg(theme::MUTED));
        }
        cy += 4;

        // Sessions path field
        let sess_label = if self.focused_field == 1 {
            "▸ Sessions"
        } else {
            "  Sessions"
        };
        let sess_color = if self.focused_field == 1 {
            theme::FIRE_ORANGE
        } else {
            theme::DIM
        };
        buf.set_string(
            inner.x,
            cy,
            sess_label,
            Style::default().fg(sess_color).add_modifier(ratatui::style::Modifier::BOLD),
        );
        cy += 1;

        // Sessions path value box
        let sess_box = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.focused_field == 1 {
                theme::FIRE_ORANGE
            } else {
                theme::BORDER
            }))
            .style(Style::default().fg(theme::TEXT));

        let sess_area = Rect {
            x: inner.x,
            y: cy,
            width: inner.width.min(60),
            height: 3,
        };
        sess_box.render(sess_area, buf);
        let sess_text_w = sess_area.width.saturating_sub(4) as usize;
        let sess_clamped = clamp_ellipsis(&self.sessions_path, sess_text_w);
        buf.set_string(inner.x + 2, cy + 1, &sess_clamped, Style::default().fg(theme::TEXT));
        if self.cursor_on && self.focused_field == 1 && !sess_clamped.is_empty() {
            let cursor_col = inner.x + 2 + sess_clamped.chars().count() as u16;
            if cursor_col < inner.x + sess_area.width - 1 {
                buf.set_string(cursor_col, cy + 1, "▏", Style::default().fg(theme::AMBER));
            }
        }

        // Hint — only render if there's room to the right of the box
        let hint_x = inner.x + sess_area.width + 2;
        if hint_x < inner.x + inner.width {
            let hint_w = (inner.x + inner.width - hint_x) as usize;
            let hint = clamp_ellipsis(
                "Per-conversation JSONL logs (grows ~5MB/day per active agent)",
                hint_w,
            );
            buf.set_string(hint_x, cy + 1, &hint, Style::default().fg(theme::MUTED));
        }
        cy += 4;

        // Mnemosyne DB field
        let mn_label = if self.focused_field == 2 {
            "▸ Mnemosyne DB"
        } else {
            "  Mnemosyne DB"
        };
        let _mn_color = if self.focused_field == 2 {
            theme::ACCENT
        } else {
            theme::DIM
        };

        let mn_box = Block::default()
            .title(format!(" {} ", mn_label))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.focused_field == 2 {
                theme::ACCENT
            } else {
                theme::BORDER
            }))
            .style(Style::default().fg(theme::TEXT));

        let mn_area = Rect {
            x: inner.x,
            y: cy,
            width: inner.width.min(60),
            height: 3,
        };
        mn_box.render(mn_area, buf);
        let mn_text_w = mn_area.width.saturating_sub(4) as usize;
        let mn_clamped = clamp_ellipsis(&self.mnemosyne_path, mn_text_w);
        buf.set_string(inner.x + 2, cy + 1, &mn_clamped, Style::default().fg(theme::TEXT));
        if self.cursor_on && self.focused_field == 2 && !mn_clamped.is_empty() {
            let cursor_col = inner.x + 2 + mn_clamped.chars().count() as u16;
            if cursor_col < inner.x + mn_area.width - 1 {
                buf.set_string(cursor_col, cy + 1, "▏", Style::default().fg(theme::AMBER));
            }
        }

        // Hint — only render if there's room to the right of the box
        let hint_x = inner.x + mn_area.width + 2;
        if hint_x < inner.x + inner.width {
            let hint_w = (inner.x + inner.width - hint_x) as usize;
            let hint = clamp_ellipsis("SQLite + vector embeddings (can grow to GBs)", hint_w);
            buf.set_string(hint_x, cy + 1, &hint, Style::default().fg(theme::MUTED));
        }
        cy += 4;

        // Disk usage projection — JSX 1334: bordered (1px muted) box on bg2,
        // header + 3-col grid (label dim / value accent / sub muted).
        let disk_area = Rect {
            x: inner.x,
            y: cy,
            width: inner.width,
            height: 6,
        };
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .style(Style::default().bg(theme::BG_PANEL))
            .render(disk_area, buf);

        buf.set_string(
            inner.x + 2,
            cy + 1,
            "DISK USAGE PROJECTION",
            Style::default().fg(theme::ACCENT_DIM).add_modifier(ratatui::style::Modifier::BOLD),
        );

        let projections = [
            ("Workspace", "~50 MB", "after 30 days"),
            ("Sessions", "~150 MB", "@ 5 MB/day for 30d"),
            ("Mnemosyne", "~800 MB", "after 1000 sessions"),
        ];

        // 3-col grid inside the box (account for the 2-col border + padding).
        let grid_x = inner.x + 2;
        let grid_w = inner.width.saturating_sub(4);
        let col_width = grid_w / 3;
        let cell_w = col_width.saturating_sub(1) as usize; // 1-col gutter
        for (i, (label, value, sub)) in projections.iter().enumerate() {
            let x = grid_x + (i as u16 * col_width);

            buf.set_string(x, cy + 2, clamp_ellipsis(label, cell_w), Style::default().fg(theme::DIM));
            buf.set_string(
                x,
                cy + 3,
                clamp_ellipsis(value, cell_w),
                Style::default().fg(theme::ACCENT).add_modifier(ratatui::style::Modifier::BOLD),
            );
            buf.set_string(x, cy + 4, clamp_ellipsis(sub, cell_w), Style::default().fg(theme::MUTED));
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render(focused: usize, cursor_on: bool) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let mut s = WorkspaceScreen::new();
            s.focused_field = focused;
            s.cursor_on = cursor_on;
            s.render(f.area(), f.buffer_mut());
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
    fn cursor_painted_on_focused_field_during_blink() {
        // Focused + blink-on + non-empty path → insertion glyph present.
        assert!(
            render(0, true).contains('▏'),
            "expected cursor `▏` on focused field during blink-on"
        );
    }

    #[test]
    fn cursor_hidden_on_blink_off() {
        assert!(
            !render(0, false).contains('▏'),
            "expected no cursor `▏` during blink-off half-cycle"
        );
    }

    #[test]
    fn cursor_follows_focus() {
        // Each focus index paints exactly one cursor on its own field. The
        // default paths are all non-empty, so blink-on always yields one glyph.
        for f in 0..3 {
            assert!(
                render(f, true).contains('▏'),
                "expected cursor on focused field {f}"
            );
        }
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// Render at a given width with long paths that would overflow without
    /// `clamp_ellipsis`. Returns the full buffer as a string.
    fn render_at(width: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, 40)).unwrap();
        term.draw(|f| {
            let mut s = WorkspaceScreen::new();
            s.workspace_path =
                "/very/long/workspace/path/that/exceeds/the/box/width/and/would/overflow".to_string();
            s.sessions_path =
                "/very/long/sessions/path/that/exceeds/the/box/width/and/would/overflow".to_string();
            s.mnemosyne_path =
                "/very/long/mnemosyne/path/that/exceeds/the/box/width/and/would/overflow".to_string();
            s.render(f.area(), f.buffer_mut());
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

    /// At narrow width (~56), long paths must be truncated with `…` — no
    /// raw overflow past the box border. This is load-bearing: removing
    /// `clamp_ellipsis` from the path renders makes this FAIL because the
    /// full path string spills past the box border into adjacent cells.
    #[test]
    fn narrow_paths_truncated_no_overflow() {
        let out = render_at(56);
        // The full un-truncated path must NOT appear — it's longer than the
        // box. If it does, clamp_ellipsis was removed/bypassed.
        assert!(
            !out.contains("/very/long/workspace/path/that/exceeds/the/box/width/and/would/overflow"),
            "workspace path not truncated at narrow width — clamp_ellipsis missing?"
        );
        assert!(
            !out.contains("/very/long/sessions/path/that/exceeds/the/box/width/and/would/overflow"),
            "sessions path not truncated at narrow width — clamp_ellipsis missing?"
        );
        assert!(
            !out.contains("/very/long/mnemosyne/path/that/exceeds/the/box/width/and/would/overflow"),
            "mnemosyne path not truncated at narrow width — clamp_ellipsis missing?"
        );
        // The ellipsis marker must be present (truncation happened).
        assert!(out.contains('…'), "expected `…` truncation marker at narrow width");
    }

    /// At normal width (~100), the paths still exceed the box (min(60)), so
    /// truncation must still apply. The ellipsis must be present.
    #[test]
    fn normal_paths_truncated_in_box() {
        let out = render_at(100);
        assert!(
            !out.contains("/very/long/workspace/path/that/exceeds/the/box/width/and/would/overflow"),
            "workspace path not truncated at normal width — clamp_ellipsis missing?"
        );
        assert!(out.contains('…'), "expected `…` truncation marker at normal width");
    }

    /// Hints must not render when there's no room to the right of the box.
    /// At width 56, the box (min(60) → clamped to ~52 inner) leaves no room
    /// for the hint text. The full hint string must NOT appear.
    #[test]
    fn narrow_hints_suppressed_when_no_room() {
        let out = render_at(56);
        // The full hint strings are long; if they rendered they'd overflow.
        // At narrow width the hint area is 0 or negative — suppressed.
        assert!(
            !out.contains("Per-conversation JSONL logs (grows ~5MB/day per active agent)"),
            "sessions hint rendered at narrow width with no room — bounds check missing?"
        );
    }

    /// Disk projection grid cells must be truncated at narrow width so
    /// columns don't overlap. The full "after 1000 sessions" sub must not
    /// appear un-truncated when the column is too narrow.
    #[test]
    fn narrow_disk_grid_cells_truncated() {
        let out = render_at(56);
        // At 56 cols, inner ~52, grid_w ~48, col_width ~16. "after 1000 sessions"
        // is 20 chars — must be truncated.
        assert!(
            !out.contains("after 1000 sessions"),
            "disk grid sub not truncated at narrow width — clamp_ellipsis missing?"
        );
    }

    /// clamp_ellipsis unit tests (same idiom as other #271 screens).
    #[test]
    fn clamp_ellipsis_semantics() {
        assert_eq!(clamp_ellipsis("short", 10), "short");
        assert_eq!(clamp_ellipsis("exactfit12", 10), "exactfit12");
        assert_eq!(clamp_ellipsis("toolongname", 6), "toolo…");
        assert_eq!(clamp_ellipsis("anything", 1), "…");
        assert_eq!(clamp_ellipsis("anything", 0), "");
    }
}
