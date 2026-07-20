use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Paragraph, Widget};

use crate::theme;

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

fn inset(area: Rect, dx: u16, dy: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(dx),
        area.y.saturating_add(dy),
        area.width.saturating_sub(dx.saturating_mul(2)),
        area.height.saturating_sub(dy.saturating_mul(2)),
    )
}

fn write_line(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, line: &Line<'_>, width: u16) {
    if width == 0 {
        return;
    }
    buf.set_line(x, y, line, width);
}

/// Security level entry — matches JSX SECURITY_LEVELS (line 133).
struct SecurityLevel {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
    blocked: &'static [&'static str],
    #[allow(dead_code)] // staged UI scaffolding
    allowed: &'static [&'static str],
    recommended: bool,
}

/// All 5 real `zeus_aegis::sandbox::SandboxLevel` values, in the enum's own
/// ascending-strictness order (None < Basic < Standard < Strict < Paranoid —
/// see zeus-aegis/src/sandbox.rs). #401: the prior 4-entry list here
/// (strict/standard/permissive/custom) was a UI-only vocabulary that could
/// never reach "basic" or "paranoid", and "custom" had no real aegis mapping
/// at all — `config_level_value()` silently fell back to "standard" for it.
const SECURITY_LEVELS: &[SecurityLevel] = &[
    SecurityLevel {
        id: "none",
        name: "None",
        glyph: "NON",
        color: theme::DIM,
        sub: "Development only — no sandboxing",
        blocked: &[],
        allowed: &["everything", "approval pipeline still active"],
        recommended: false,
    },
    SecurityLevel {
        id: "basic",
        name: "Basic",
        glyph: "BSC",
        color: theme::YELLOW,
        sub: "Block dangerous operations",
        blocked: &["known-destructive shell patterns"],
        allowed: &["all read/write", "shell (filtered)", "web_fetch", "apply_patch"],
        recommended: false,
    },
    SecurityLevel {
        id: "standard",
        name: "Standard",
        glyph: "STD",
        color: theme::AMBER,
        sub: "Personal coding assistant",
        blocked: &["shell with sudo", "fs_write outside workspace + home"],
        allowed: &["all read", "shell (filtered)", "web_fetch (allowlisted)", "apply_patch"],
        recommended: true,
    },
    SecurityLevel {
        id: "strict",
        name: "Strict",
        glyph: "STR",
        color: theme::RED,
        sub: "Shared-machine fleet bots",
        blocked: &["shell", "web_fetch", "apply_patch", "fs_write outside workspace"],
        allowed: &["fs_read in workspace", "memory ops", "channel send"],
        recommended: false,
    },
    SecurityLevel {
        id: "paranoid",
        name: "Paranoid",
        glyph: "PAR",
        color: theme::RED,
        sub: "Minimal permissions — network allowlist only",
        blocked: &["shell", "web_fetch", "apply_patch", "fs_write anywhere"],
        allowed: &["fs_read in workspace", "memory ops"],
        recommended: false,
    },
];

/// Security screen state — step 11, id: "security", code: SECR.
pub struct SecurityScreen {
    /// Index of the currently selected security level.
    selected: usize,
    /// #401: true once the user has actually interacted with this screen
    /// (moved the selection) OR the screen was hydrated from a real existing
    /// `cfg.aegis.sandbox_level` on disk. `collect_and_persist()` gates its
    /// `[aegis]` write on this flag so that a no-op press-through on a config
    /// with no `[aegis]` section (or one the user never touched) does not
    /// fabricate a new section out of the wizard's cosmetic default.
    touched: bool,
}

impl Default for SecurityScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityScreen {
    pub fn new() -> Self {
        // Default to "standard" (recommended: true) — index 2 in the 5-level
        // ordering (none, basic, standard, strict, paranoid).
        Self { selected: 2, touched: false }
    }

