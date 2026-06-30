use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Truncate `text` to at most `max_w` display columns, appending `…` when clipped.
/// Returns the original string when it already fits. A `max_w` of 0 yields empty.
/// #271: card interior values (name/desc/sdk) were written via unclamped
/// `set_string` and overflowed the right border into the adjacent column.
fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let n = text.chars().count();
    if n <= max_w {
        return text.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let keep = max_w - 1;
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// Channel entry — matches JSX CHANNELS array (line 112).
struct Channel {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    group: &'static str,
    desc: &'static str,
    sdk: &'static str,
}

const CHANNELS: &[Channel] = &[
    Channel {
        id: "telegram",
        name: "Telegram",
        glyph: "TG",
        color: theme::BLUE,
        group: "Cloud APIs",
        desc: "Full chat, groups, bots, media",
        sdk: "grammers MTProto",
    },
    Channel {
        id: "discord",
        name: "Discord",
        glyph: "DC",
        color: theme::PURPLE,
        group: "Cloud APIs",
        desc: "Channels, threads, reactions, embeds",
        sdk: "Serenity gateway",
    },
    Channel {
        id: "slack",
        name: "Slack",
        glyph: "SL",
        color: theme::GREEN,
        group: "Cloud APIs",
        desc: "Channels, threads, DMs, files",
        sdk: "Socket Mode + Web API",
    },
    Channel {
        id: "email",
        name: "Email",
        glyph: "EM",
        color: theme::AMBER,
        group: "Cloud APIs",
        desc: "Send, read, search, flag, forward",
        sdk: "lettre SMTP + IMAP",
    },
    Channel {
        id: "irc",
        name: "IRC",
        glyph: "IR",
        color: theme::YELLOW,
        group: "Cloud APIs",
        desc: "Channels, nicks, highlights, TLS",
        sdk: "Tokio IRC client",
    },
    Channel {
        id: "x_twitter",
        name: "X / Twitter",
        glyph: "X",
        color: theme::WHITE,
        group: "Cloud APIs",
        desc: "Post, reply, mentions, DMs",
        sdk: "v2 API + OAuth 1.0a",
    },
    Channel {
        id: "imessage",
        name: "iMessage",
        glyph: "iM",
        color: theme::CYAN,
        group: "Phone-paired",
        desc: "Send, read, conversations (macOS)",
        sdk: "AppleScript bridge",
    },
    Channel {
        id: "whatsapp",
        name: "WhatsApp",
        glyph: "WA",
        color: theme::GREEN,
        group: "Phone-paired",
        desc: "Requires QR scan from your phone",
        sdk: "Cloud API",
    },
    Channel {
        id: "signal",
        name: "Signal",
        glyph: "SG",
        color: theme::BLUE,
        group: "Phone-paired",
        desc: "Requires QR scan from your phone",
        sdk: "signal-cli JSON-RPC",
    },
    Channel {
        id: "matrix",
        name: "Matrix",
        glyph: "MX",
        color: theme::FIRE_ORANGE,
        group: "Phone-paired",
        desc: "Decentralized, end-to-end encrypted",
        sdk: "matrix-rust-sdk",
    },
];

const GROUPS: &[&str] = &["Cloud APIs", "Phone-paired"];

/// Channels screen — matches JSX ChannelsStep (line ~950).
/// Two-column layout: channel grid (left) + selected summary (right).
pub struct ChannelsScreen {
    pub selected: Vec<usize>, // indices into CHANNELS
    pub focused: usize,       // index into CHANNELS
}

impl Default for ChannelsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelsScreen {
    pub fn new() -> Self {
        Self {
            selected: vec![1, 0], // discord, telegram pre-selected per JSX default
            focused: 0,
        }
    }

    pub fn toggle_focused(&mut self) {
        if let Some(pos) = self.selected.iter().position(|&i| i == self.focused) {
            self.selected.remove(pos);
        } else {
            self.selected.push(self.focused);
        }
    }

    /// Ids of the currently selected channels (for the ChannelConfig step).
    pub fn selected_ids(&self) -> Vec<String> {
        self.selected.iter().map(|&i| CHANNELS[i].id.to_string()).collect()
    }

    /// ←: move focus to the previous card (horizontal grid nav).
    pub fn move_left(&mut self) {
        if self.focused > 0 {
            self.focused -= 1;
        }
    }

    /// →: move focus to the next card (horizontal grid nav).
    pub fn move_right(&mut self) {
        if self.focused + 1 < CHANNELS.len() {
            self.focused += 1;
        }
    }

    /// ↑: move focus up one row (2-col grid → step back by 2, clamped).
    pub fn move_up(&mut self) {
        if self.focused >= 2 {
            self.focused -= 2;
        } else {
            self.focused = 0;
        }
    }

    /// ↓: move focus down one row (2-col grid → step forward by 2, clamped).
    pub fn move_down(&mut self) {
        let next = self.focused + 2;
        if next < CHANNELS.len() {
            self.focused = next;
        } else {
            self.focused = CHANNELS.len() - 1;
        }
    }
}

impl Widget for &ChannelsScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.width < 10 || area.height < 6 {
            return;
        }

        // Two-column layout: left = grid, right = selected summary
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);

        let left = cols[0];
        let right = cols[1];

        // ── LEFT: Channel grid ──
        // The app-level StepHeader already renders the title/subtitle. Starting
        // directly at the groups preserves the prototype's compact 100×30
        // rhythm and keeps the footer from colliding with low rows.
        let mut cy = left.y;

        for group in GROUPS {
            // Group header with divider
            let group_label = group.to_uppercase();
            let hint = if *group == "Cloud APIs" {
                "API key auth"
            } else {
                "QR pairing required"
            };

            buf.set_string(
                left.x,
                cy,
                &group_label,
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            );
            // Divider line
            let divider_x = left.x + group_label.len() as u16 + 1;
            let divider_end = left.x + left.width - hint.len() as u16 - 1;
            for x in divider_x..divider_end {
                buf.set_string(x, cy, "─", Style::default().fg(theme::MUTED));
            }
            buf.set_string(
                divider_end,
                cy,
                hint,
                Style::default().fg(theme::MUTED),
            );
            cy += 1;

            // Grid: 2 columns per group
            let group_channels: Vec<(usize, &Channel)> = CHANNELS
                .iter()
                .enumerate()
                .filter(|(_, c)| c.group == *group)
                .collect();

            let mut row_y = cy;
            for chunk in group_channels.chunks(2) {
                let mut cx = left.x;
                let col_width = (left.width / 2).saturating_sub(1);

                for (global_idx, ch) in chunk.iter() {
                    let is_focused = self.focused == *global_idx;
                    let is_selected = self.selected.contains(global_idx);

                    // Card background / border
                    let _card_style = if is_focused {
                        Style::default().bg(theme::BG_HIGHLIGHT)
                    } else {
                        Style::default().bg(theme::BG_PANEL)
                    };

                    // Draw card box
                    let card_x = cx;
                    let card_w = col_width.min(left.width - (cx - left.x));
                    // Vertical-fit guard: the card spans row_y..=row_y+5 (bot_y).
                    // ratatui's set_string panics (index outside of buffer) rather
                    // than clipping on an out-of-bounds y, so skip any card whose
                    // bottom border would fall on/past area.bottom() when the body
                    // region is short a row.
                    let card_fits = row_y + 5 < area.bottom();
                    if card_w > 4 && card_fits {
                        // Top border
                        buf.set_string(card_x, row_y, "┌", Style::default().fg(theme::MUTED));
                        for x in (card_x + 1)..(card_x + card_w - 1) {
                            buf.set_string(x, row_y, "─", Style::default().fg(theme::MUTED));
                        }
                        buf.set_string(card_x + card_w - 1, row_y, "┐", Style::default().fg(theme::MUTED));

                        // Left accent bar if focused
                        let content_y = row_y + 1;
                        if is_focused && card_w > 2 {
                            buf.set_string(card_x, content_y, "│", Style::default().fg(theme::FIRE_ORANGE));
                        } else if card_w > 2 {
                            buf.set_string(card_x, content_y, "│", Style::default().fg(theme::MUTED));
                        }

                        // Glyph + checkbox
                        let checkbox = if is_selected { "[✓]" } else { "[ ]" };
                        let glyph_text = format!("{} {}", ch.glyph, checkbox);
                        let content_x = card_x + 2;
                        buf.set_string(
                            content_x,
                            content_y,
                            &glyph_text,
                            Style::default().fg(ch.color).add_modifier(Modifier::BOLD),
                        );

                        // Interior text budget: from content_x to the right
                        // border (card_x + card_w - 1), exclusive. Clamp every
                        // value so a long name/desc/sdk truncates with `…`
                        // INSIDE the card rather than overflowing the border.
                        let text_w =
                            (card_w as usize).saturating_sub(3); // 2 left pad + 1 border

                        // Name
                        let name_y = content_y + 1;
                        buf.set_string(
                            content_x,
                            name_y,
                            clamp_ellipsis(ch.name, text_w),
                            Style::default()
                                .fg(theme::TEXT)
                                .add_modifier(Modifier::BOLD),
                        );

                        // Desc
                        let desc_y = name_y + 1;
                        buf.set_string(
                            content_x,
                            desc_y,
                            clamp_ellipsis(ch.desc, text_w),
                            Style::default().fg(theme::DIM),
                        );

                        // SDK
                        let sdk_y = desc_y + 1;
                        buf.set_string(
                            content_x,
                            sdk_y,
                            clamp_ellipsis(ch.sdk, text_w),
                            Style::default().fg(theme::MUTED),
                        );

                        // Right border
                        let bot_y = sdk_y + 1;
                        if is_focused && card_w > 2 {
                            buf.set_string(card_x + card_w - 1, content_y, "│", Style::default().fg(theme::FIRE_ORANGE));
                            buf.set_string(card_x + card_w - 1, name_y, "│", Style::default().fg(theme::FIRE_ORANGE));
                            buf.set_string(card_x + card_w - 1, desc_y, "│", Style::default().fg(theme::FIRE_ORANGE));
                            buf.set_string(card_x + card_w - 1, sdk_y, "│", Style::default().fg(theme::FIRE_ORANGE));
                        } else if card_w > 2 {
                            buf.set_string(card_x + card_w - 1, content_y, "│", Style::default().fg(theme::MUTED));
                            buf.set_string(card_x + card_w - 1, name_y, "│", Style::default().fg(theme::MUTED));
                            buf.set_string(card_x + card_w - 1, desc_y, "│", Style::default().fg(theme::MUTED));
                            buf.set_string(card_x + card_w - 1, sdk_y, "│", Style::default().fg(theme::MUTED));
                        }

                        // Bottom border
                        buf.set_string(card_x, bot_y, "└", Style::default().fg(if is_focused { theme::FIRE_ORANGE } else { theme::MUTED }));
                        for x in (card_x + 1)..(card_x + card_w - 1) {
                            buf.set_string(x, bot_y, "─", Style::default().fg(if is_focused { theme::FIRE_ORANGE } else { theme::MUTED }));
                        }
                        buf.set_string(card_x + card_w - 1, bot_y, "┘", Style::default().fg(if is_focused { theme::FIRE_ORANGE } else { theme::MUTED }));
                    }

                    cx += col_width + 1;
                }
                row_y += 6; // card height + gap
            }
            cy = row_y + 1;
        }

        // ── RIGHT: Selected summary ──
        let mut ry = right.y;
        let selected_label = format!("SELECTED ({})", self.selected.len());
        buf.set_string(
            right.x + 1,
            ry,
            &selected_label,
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        );
        ry += 2;

        if self.selected.is_empty() {
            // Empty-state: 1px dashed border box, centered copy (JSX 998–1000).
            let box_w = right.width.saturating_sub(2).max(4);
            let box_x = right.x + 1;
            let box_top = ry;
            let box_h: u16 = 4;
            // Top + bottom dashed rules
            for x in box_x..(box_x + box_w) {
                buf.set_string(x, box_top, "╌", Style::default().fg(theme::MUTED));
                buf.set_string(x, box_top + box_h - 1, "╌", Style::default().fg(theme::MUTED));
            }
            // Side dashed rules
            for y in (box_top + 1)..(box_top + box_h - 1) {
                buf.set_string(box_x, y, "╎", Style::default().fg(theme::MUTED));
                buf.set_string(box_x + box_w - 1, y, "╎", Style::default().fg(theme::MUTED));
            }
            // Centered copy
            let l1 = "No channels selected.";
            let l2 = "Zeus will run console-only.";
            let c1x = box_x + (box_w.saturating_sub(l1.chars().count() as u16)) / 2;
            let c2x = box_x + (box_w.saturating_sub(l2.chars().count() as u16)) / 2;
            buf.set_string(c1x, box_top + 1, l1, Style::default().fg(theme::MUTED));
            buf.set_string(c2x, box_top + 2, l2, Style::default().fg(theme::MUTED));
        } else {
            for &idx in &self.selected {
                let ch = &CHANNELS[idx];
                // Left-border accent in the provider color (JSX borderLeft 2px).
                buf.set_string(
                    right.x + 1,
                    ry,
                    "▌",
                    Style::default().fg(ch.color),
                );
                let line = format!("{}  {}", ch.glyph, ch.name);
                // Clamp to the panel interior (from right.x+3 to the edge,
                // less 1 for breathing room) so a long name never bleeds past
                // the summary panel.
                let line_w = (right.width as usize).saturating_sub(4);
                buf.set_string(
                    right.x + 3,
                    ry,
                    clamp_ellipsis(&line, line_w),
                    Style::default().fg(ch.color).add_modifier(Modifier::BOLD),
                );
                // Dim "next: config" right-aligned in the panel.
                let nxt = "next: config";
                let nxt_x = right.x + right.width.saturating_sub(nxt.len() as u16 + 1);
                if nxt_x > right.x + 3 + line.chars().count() as u16 {
                    buf.set_string(nxt_x, ry, nxt, Style::default().fg(theme::DIM));
                }
                ry += 2;
            }
        }

        // Hint
        let hint_y = area.y + area.height.saturating_sub(1);
        buf.set_string(
            left.x,
            hint_y,
            "↑↓←→ navigate  •  space toggle  •  ↵ continue  •  esc back",
            Style::default().fg(theme::MUTED),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    /// Render the screen into a fresh buffer of the given size and return the
    /// per-row text (symbols only) for inspection.
    fn render_rows(width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        let screen = ChannelsScreen::new();
        (&screen).render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Width of the left grid column (65% of total), matching the layout split.
    fn left_width(total: u16) -> u16 {
        (total as u32 * 65 / 100) as u16
    }

    #[test]
    fn render_verify_dump_narrow_and_normal() {
        // Visual aid — run with `--nocapture` to eyeball the parity.
        for w in [64u16, 120u16] {
            println!("=== width {w} ===");
            for (i, row) in render_rows(w, 30).iter().enumerate() {
                println!("{i:2}|{}|", row.trim_end());
            }
        }
    }

    /// LOAD-BEARING: at a narrow width the card interior values (name/desc/sdk)
    /// must never write past the card's right border into the adjacent column.
    /// The card-interior text begins at `card_x + 2`; the right border sits at
    /// `card_x + card_w - 1`. We render each card's interior in isolation and
    /// assert that the value glyphs all land strictly left of the border. The
    /// previous full-screen render confirms no overflow into the second card.
    /// Proven-by-revert: removing the `clamp_ellipsis` calls makes the long
    /// "Channels, threads, reactions, embeds" desc overflow the border column
    /// and this fails on the `interior_right` assertion.
    #[test]
    fn narrow_width_card_values_clamp_inside_border() {
        let width = 64u16;
        let height = 30u16;
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        let screen = ChannelsScreen::new();
        (&screen).render(area, &mut buf);

        let left_w = left_width(width);
        let col_width = (left_w / 2).saturating_sub(1);
        // First card: card_x=0, card_w=col_width. Its right border sits at
        // x=col_width-1; the GAP between the two cards is at x=col_width. An
        // unclamped desc/sdk overflows the border and deposits text glyphs into
        // the gap (e.g. raw "Full chat, groups, bots, media" bleeds past the
        // border). With clamp_ellipsis, the gap stays free of text on card rows.
        let border_x = col_width as u16; // first card right border
        let gap_x = (col_width + 1) as u16; // gap between the two cards

        // Detect card rows by the border glyph at border_x — the desc/sdk
        // content rows START WITH SPACES (no left-edge border), so checking
        // x=0 would skip exactly the overflowing lines. The border column holds
        // a box-drawing glyph on every row the card spans.
        let mut checked = 0;
        for y in 4..(height - 1) {
            let at_border = buf[(border_x, y)].symbol();
            let is_card_row = matches!(at_border, "\u{2502}" | "\u{250c}" | "\u{2510}" | "\u{2514}" | "\u{2518}");
            if !is_card_row {
                continue;
            }
            checked += 1;
            let in_gap = buf[(gap_x, y)].symbol();
            assert!(
                !in_gap
                    .chars()
                    .next()
                    .map(|c| c.is_alphanumeric())
                    .unwrap_or(false),
                "card interior bled past its border into the gap at row {y}: {:?}",
                in_gap
            );
        }
        assert!(checked > 0, "expected to inspect at least one card row");
    }

    /// LOAD-BEARING: at a wide width a desc that FITS the card must render in
    /// full with no premature clamp. The Telegram desc "Full chat, groups,
    /// bots, media" (30 cols) fits the ~36-col wide card, so it must appear
    /// intact and without an ellipsis — guards against over-clamping.
    #[test]
    fn normal_width_renders_fitting_desc_in_full() {
        let rows = render_rows(120, 30);
        let joined = rows.join("\n");
        assert!(
            joined.contains("Full chat, groups, bots, media"),
            "wide render must show the full Telegram desc, got:\n{joined}"
        );
        // And it must NOT be ellipsified.
        assert!(
            !joined.contains("Full chat, groups, bots, medi…"),
            "fitting desc must not be over-clamped, got:\n{joined}"
        );
    }

    /// LOAD-BEARING: at a NARROW width the longest desc must truncate with `…`
    /// (proven-by-revert: drop the clamp_ellipsis calls and the raw desc
    /// overflows instead of ending in `…`, failing this).
    #[test]
    fn narrow_width_long_desc_truncates_with_ellipsis() {
        let rows = render_rows(64, 30);
        let joined = rows.join("\n");
        // The raw 36-char Discord desc cannot fit the ~16-col narrow card.
        assert!(
            !joined.contains("Channels, threads, reactions, embeds"),
            "narrow render must NOT show the full overflowing desc"
        );
        // It must instead be present as a clamped, ellipsified prefix.
        assert!(
            joined.contains('…'),
            "narrow render must clamp long values with an ellipsis"
        );
    }
}
