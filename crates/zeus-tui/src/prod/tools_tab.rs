use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::prod::draw::BufferClampExt;
use crate::theme;

// ── Tool category ──
#[derive(Debug, Clone)]
pub struct ToolCategory {
    pub id: &'static str,
    pub name: &'static str,
    pub color: (u8, u8, u8),
}

/// Categories — matches JSX `categories` const (line 818).
pub const CATEGORIES: &[ToolCategory] = &[
    ToolCategory {
        id: "core",
        name: "Core",
        color: (255, 60, 20),
    },
    ToolCategory {
        id: "talos",
        name: "Talos · macOS",
        color: (255, 160, 80),
    },
    ToolCategory {
        id: "browser",
        name: "Browser CDP",
        color: (59, 130, 246),
    },
    ToolCategory {
        id: "git",
        name: "Git",
        color: (34, 197, 94),
    },
    ToolCategory {
        id: "files",
        name: "Files",
        color: (6, 182, 212),
    },
    ToolCategory {
        id: "shell",
        name: "Shell",
        color: (239, 68, 68),
    },
    ToolCategory {
        id: "memory",
        name: "Memory",
        color: (168, 85, 247),
    },
    ToolCategory {
        id: "channels",
        name: "Channels",
        color: (34, 197, 94),
    },
    ToolCategory {
        id: "media",
        name: "Media gen",
        color: (255, 160, 80),
    },
    ToolCategory {
        id: "mcp",
        name: "MCP",
        color: (6, 182, 212),
    },
];

// ── Tool entry ──
#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub name: &'static str,
    pub category: &'static str,
    pub desc: &'static str,
    pub danger: bool,
    pub schema: &'static str,
}

/// Tools — matches JSX `tools` const (line 830).
pub const TOOLS: &[ToolEntry] = &[
    ToolEntry {
        name: "shell",
        category: "shell",
        desc: "Execute shell command (sandboxed)",
        danger: true,
        schema: r#"{"command": "string", "cwd?": "string"}"#,
    },
    ToolEntry {
        name: "read_file",
        category: "files",
        desc: "Read file contents",
        danger: false,
        schema: r#"{"path": "string", "offset?": "int", "limit?": "int"}"#,
    },
    ToolEntry {
        name: "write_file",
        category: "files",
        desc: "Write or create file",
        danger: false,
        schema: r#"{"path": "string", "content": "string"}"#,
    },
    ToolEntry {
        name: "apply_patch",
        category: "files",
        desc: "Apply unified diff patch",
        danger: false,
        schema: r#"{"patch": "string"}"#,
    },
    ToolEntry {
        name: "web_fetch",
        category: "core",
        desc: "Fetch URL content (allowlisted)",
        danger: false,
        schema: r#"{"url": "string"}"#,
    },
    ToolEntry {
        name: "git_status",
        category: "git",
        desc: "Show working tree status",
        danger: false,
        schema: "{}",
    },
    ToolEntry {
        name: "git_commit",
        category: "git",
        desc: "Commit staged changes",
        danger: false,
        schema: r#"{"message": "string", "amend?": "bool"}"#,
    },
    ToolEntry {
        name: "applescript_calendar_create",
        category: "talos",
        desc: "Create Calendar event via AppleScript",
        danger: false,
        schema: r#"{"title": "string", "start": "datetime", "end": "datetime"}"#,
    },
    ToolEntry {
        name: "browser_navigate",
        category: "browser",
        desc: "Navigate Chrome to URL",
        danger: false,
        schema: r#"{"url": "string"}"#,
    },
    ToolEntry {
        name: "browser_click",
        category: "browser",
        desc: "Click element by CSS selector",
        danger: false,
        schema: r#"{"selector": "string"}"#,
    },
    ToolEntry {
        name: "memory_store",
        category: "memory",
        desc: "Store fact in Mnemosyne",
        danger: false,
        schema: r#"{"content": "string", "memory_type?": "string"}"#,
    },
    ToolEntry {
        name: "memory_search",
        category: "memory",
        desc: "Search Mnemosyne vector + FTS",
        danger: false,
        schema: r#"{"query": "string", "max_results?": "int"}"#,
    },
    ToolEntry {
        name: "message",
        category: "channels",
        desc: "Send message to channel",
        danger: false,
        schema: r#"{"channel": "string", "content": "string"}"#,
    },
    ToolEntry {
        name: "spawn",
        category: "core",
        desc: "Spawn background subagent",
        danger: false,
        schema: r#"{"task": "string", "context?": "string"}"#,
    },
    ToolEntry {
        name: "web_search",
        category: "core",
        desc: "Search the web (DuckDuckGo)",
        danger: false,
        schema: r#"{"query": "string", "max_results?": "int"}"#,
    },
];

/// Tools tab — matches JSX ToolsTab (line 816).
/// 3-column layout: categories (left), tool list (middle), tool detail (right).
pub struct ToolsTab<'a> {
    pub selected_category: Option<&'a str>,
    pub selected_tool: &'a str,
    pub tool_filter: &'a str,
    pub scroll_offset: usize,
    /// Live tool registry from the gateway (leaked to 'static, loaded once).
    /// `None` = use the seed `TOOLS` catalog (standalone / pre-fetch).
    pub tools: Option<&'static [ToolEntry]>,
}

