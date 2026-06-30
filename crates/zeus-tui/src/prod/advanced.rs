use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Advanced tabs — matches JSX ADVANCED_TABS (docs/zeus-tui-production.jsx line 32).
/// 13 specialized subsystem views. ids/names/descs/glyphs/colors EXACTLY per JSX.
pub const ADVANCED_TABS: &[AdvTabDef] = &[
    AdvTabDef { id: "agents", name: "Agents", glyph: "AGT", color: theme::ACCENT, desc: "Local + fleet roster, personas, bindings" },
    AdvTabDef { id: "skills", name: "Skills", glyph: "SKL", color: theme::AMBER, desc: "Marketplace, install/enable, SKILL.md" },
    AdvTabDef { id: "mcp", name: "MCP Servers", glyph: "MCP", color: theme::CYAN, desc: "Connected servers, tools, health" },
    AdvTabDef { id: "projects", name: "Projects", glyph: "PRJ", color: theme::GREEN, desc: "Create, assign agents, status" },
    AdvTabDef { id: "canvas", name: "Canvas", glyph: "CNV", color: theme::PURPLE, desc: "Visual plan / workflow builder" },
    AdvTabDef { id: "voice", name: "Voice", glyph: "VCE", color: theme::BLUE, desc: "Calls, STT/TTS config, recordings" },
    AdvTabDef { id: "nodecomms", name: "NodeComms", glyph: "NCM", color: theme::CYAN, desc: "Inter-agent fleet messaging" },
    AdvTabDef { id: "vectorstores", name: "VectorStores", glyph: "VEC", color: theme::AMBER, desc: "Mnemosyne collections, semantic search" },
    AdvTabDef { id: "economy", name: "Economy", glyph: "ECN", color: theme::GREEN, desc: "Agora wallet, marketplace, x402" },
    AdvTabDef { id: "extensions", name: "Extensions", glyph: "EXT", color: theme::PURPLE, desc: "Deno/MCP extensions, runtime" },
    AdvTabDef { id: "knowledge-graph", name: "Knowledge Graph", glyph: "GRA", color: theme::BLUE, desc: "Memory graph, communities" },
    AdvTabDef { id: "spawner", name: "Spawner", glyph: "SPN", color: theme::ACCENT, desc: "Active subagents, kill, logs" },
    AdvTabDef { id: "deploy", name: "Deploy / Daemon", glyph: "DPL", color: theme::RED, desc: "Health, restart, launchd logs" },
];

pub struct AdvTabDef {
    pub id: &'static str,
    pub name: &'static str,
    pub glyph: &'static str,
    pub color: ratatui::style::Color,
    pub desc: &'static str,
}

/// Advanced overlay — 3-column grid of 13 advanced tabs.
/// Matches JSX AdvancedTab (line 1270).
pub struct AdvancedOverlay {
    pub selected: Option<usize>,
}

impl Widget for AdvancedOverlay {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height < 5 || area.width < 40 {
            return;
        }

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(theme::BG));
            }
        }

        // Header
        buf.set_string(
            area.x + 2,
            area.y,
            "Advanced subsystems",
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            area.x + 2,
            area.y + 1,
            "13 specialized views — every backend feature has a TUI surface",
            Style::default().fg(theme::DIM),
        );

        // 3-column grid
        let cols = 3;
        let card_width = (area.width.saturating_sub(4)) / cols;
        let card_height = 3u16;
        let start_y = area.y + 3;

        for (i, tab) in ADVANCED_TABS.iter().enumerate() {
            let col = (i as u16) % cols;
            let row = (i as u16) / cols;
            let x = area.x + 2 + col * card_width;
            let y = start_y + row * (card_height + 1);

            if y + card_height > area.bottom() {
                break;
            }

            let is_selected = self.selected == Some(i);

            // Card background
            for cy in y..y + card_height {
                for cx in x..x + card_width.saturating_sub(1) {
                    buf[(cx, cy)]
                        .set_symbol(" ")
                        .set_style(Style::default().bg(theme::BG_PANEL));
                }
            }

            // Left accent stripe
            for cy in y..y + card_height {
                buf[(x, cy)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(tab.color));
            }

            // Glyph box
            buf.set_string(
                x + 2,
                y,
                format!("[{}]", tab.glyph),
                Style::default()
                    .fg(tab.color)
                    .add_modifier(Modifier::BOLD),
            );

            // Name
            buf.set_string(
                x + 7,
                y,
                tab.name,
                Style::default()
                    .fg(if is_selected { theme::TEXT_BRIGHT } else { theme::TEXT })
                    .add_modifier(Modifier::BOLD),
            );

            // Description
            buf.set_string(
                x + 2,
                y + 1,
                tab.desc,
                Style::default().fg(theme::DIM),
            );

            // Selection indicator
            if is_selected {
                buf.set_string(
                    x + card_width.saturating_sub(8),
                    y,
                    "▸ SELECTED",
                    Style::default()
                        .fg(theme::FIRE_ORANGE)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADVANCED_TABS must match JSX ADVANCED_TABS (docs/zeus-tui-production.jsx
    /// line 32) exactly: 13 entries, ids + glyphs in order.
    #[test]
    fn advanced_tabs_match_jsx() {
        let expected: &[(&str, &str)] = &[
            ("agents", "AGT"),
            ("skills", "SKL"),
            ("mcp", "MCP"),
            ("projects", "PRJ"),
            ("canvas", "CNV"),
            ("voice", "VCE"),
            ("nodecomms", "NCM"),
            ("vectorstores", "VEC"),
            ("economy", "ECN"),
            ("extensions", "EXT"),
            ("knowledge-graph", "GRA"),
            ("spawner", "SPN"),
            ("deploy", "DPL"),
        ];
        assert_eq!(ADVANCED_TABS.len(), 13, "must be the JSX 13");
        for (i, (id, glyph)) in expected.iter().enumerate() {
            assert_eq!(&ADVANCED_TABS[i].id, id, "id mismatch at {i}");
            assert_eq!(&ADVANCED_TABS[i].glyph, glyph, "glyph mismatch at {i}");
        }
    }

    /// The dispatcher must have a branch for every ADVANCED_TABS id
    /// (no panic / no silent miss).
    #[test]
    fn dispatcher_covers_all_ids() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 60, 10);
        for tab in ADVANCED_TABS {
            let mut buf = Buffer::empty(area);
            // Should render without panicking for every id.
            let live = crate::prod::advanced_sub::AdvancedLive::default();
            crate::prod::advanced_sub::render(tab.id, area, &mut buf, &live);
        }
    }
}
