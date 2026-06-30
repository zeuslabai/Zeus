use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Feature subsystem — matches JSX FEATURES const (line 139).
struct Feature {
    #[allow(dead_code)] // staged UI scaffolding
    id: &'static str,
    name: &'static str,
    color: ratatui::style::Color,
    desc: &'static str,
    #[allow(dead_code)] // staged for follow-up: per-platform availability gating
    platforms: &'static [&'static str],
    required_on: &'static str,
    warning: &'static str,
}

const FEATURES: &[Feature] = &[
    Feature {
        id: "talos",
        name: "Talos (macOS automation)",
        color: theme::FIRE_ORANGE,
        desc: "193 tools across Calendar, Notes, Mail, Safari, etc.",
        platforms: &["macOS"],
        required_on: "macOS",
        warning: "macOS gate — without this, image-gen, AppleScript, system-info ALL silently fail",
    },
    Feature {
        id: "nous",
        name: "Nous (cognitive learning)",
        color: theme::CYAN,
        desc: "Captures intent + improves over time. Optional but recommended.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "mnemosyne",
        name: "Mnemosyne (memory)",
        color: theme::AMBER,
        desc: "Three-layer persistent memory system.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "hermes",
        name: "Hermes (channels)",
        color: theme::GREEN,
        desc: "Cross-channel messaging coordination.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "athena",
        name: "Athena (research)",
        color: theme::PURPLE,
        desc: "Vault-based knowledge synthesis (Obsidian).",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "browser",
        name: "Browser (Chrome CDP)",
        color: theme::BLUE,
        desc: "11 browser automation tools.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "voice",
        name: "Voice (TTS/STT)",
        color: theme::CYAN,
        desc: "Twilio calls + Whisper STT + TTS.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
    Feature {
        id: "skills",
        name: "Skill marketplace",
        color: theme::YELLOW,
        desc: "Plugin system for adding tools.",
        platforms: &[],
        required_on: "",
        warning: "",
    },
];

/// Features screen — vertical toggle list of 8 subsystems.
/// Matches JSX FeaturesStep (line 1410).
pub struct FeaturesScreen {
    /// Set of toggled-on feature ids (indices into FEATURES).
    pub toggled: Vec<bool>,
    /// Currently focused feature index.
    pub focused: usize,
    /// Current platform (determines macOS-gate behavior).
    pub platform: &'static str,
}

impl Default for FeaturesScreen {
    fn default() -> Self {
        Self {
            // nous, mnemosyne, hermes on by default (matching JSX useState)
            toggled: vec![false, true, true, true, false, false, false, false],
            focused: 0,
            platform: "macOS",
        }
    }
}

impl FeaturesScreen {
    /// Toggle the focused feature. Talos on macOS is force-enabled (no-op toggle).
    pub fn toggle_focused(&mut self) {
        let f = &FEATURES[self.focused];
        let is_mandatory =
            !f.required_on.is_empty() && f.required_on.eq_ignore_ascii_case(self.platform);
        if is_mandatory {
            return; // force-enabled, can't toggle off
        }
        self.toggled[self.focused] = !self.toggled[self.focused];
    }

    pub fn move_up(&mut self) {
        if self.focused > 0 {
            self.focused -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.focused + 1 < FEATURES.len() {
            self.focused += 1;
        }
    }
}

impl Widget for FeaturesScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // title + subtitle
                Constraint::Length(5), // talos macOS-gate banner
                Constraint::Min(0),    // feature cards
            ])
            .split(area);

        // ── Title ──
        let title_x = inner[0].x.saturating_add(2);
        let title_w = inner[0].width.saturating_sub(2) as usize;
        let _ = buf.set_stringn(
            title_x,
            inner[0].y,
            "Enable subsystems",
            title_w,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        let _ = buf.set_stringn(
            title_x,
            inner[0].y + 1,
            "Toggle which Zeus crates are active in this deployment. Disabled crates compile but don't load.",
            title_w,
            Style::default().fg(theme::DIM),
        );

        // ── Talos macOS-gate banner (accent border, jsx:1421) ──
        let banner_area = inner[1];
        let banner_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::FIRE_ORANGE))
            .style(Style::default().bg(theme::BG_PANEL));
        let banner_inner = banner_block.inner(banner_area);
        banner_block.render(banner_area, buf);

        // Banner title
        buf.set_string(
            banner_inner.x + 1,
            banner_inner.y,
            "⚠ MACOS GATE — TALOS IS MANDATORY",
            Style::default()
                .fg(theme::FIRE_ORANGE)
                .add_modifier(Modifier::BOLD),
        );
        // Banner body — first line
        let line1 = "On macOS, the [talos] block must be present (even if empty) or";
        buf.set_string(
            banner_inner.x + 1,
            banner_inner.y + 1,
            line1,
            Style::default().fg(theme::TEXT),
        );
        // Banner body — second line
        let line2 = "193 tools — including image-gen, AppleScript, system-info — silently fail to register.";
        buf.set_string(
            banner_inner.x + 1,
            banner_inner.y + 2,
            line2,
            Style::default().fg(theme::TEXT),
        );

        // ── Feature cards ──
        let gap: u16 = 1;
        let mut y = inner[2].y;

        for (i, f) in FEATURES.iter().enumerate() {
            let is_focused = self.focused == i;
            let is_mandatory =
                !f.required_on.is_empty() && f.required_on.eq_ignore_ascii_case(self.platform);
            // JSX: isEnabled = isMandatory || toggled.has(f.id). Mandatory
            // (talos@macOS) reads ON regardless of the toggle vec.
            let is_enabled = is_mandatory || self.toggled[i];

            // Mandatory cards with warnings need 4 rows; others need 3
            let card_height: u16 = if is_mandatory && !f.warning.is_empty() {
                4
            } else {
                3
            };

            if y + card_height > inner[2].y + inner[2].height {
                break;
            }

            // Card border — focused gets accent, else muted
            let border_color = if is_focused {
                theme::FIRE_ORANGE
            } else {
                theme::BORDER
            };
            let card_area = Rect {
                x: inner[2].x.saturating_add(1),
                y,
                width: inner[2].width.saturating_sub(2),
                height: card_height,
            };
            if card_area.width <= 2 || card_area.height <= 2 {
                break;
            }
            let card_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(if is_focused {
                    theme::BG_HIGHLIGHT
                } else {
                    theme::BG_PANEL
                }));
            let card_inner = card_block.inner(card_area);
            card_block.render(card_area, buf);

            // Color dot + name
            let dot = "●";
            buf.set_string(
                card_inner.x,
                card_inner.y,
                dot,
                Style::default().fg(f.color).add_modifier(Modifier::BOLD),
            );
            let status_reserved = 7.min(card_inner.width);
            let row_w = card_inner
                .width
                .saturating_sub(status_reserved.saturating_add(2)) as usize;
            let _ = buf.set_stringn(
                card_inner.x + 2,
                card_inner.y,
                f.name,
                row_w,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            );

            // Platform badge (FORCE-ON ON {PLATFORM}) — only for mandatory features
            if is_mandatory {
                let badge = format!(" FORCE-ON ON {} ", self.platform.to_uppercase());
                let badge_x = card_inner.x + 2 + f.name.len() as u16 + 2;
                if badge_x + (badge.len() as u16) < card_inner.x + card_inner.width {
                    buf.set_string(
                        badge_x,
                        card_inner.y,
                        &badge,
                        Style::default()
                            .fg(theme::FIRE_ORANGE)
                            .bg(theme::ACCENT_FAINT)
                            .add_modifier(Modifier::BOLD),
                    );
                }
            }

            // Description
            let _ = buf.set_stringn(
                card_inner.x + 2,
                card_inner.y + 1,
                f.desc,
                card_inner.width.saturating_sub(2) as usize,
                Style::default().fg(theme::DIM),
            );

            // Warning line (only for mandatory features with warnings) — uses row 3
            if is_mandatory && !f.warning.is_empty() {
                let warning = format!("⚠ {}", f.warning);
                let _ = buf.set_stringn(
                    card_inner.x + 2,
                    card_inner.y + 2,
                    &warning,
                    card_inner.width.saturating_sub(2) as usize,
                    Style::default()
                        .fg(theme::AMBER)
                        .add_modifier(Modifier::ITALIC),
                );
            }

            // ON/OFF indicator — right-aligned on row 0
            let status = if is_enabled { "● ON" } else { "○ OFF" };
            let status_color = if is_enabled {
                theme::GREEN
            } else {
                theme::MUTED
            };
            let status_x = card_inner.x + card_inner.width.saturating_sub(6);
            let _ = buf.set_stringn(
                status_x,
                card_inner.y,
                status,
                6,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            );

            y += card_height + gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn render_buffer(screen: FeaturesScreen, width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| screen.render(f.area(), f.buffer_mut()))
            .unwrap();
        term.backend().buffer().clone()
    }

    fn region_text(buf: &Buffer, x: u16, y: u16, width: u16, height: u16) -> String {
        let area = buf.area();
        let mut out = String::new();
        for row in y..y.saturating_add(height).min(area.height) {
            for col in x..x.saturating_add(width).min(area.width) {
                out.push_str(buf[(col, row)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn features_render_clamps_at_narrow_width() {
        let mut screen = FeaturesScreen::default();
        screen.focused = 0;
        let buf = render_buffer(screen, 50, 40);
        let dump = region_text(&buf, 0, 0, 50, 40);

        assert!(
            dump.contains("Enable subsystems"),
            "title should survive narrow render:\n{dump}"
        );
        assert!(
            dump.contains("MACOS GATE"),
            "Talos warning banner should survive narrow render:\n{dump}"
        );
        assert!(
            dump.contains("Talos (macOS automation)"),
            "focused Talos card should keep the real feature name:\n{dump}"
        );
        assert!(
            dump.contains("● ON"),
            "force-enabled Talos status should remain visible and unclipped:\n{dump}"
        );
        assert!(
            dump.contains("Skill marketplace"),
            "list should render through the final feature at narrow width:\n{dump}"
        );
    }

    #[test]
    fn features_render_preserves_full_content_at_normal_width() {
        let mut screen = FeaturesScreen::default();
        screen.focused = 0;
        let buf = render_buffer(screen, 100, 34);
        let dump = region_text(&buf, 0, 0, 100, 34);

        assert!(
            dump.contains("Toggle which Zeus crates are active in this deployment"),
            "subtitle should render at normal width:\n{dump}"
        );
        assert!(
            dump.contains("FORCE-ON ON MACOS"),
            "mandatory Talos badge should render at normal width:\n{dump}"
        );
        assert!(
            dump.contains("image-gen, AppleScript, system-info ALL silently fail"),
            "Talos warning should render without clipping at normal width:\n{dump}"
        );
        assert!(
            dump.contains("Captures intent + improves over time. Optional but recommended."),
            "Nous real description should remain in the card list:\n{dump}"
        );
        assert!(
            dump.contains("○ OFF"),
            "disabled feature status should render at normal width:\n{dump}"
        );
    }
}