    /// Move selection up (left in the grid).
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.touched = true;
    }

    /// Move selection down (right in the grid).
    pub fn select_next(&mut self) {
        if self.selected < SECURITY_LEVELS.len() - 1 {
            self.selected += 1;
        }
        self.touched = true;
    }

    /// Get the currently selected level id (UI identity).
    pub fn selected_id(&self) -> &'static str {
        SECURITY_LEVELS[self.selected].id
    }

    /// The UI level id IS the aegis `SandboxLevel` value — identity passthrough.
    /// (none/basic/standard/strict/paranoid — see zeus-aegis/src/sandbox.rs
    /// FromStr.) #401: this used to be a lossy 4→3 remap for a UI vocabulary
    /// that didn't match the enum; now `SECURITY_LEVELS` IS the enum's 5
    /// values so no translation is needed.
    pub fn config_level_value(&self) -> &'static str {
        self.selected_id()
    }

    /// Whether the screen's current selection should be persisted — either
    /// the user explicitly moved it, or it was hydrated from a real existing
    /// `cfg.aegis.sandbox_level` on disk (see `hydrate_from_id`).
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// Pre-select the card matching an existing on-disk `sandbox_level`
    /// string, if it names one of the 5 real levels. Marks the screen
    /// touched so `collect_and_persist()` preserves (rather than silently
    /// overwrites) the operator's real prior choice on a no-op press-through.
    pub fn hydrate_from_id(&mut self, id: &str) {
        if let Some(idx) = SECURITY_LEVELS.iter().position(|l| l.id == id) {
            self.selected = idx;
            self.touched = true;
        }
    }

    /// Render the security screen into the given area.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        // Opaque background
        Block::default().style(Style::default().bg(theme::BG)).render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // header + sub
                Constraint::Length(14), // 5 level cards in a row
                Constraint::Min(4),    // selected detail panel
            ])
            .split(area);

        // --- Header ---
        let header_inner = inset(chunks[0], 2, 0);
        let header = Line::from(vec![
            Span::styled("Aegis security level", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
        ]);
        write_line(buf, header_inner.x, header_inner.y, &header, header_inner.width);

        let sub = Line::from(vec![
            Span::styled(
                "Sandbox aggressiveness for tool execution. Approval pipeline is always active regardless of level.",
                Style::default().fg(theme::DIM),
            ),
        ]);
        write_line(buf, header_inner.x, header_inner.y + 1, &sub, header_inner.width);

        // --- Level cards (4 across) ---
        let card_gap = 2;
        let card_pad = 2;
        let total_gap = card_gap * (SECURITY_LEVELS.len().saturating_sub(1) as u16);
        let cards_width = chunks[1].width.saturating_sub(card_pad * 2).saturating_sub(total_gap);
        let card_width = (cards_width / SECURITY_LEVELS.len() as u16).max(10);

        for (i, level) in SECURITY_LEVELS.iter().enumerate() {
            let is_selected = i == self.selected;
            let x = chunks[1].x + card_pad + (i as u16 * (card_width + card_gap));
            let right = chunks[1].x.saturating_add(chunks[1].width);
            let width = card_width.min(right.saturating_sub(x));
            if width == 0 {
                continue;
            }
            let card_area = Rect::new(x, chunks[1].y, width, chunks[1].height);

            // Card border: accent if selected, green if recommended, muted otherwise
            let border_color = if is_selected {
                theme::ACCENT
            } else if level.recommended {
                theme::GREEN
            } else {
                theme::MUTED
            };

            let bg = if is_selected { theme::ACCENT_FAINT } else { BG2 };

            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(bg))
                .render(card_area, buf);

            // Left accent stripe (2px via border-left simulation)
            let stripe_style = Style::default().fg(level.color);
            for dy in 1..card_area.height.saturating_sub(1) {
                buf[(card_area.x, card_area.y + dy)].set_symbol("▐").set_style(stripe_style);
            }

            let inner = inset(card_area, 2, 1);

            // ▸ SELECTED badge (top-right)
            if is_selected {
                let badge = Line::from(vec![
                    Span::styled("▸ SELECTED", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                ]);
                let badge_width = 12.min(inner.width);
                write_line(buf, inner.x + inner.width.saturating_sub(badge_width), inner.y, &badge, badge_width);
            }

            // ★ RECOMMENDED badge (top-right when not selected)
            if level.recommended && !is_selected {
                let badge = Line::from(vec![
                    Span::styled("★ REC", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
                ]);
                buf.set_line(inner.x + inner.width.saturating_sub(5), inner.y, &badge, 5);
            }

            // Glyph badge — JSX: 36×36 filled box, level-color bg, C.bg text.
            // TUI cell-approx: a short filled run of the glyph on a colored bg.
            let glyph_badge = Line::from(vec![
                Span::styled(
                    format!(" {} ", level.glyph),
                    Style::default()
                        .fg(theme::BG)
                        .bg(level.color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            write_line(buf, inner.x, inner.y + 1, &glyph_badge, inner.width);

            // Name — JSX C.white 14px bold.
            let name_line = Line::from(vec![
                Span::styled(level.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD)),
            ]);
            write_line(buf, inner.x, inner.y + 2, &name_line, inner.width);

            // Sub — JSX dim italic.
            let sub_line = Line::from(vec![
                Span::styled(level.sub, Style::default().fg(theme::DIM).add_modifier(Modifier::ITALIC)),
            ]);
            write_line(buf, inner.x, inner.y + 3, &sub_line, inner.width);

            // Blocked section
            if !level.blocked.is_empty() {
                let blocked_label = Line::from(vec![
                    Span::styled("BLOCKED", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
                ]);
                write_line(buf, inner.x, inner.y + 5, &blocked_label, inner.width);

                // JSX: blocked.slice(0, 3) — first 3 only, prefixed `✕ `.
                for (j, item) in level.blocked.iter().take(3).enumerate() {
                    let item_line = Line::from(vec![
                        Span::styled(format!("✕ {}", item), Style::default().fg(theme::DIM)),
                    ]);
                    write_line(buf, inner.x, inner.y + 6 + j as u16, &item_line, inner.width);
                }
            }
            // NOTE: JSX cards render no ALLOWED list — only BLOCKED. (Removed.)
        }

        // --- Selected detail panel (bottom) ---
        let sec = &SECURITY_LEVELS[self.selected];
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .style(Style::default().bg(BG2))
            .render(chunks[2], buf);

        let detail_inner = inset(chunks[2], 2, 1);

        // SELECTED: {name}
        let selected_label = Line::from(vec![
            Span::styled(
                format!("SELECTED: {}", sec.name.to_uppercase()),
                Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD),
            ),
        ]);
        write_line(buf, detail_inner.x, detail_inner.y, &selected_label, detail_inner.width);

        // Config write preview
        let config_text = format!(
            "Will write [aegis] level = \"{}\" to ~/.zeus/config.toml",
            self.config_level_value()
        );
        let config = Paragraph::new(config_text).style(Style::default().fg(FG));
        config.render(
            Rect::new(
                detail_inner.x,
                detail_inner.y + 2,
                detail_inner.width,
                detail_inner.height.saturating_sub(2),
            ),
            buf,
        );
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn render_buffer(width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| {
            let screen = SecurityScreen::new();
            screen.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        term.backend().buffer().clone()
    }

    fn region_text(buf: &Buffer, x: u16, y: u16, width: u16, height: u16) -> String {
        let mut out = String::new();
        let max_x = x.saturating_add(width).min(buf.area.width);
        let max_y = y.saturating_add(height).min(buf.area.height);
        for row in y..max_y {
            for col in x..max_x {
                out.push_str(buf[(col, row)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn assert_security_render_clamped(width: u16, height: u16) {
        let buf = render_buffer(width, height);
        let dump = region_text(&buf, 0, 0, width, height);

        assert!(
            dump.contains("Aegis security level"),
            "header must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("Standard"),
            "selected Standard card must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("SELECTED: STANDARD"),
            "selected detail must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("[aegis] level = \"standard\""),
            "write preview must render untruncated at {width}x{height}:\n{dump}"
        );
        let preview_count = dump.matches("[aegis] level = \"standard\"").count();
        assert_eq!(
            preview_count, 1,
            "write preview must appear exactly once and fully intact at {width}x{height}:\n{dump}"
        );
    }

    #[test]
    fn security_render_clamps_at_narrow_width() {
        assert_security_render_clamped(80, 30);
    }

    #[test]
    fn security_render_clamps_at_normal_width() {
        assert_security_render_clamped(120, 40);
    }
}