impl<'a> Widget for ToolsTab<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.height < 5 || area.width < 60 {
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

        // 3-column split: categories(20) | tool list(35) | detail(rest)
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Length(39),
                Constraint::Min(30),
            ])
            .split(area);

        self.render_categories(cols[0], buf);
        self.render_tool_list(cols[1], buf);
        self.render_tool_detail(cols[2], buf);
    }
}

fn category_count(tools: &[ToolEntry], category: &str) -> usize {
    tools
        .iter()
        .filter(|tool| tool.category == category)
        .count()
}

impl<'a> ToolsTab<'a> {
    fn render_categories(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height < 2 {
            return;
        }

        // Header
        let header = Rect::new(area.x, area.y, area.width, 3.min(area.height));
        for x in area.x..area.right() {
            buf[(x, area.y + 2)]
                .set_symbol("─")
                .set_style(Style::default().fg(theme::BORDER));
        }
        buf.set_string_clamped(
            header.x + 1,
            header.y,
            "CATEGORIES",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        );
        let src = self.tools.unwrap_or(TOOLS);
        let total = src.len();
        buf.set_string_clamped(
            area.x + 1,
            area.y + 1,
            &format!("{} tools", total),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );

        // Category list
        for (i, cat) in CATEGORIES.iter().enumerate() {
            let y = area.y + 3 + i as u16 * 2;
            if y >= area.bottom().saturating_sub(1) {
                break;
            }

            let is_selected = self.selected_category == Some(cat.id);
            let cat_color = Color::Rgb(cat.color.0, cat.color.1, cat.color.2);

            // Color swatch
            buf.set_string_clamped(area.x + 1, y, "■", Style::default().fg(cat_color));

            // Category name
            buf.set_string_clamped(
                area.x + 3,
                y,
                cat.name,
                Style::default().fg(if is_selected {
                    theme::TEXT_BRIGHT
                } else {
                    theme::TEXT
                }),
            );

            // Count — derive from the current tool source instead of the prototype totals (#274).
            let source_count = category_count(src, cat.id);
            buf.set_string_clamped(
                area.right().saturating_sub(5),
                y,
                format!("{}", source_count),
                Style::default().fg(theme::DIM),
            );

            // Bottom border
            if y + 1 < area.bottom() {
                for x in area.x..area.right() {
                    buf[(x, y + 1)]
                        .set_symbol("─")
                        .set_style(Style::default().fg(theme::BORDER));
                }
            }
        }
    }

    fn render_tool_list(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height < 2 {
            return;
        }

        // Filter bar
        for x in area.x..area.right() {
            buf[(x, area.y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(theme::BG_PANEL));
        }
        buf.set_string_clamped(area.x + 1, area.y, "/", Style::default().fg(theme::DIM));
        let filter_display = if self.tool_filter.is_empty() {
            "Search tools…".to_string()
        } else {
            self.tool_filter.to_string()
        };
        buf.set_string_clamped(
            area.x + 3,
            area.y,
            &filter_display,
            Style::default().fg(if self.tool_filter.is_empty() {
                theme::DIM
            } else {
                theme::TEXT
            }),
        );

        // Filtered tools (live registry from the gateway when present, else seed)
        let src: &[ToolEntry] = self.tools.unwrap_or(TOOLS);
        let filtered: Vec<&ToolEntry> = src
            .iter()
            .filter(|t| {
                self.tool_filter.is_empty()
                    || t.name.contains(&self.tool_filter.to_lowercase())
                    || t.desc
                        .to_lowercase()
                        .contains(&self.tool_filter.to_lowercase())
            })
            .collect();

        // Count
        buf.set_string_clamped(
            area.right().saturating_sub(4),
            area.y,
            format!("{}", filtered.len()),
            Style::default().fg(theme::MUTED),
        );

        // Bottom border of filter bar
        for x in area.x..area.right() {
            buf[(x, area.y + 1)]
                .set_symbol("─")
                .set_style(Style::default().fg(theme::BORDER));
        }

        // Tool list
        for (i, tool) in filtered.into_iter().enumerate() {
            let y = area.y + 2 + i as u16 * 2;
            if y >= area.bottom() {
                break;
            }

            let is_selected = tool.name == self.selected_tool;

            // Selection highlight bg
            if is_selected {
                for x in area.x..area.right() {
                    buf[(x, y)]
                        .set_symbol(" ")
                        .set_style(Style::default().bg(theme::BG_HIGHLIGHT));
                }
            }

            // Tool name
            buf.set_string_clamped(
                area.x + 1,
                y,
                tool.name,
                Style::default()
                    .fg(if is_selected {
                        theme::TEXT_BRIGHT
                    } else {
                        theme::TEXT
                    })
                    .add_modifier(if is_selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            );

            // Danger badge mirrors prototype SANDBOXED marker.
            if tool.danger {
                buf.set_string_clamped(
                    area.right().saturating_sub(12),
                    y,
                    "● SANDBOXED",
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                );
            }

            // Description, second row like the prototype list item subtitle.
            if y + 1 < area.bottom() {
                let desc_text: String = tool
                    .desc
                    .chars()
                    .take(area.width.saturating_sub(5) as usize)
                    .collect();
                buf.set_string_clamped(
                    area.x + 3,
                    y + 1,
                    &desc_text,
                    Style::default().fg(theme::DIM),
                );
            }
        }
    }

    fn render_tool_detail(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height < 5 {
            return;
        }

        // Find selected tool (live registry when present, else seed)
        let src: &[ToolEntry] = self.tools.unwrap_or(TOOLS);
        let sel = src
            .iter()
            .find(|t| t.name == self.selected_tool)
            .unwrap_or(&src[0]);

        // Right detail pane border
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_set(border::PLAIN)
            .border_style(Style::default().fg(theme::BORDER));
        block.render(area, buf);

        let inner_x = area.x + 2;

        // Tool name + gear glyph
        buf.set_string_clamped(
            inner_x,
            area.y,
            "⚙",
            Style::default().fg(if sel.danger {
                theme::RED
            } else {
                theme::YELLOW
            }),
        );
        buf.set_string_clamped(
            inner_x + 2,
            area.y,
            sel.name,
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        );

        // Category tag
        if sel.danger {
            buf.set_string_clamped(
                area.right().saturating_sub(12),
                area.y,
                "SANDBOXED",
                Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
            );
        }
        let cat = CATEGORIES.iter().find(|c| c.id == sel.category);
        if let Some(cat) = cat {
            buf.set_string_clamped(
                inner_x,
                area.y + 1,
                cat.name,
                Style::default().fg(Color::Rgb(cat.color.0, cat.color.1, cat.color.2)),
            );
        }

        // Description
        buf.set_string_clamped(
            inner_x,
            area.y + 2,
            sel.desc,
            Style::default().fg(theme::TEXT),
        );

        // Schema
        buf.set_string_clamped(
            inner_x,
            area.y + 4,
            "SCHEMA",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(
            inner_x,
            area.y + 5,
            sel.schema,
            Style::default().fg(theme::DIM),
        );

        // Danger warning
        if sel.danger {
            buf.set_string_clamped(
                inner_x,
                area.y + 7,
                "⚠ DANGEROUS — requires approval",
                Style::default().fg(theme::RED),
            );
        }

        // Args input area
        let input_y = area.y + 9;
        if input_y < area.bottom().saturating_sub(4) {
            buf.set_string_clamped(
                inner_x,
                input_y,
                "ARGS",
                Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
            );

            // Input box border
            let box_y = input_y + 1;
            if box_y + 3 < area.bottom() {
                for x in inner_x..area.right().saturating_sub(1) {
                    buf[(x, box_y)]
                        .set_symbol("─")
                        .set_style(Style::default().fg(theme::BORDER));
                    buf[(x, box_y + 3)]
                        .set_symbol("─")
                        .set_style(Style::default().fg(theme::BORDER));
                }
                for y in box_y..=box_y + 3 {
                    buf[(inner_x, y)]
                        .set_symbol("│")
                        .set_style(Style::default().fg(theme::BORDER));
                    buf[(area.right().saturating_sub(2), y)]
                        .set_symbol("│")
                        .set_style(Style::default().fg(theme::BORDER));
                }
            }

            // Execute / Validate buttons
            let btn_y = box_y + 4;
            if btn_y < area.bottom() {
                buf.set_string_clamped(
                    inner_x,
                    btn_y,
                    "▸ EXECUTE",
                    Style::default()
                        .fg(theme::BG)
                        .bg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                );
                buf.set_string_clamped(
                    inner_x + 12,
                    btn_y,
                    "VALIDATE",
                    Style::default().fg(theme::DIM),
                );

                let source = if self.tools.is_some() {
                    "live /v1/tools"
                } else {
                    "seed catalog"
                };
                buf.set_string_clamped(
                    area.right().saturating_sub(28),
                    btn_y,
                    "last run · 14:32 · ✓ 24l",
                    Style::default().fg(theme::MUTED),
                );
                if btn_y + 1 < area.bottom() {
                    buf.set_string_clamped(
                        area.right().saturating_sub(20),
                        btn_y + 1,
                        source,
                        Style::default().fg(theme::MUTED),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_count_uses_source_tools() {
        let tools = [
            ToolEntry {
                name: "A",
                category: "core",
                desc: "",
                danger: false,
                schema: "{}",
            },
            ToolEntry {
                name: "B",
                category: "git",
                desc: "",
                danger: false,
                schema: "{}",
            },
            ToolEntry {
                name: "C",
                category: "git",
                desc: "",
                danger: false,
                schema: "{}",
            },
        ];

        assert_eq!(category_count(&tools, "core"), 1);
        assert_eq!(category_count(&tools, "git"), 2);
        assert_eq!(category_count(&tools, "missing"), 0);
    }
}
