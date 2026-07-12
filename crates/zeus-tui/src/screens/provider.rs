use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};

use crate::theme;

// Provider metadata now lives in the canonical shared registry
// (`crate::screens::providers`) so Provider / Model / Fallback all read ONE
// source of truth. `PROVIDERS` here is a thin alias to that const.
use crate::screens::providers::{self, PROVIDERS};

/// Id of the provider at `idx` (clamped) — for the Complete summary.
pub fn provider_id_at(idx: usize) -> &'static str {
    providers::id_at(idx)
}

/// Display fields (name, accent color, key format) for the provider at `idx`
/// (clamped) — drives the Auth screen for whichever provider was selected.
pub fn provider_display(idx: usize) -> (&'static str, ratatui::style::Color, &'static str) {
    providers::display(idx)
}


/// Provider selection screen — step 3 (PROV).
pub struct ProviderScreen {
    pub selected: usize,
    pub ollama_detected: Option<bool>,
}

impl Widget for ProviderScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.width < 10 || area.height < 5 {
            return;
        }

        // Three-region layout (JSX 577–648):
        //   left  list (~360px → ~38 cols min) | center detail (flex) | right HINTS (200px → ~26 cols)
        // Mirror the prototype's fixed-side / flex-center proportion at wide
        // sizes. At the 100x30 render gate the right HINTS rail steals enough
        // width that the selected-provider detail labels/body visibly clip, so
        // drop it there and prioritize readable list + detail.
        let right_w = if area.width >= 112 { 26 } else { 0 };
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(40),
                Constraint::Min(28),
                Constraint::Length(right_w),
            ])
            .split(area);

        // ── Left: provider list (bordered cards) ──
        // Per-region clear before each render (#250). These panels write via
        // direct `buf`/`set_line` without padding to full region width, and the
        // detail panel's content length varies per provider (config-box rows,
        // NEXT line position). Without clearing, stale glyphs from the prior
        // frame's longer content survive in the lower rows when the selection
        // changes to a provider with a shorter panel — the detail-over-card
        // bleed. Repaint each region with the page bg first (NOT `Clear`, which
        // resets to the terminal default and would punch holes in the themed
        // background — gate-b catch) so every region starts from a clean,
        // correctly-colored slate.
        // NOTE: a bare `Block::default().style(bg)` only rewrites cell *style*,
        // NOT the cell symbol — old glyphs survive (gate-b catch). `Clear`
        // resets the symbol to a space; we then repaint `theme::BG` so the
        // cleared region keeps the themed page background instead of the
        // terminal default. Clear-then-bg per region = clean, correctly-colored
        // slate every frame.
        let clear_region = |rect: Rect, buf: &mut ratatui::buffer::Buffer| {
            Clear.render(rect, buf);
            Block::default().style(Style::default().bg(theme::BG)).render(rect, buf);
        };
        clear_region(cols[0], buf);
        render_list(cols[0], buf, self.selected, self.ollama_detected);

        // ── Center: detail panel (badge + config-box) ──
        clear_region(cols[1], buf);
        render_detail(cols[1], buf, self.selected, self.ollama_detected);

        // ── Right: HINTS + RECOMMENDATIONS column ──
        if right_w > 0 {
            clear_region(cols[2], buf);
            render_hints(cols[2], buf, self.selected);
        }
    }
}

fn render_list(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    selected: usize,
    ollama_detected: Option<bool>,
) {
    // Slim card (#271) = 2 content rows (badge+name / 1-line desc) + top/bottom
    // border (2) + 1-row gutter (JSX gap:6) = 5 total. Body is the inner 2 rows;
    // model/pricing/key moved to the center detail panel.
    let item_height = 5u16;
    let visible_count = (area.height / item_height.max(1)) as usize;
    let total = PROVIDERS.len();

    // Simple scroll: center selected if possible
    let scroll_start = if selected >= visible_count {
        selected.saturating_sub(visible_count / 2)
    } else {
        0
    }
    .min(total.saturating_sub(visible_count.max(1)));

    for (idx, provider) in PROVIDERS.iter().enumerate().skip(scroll_start).take(visible_count) {
        let row_y = area.y + ((idx - scroll_start) as u16) * item_height;
        if row_y + item_height > area.bottom() {
            break;
        }

        let is_selected = idx == selected;
        // Bordered card: leave a 1-row gutter between cards (JSX gap:6).
        let card = Rect::new(area.x, row_y, area.width, item_height.saturating_sub(1));
        let row_area = card;

        // Card background (JSX: selected → C.accentFaint, focused → bg2, else bg).
        // We have no separate keyboard-focus state here, so: selected → accentFaint.
        if is_selected {
            for x in card.left()..card.right() {
                for y in card.top()..card.bottom() {
                    buf[(x, y)].set_style(Style::default().bg(theme::ACCENT_FAINT));
                }
            }
        }

        // Card border (JSX: `1px solid ${selected ? accent : featured ? amber : muted}`).
        let border_color = if is_selected {
            theme::FIRE_ORANGE
        } else if provider.featured {
            theme::AMBER
        } else {
            theme::MUTED
        };
        for x in card.left()..card.right() {
            buf[(x, card.top())].set_symbol("─").set_style(Style::default().fg(border_color));
            buf[(x, card.bottom() - 1)].set_symbol("─").set_style(Style::default().fg(border_color));
        }
        for y in card.top()..card.bottom() {
            buf[(card.left(), y)].set_symbol("│").set_style(Style::default().fg(border_color));
            buf[(card.right() - 1, y)].set_symbol("│").set_style(Style::default().fg(border_color));
        }
        buf[(card.left(), card.top())].set_symbol("┌").set_style(Style::default().fg(border_color));
        buf[(card.right() - 1, card.top())].set_symbol("┐").set_style(Style::default().fg(border_color));
        buf[(card.left(), card.bottom() - 1)].set_symbol("└").set_style(Style::default().fg(border_color));
        buf[(card.right() - 1, card.bottom() - 1)].set_symbol("┘").set_style(Style::default().fg(border_color));

        // Colored left rail (JSX: `borderLeft: 2px solid ${selected ? accent : color}`).
        // Overdraw the card's left border edge with the provider's brand color
        // (or accent when selected) to give each card its color identity.
        let rail_color = if is_selected { theme::FIRE_ORANGE } else { provider.color };
        for y in card.top()..card.bottom() {
            buf[(card.left(), y)].set_style(Style::default().fg(rail_color).bg(
                if is_selected { theme::ACCENT_FAINT } else { theme::BG },
            ));
        }

        // Selected caret (JSX: `▸` marker hanging at left:-14). Drawn in the
        // 1-row gutter to the left of the card's top content row.
        if is_selected && card.left() > area.left() {
            buf[(card.left() - 1, card.top() + 1)]
                .set_symbol("▸")
                .set_style(Style::default().fg(theme::FIRE_ORANGE).add_modifier(Modifier::BOLD));
        }

        // ── Glyph badge box (JSX: 32×18 box, `1px solid color`; selected →
        //    filled bg=color/fg=bg, else outlined bg=bg2/fg=color). We render a
        //    compact bracketed badge `[GLY]` carrying the same fill semantics.
        let badge_x = card.x + 2;
        let badge_y = card.y + 1;
        let (badge_fg, badge_bg) = if is_selected {
            (theme::BG, provider.color)
        } else {
            (provider.color, theme::BG_PANEL)
        };
        let badge_style = Style::default()
            .fg(badge_fg)
            .bg(badge_bg)
            .add_modifier(Modifier::BOLD);
        buf.set_string(badge_x, badge_y, " ", badge_style);
        buf.set_string(badge_x + 1, badge_y, provider.glyph, badge_style);
        let glyph_w = provider.glyph.chars().count() as u16;
        buf.set_string(badge_x + 1 + glyph_w, badge_y, " ", badge_style);

        // Name + badges sit to the right of the glyph badge.
        let content_x = badge_x + glyph_w + 4;
        let mut cy = card.y + 1;

        // Name line (glyph now lives in its own badge box, above).
        let mut spans = vec![Span::styled(
            provider.name,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )];

        // Featured badge
        if provider.featured {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "★ FEATURED",
                Style::default()
                    .fg(theme::FIRE_ORANGE)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Detected badge
        if provider.id == "ollama" && matches!(ollama_detected, Some(true)) {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "● DETECTED",
                Style::default().fg(theme::GREEN),
            ));
        }

        let line = Line::from(spans);
        buf.set_line(content_x, cy, &line, row_area.width.saturating_sub(2));
        cy += 1;

        // Sub description (1-line). Model / pricing / key-format intentionally
        // live ONLY in the center detail panel (#271 visual parity) — slim
        // cards = `[GLY] Name + 1-line desc`, no per-card meta cram/truncation.
        buf.set_string(
            content_x,
            cy,
            provider.sub,
            Style::default().fg(theme::DIM),
        );
        cy += 1;

        // (Cards are self-delimited by their border — no separator line needed.)
        let _ = cy;
    }

    // Scroll indicator if needed
    if total > visible_count {
        let indicator = format!("{} providers", total);
        let x = area.x + area.width.saturating_sub(indicator.len() as u16 + 1);
        buf.set_string(
            x,
            area.y + area.height.saturating_sub(1),
            &indicator,
            Style::default().fg(theme::DIM),
        );
    }
}

fn render_detail(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    selected: usize,
    ollama_detected: Option<bool>,
) {
    let provider = match PROVIDERS.get(selected) {
        Some(p) => p,
        None => return,
    };

    let inner = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), area.height.saturating_sub(2));

    // ── Header flex-row: 56×56 glyph badge (left) | name + badges + sub (right) ──
    // JSX badge is a 56×56px filled square in `p.color`. In TUI cells (~2:1
    // tall) that maps to a 6w × 3h filled block, glyph centered, color bg.
    const BADGE_W: u16 = 6;
    const BADGE_H: u16 = 3;
    let badge_top = inner.y;
    for by in badge_top..badge_top + BADGE_H {
        if by >= inner.bottom() {
            break;
        }
        for bx in inner.x..inner.x + BADGE_W {
            buf[(bx, by)].set_symbol(" ").set_style(Style::default().bg(provider.color));
        }
    }
    // Glyph centered in the badge (on the color bg, in the dark page color).
    let glyph_x = inner.x + BADGE_W.saturating_sub(provider.glyph.chars().count() as u16) / 2;
    let glyph_y = badge_top + BADGE_H / 2;
    buf.set_string(
        glyph_x,
        glyph_y,
        provider.glyph,
        Style::default()
            .fg(theme::BG)
            .bg(provider.color)
            .add_modifier(Modifier::BOLD),
    );

    // Right of the badge: name + FEATURED/DETECTED badges, then sub.
    let tx = inner.x + BADGE_W + 2;
    let tw = inner.width.saturating_sub(BADGE_W + 2);
    let mut name_spans = vec![Span::styled(
        provider.name,
        Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD),
    )];
    if provider.featured {
        name_spans.push(Span::raw("  "));
        name_spans.push(Span::styled(
            " FEATURED ",
            Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD),
        ));
    }
    if provider.id == "ollama" && matches!(ollama_detected, Some(true)) {
        name_spans.push(Span::raw("  "));
        name_spans.push(Span::styled(
            " ● DETECTED ",
            Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    buf.set_line(tx, badge_top, &Line::from(name_spans), tw);
    buf.set_string(tx, badge_top + 1, provider.sub, Style::default().fg(theme::DIM));

    // FLAGSHIP / PRICING / KEY FORMAT meta — stacked on TWO full-width rows
    // below the badge header (#271: use the empty center width, don't cram them
    // onto one badge-offset line where PRICING/KEY truncate). Row 1: FLAGSHIP +
    // PRICING; Row 2: KEY FORMAT. Rendered at full `inner.width` (not the
    // badge-offset `tw`) so the long `$/$ per Mtok` price never clips.
    // Three stacked label/value rows, each full-width — the center column is
    // only ~42 cols (`Min(28)` between the 40-wide list and 26-wide hints), so
    // a single combined line clips the price/key. One field per row never does.
    let meta_y = badge_top + BADGE_H;
    let meta_rows = [
        ("FLAGSHIP ", provider.flagship),
        ("PRICING ", provider.price),
        ("KEY FORMAT ", provider.key_fmt),
    ];
    for (i, (label, value)) in meta_rows.iter().enumerate() {
        let row = Line::from(vec![
            Span::styled(*label, Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
            Span::styled(*value, Style::default().fg(theme::TEXT)),
        ]);
        buf.set_line(inner.x, meta_y + i as u16, &row, inner.width);
    }

    // ── Config-box (JSX 615–620): bordered, "WILL WRITE TO ~/.zeus/config.toml". ──
    // Sits below the 3-row stacked meta block (meta_y .. meta_y+2) + 1 gap row.
    let box_y = meta_y + 3 + 1;
    let box_h = 4u16;
    if box_y + box_h <= inner.bottom() {
        let box_area = Rect::new(inner.x, box_y, inner.width, box_h);
        // Border (bg2 fill + muted frame).
        for x in box_area.left()..box_area.right() {
            buf[(x, box_area.top())].set_symbol("─").set_style(Style::default().fg(theme::MUTED));
            buf[(x, box_area.bottom() - 1)].set_symbol("─").set_style(Style::default().fg(theme::MUTED));
        }
        for y in box_area.top()..box_area.bottom() {
            buf[(box_area.left(), y)].set_symbol("│").set_style(Style::default().fg(theme::MUTED));
            buf[(box_area.right() - 1, y)].set_symbol("│").set_style(Style::default().fg(theme::MUTED));
        }
        buf[(box_area.left(), box_area.top())].set_symbol("┌");
        buf[(box_area.right() - 1, box_area.top())].set_symbol("┐");
        buf[(box_area.left(), box_area.bottom() - 1)].set_symbol("└");
        buf[(box_area.right() - 1, box_area.bottom() - 1)].set_symbol("┘");

        buf.set_string(
            box_area.left() + 2,
            box_area.top() + 1,
            "WILL WRITE TO ~/.zeus/config.toml",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        );
        let cfg = Line::from(vec![
            Span::styled("model", Style::default().fg(theme::TEXT)),
            Span::styled(" = ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("\"{}/{}\"", provider.id, provider.flagship),
                Style::default().fg(theme::ACCENT_BRIGHT),
            ),
        ]);
        buf.set_line(box_area.left() + 2, box_area.top() + 2, &cfg, box_area.width.saturating_sub(4));
    }

    // ── NEXT line (JSX 622–625). ──
    let next_y = box_y + box_h + 1;
    if next_y < inner.bottom() {
        let next_line = Line::from(vec![
            Span::styled("NEXT ", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
            Span::styled("Step 04 (AUTH) will collect the API key for ", Style::default().fg(theme::DIM)),
            Span::styled(provider.name, Style::default().fg(theme::ACCENT_BRIGHT)),
            Span::styled(".", Style::default().fg(theme::DIM)),
        ]);
        buf.set_line(inner.x, next_y, &next_line, inner.width);
    }
}

/// Right column — HINTS + RECOMMENDATIONS (JSX 628–648).
fn render_hints(area: Rect, buf: &mut ratatui::buffer::Buffer, _selected: usize) {
    // Left border + bg2 panel.
    for y in area.top()..area.bottom() {
        buf[(area.left(), y)].set_symbol("│").set_style(Style::default().fg(theme::MUTED));
    }
    let inner = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(3), area.height.saturating_sub(2));
    let mut cy = inner.y;

    buf.set_string(inner.x, cy, "HINTS", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD));
    cy += 2;

    // Wrapped hint body.
    let hint = "Pick the provider you'll use most. You can configure backup providers in step 06.";
    for line in wrap(hint, inner.width as usize) {
        if cy >= inner.bottom() {
            break;
        }
        buf.set_string(inner.x, cy, &line, Style::default().fg(theme::TEXT));
        cy += 1;
    }
    cy += 1;

    if cy < inner.bottom() {
        buf.set_string(inner.x, cy, "RECOMMENDATIONS", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD));
        cy += 2;
    }

    // NOTE: recommendations name providers from the canonical PROVIDERS const,
    // not the JSX prototype — the JSX's "Speed → Groq" is stale (Groq was
    // dropped in the provider unification; xAI/Grok replaced it). Substrate-
    // truth (the list) is the authority, so the right column never names a
    // provider absent from the left list.
    let recs: &[(&str, &str, ratatui::style::Color)] = &[
        ("Reasoning", "Anthropic", theme::ACCENT),
        ("Multimodal", "OpenAI", theme::GREEN),
        ("Throughput", "MiniMax", theme::AMBER),
        ("Local", "Ollama", theme::CYAN),
        ("Speed", "xAI", theme::YELLOW),
    ];
    for (cat, prov, color) in recs {
        if cy >= inner.bottom() {
            break;
        }
        let line = Line::from(vec![
            Span::styled("● ", Style::default().fg(*color)),
            Span::styled(format!("{} → ", cat), Style::default().fg(theme::DIM)),
            Span::styled(*prov, Style::default().fg(*color)),
        ]);
        buf.set_line(inner.x, cy, &line, inner.width);
        cy += 1;
    }
}

/// Minimal greedy word-wrap for the hints body.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.len() + 1 + word.len() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::widgets::Widget as _;

    /// Render the whole buffer to a single string for glyph/text diffing.
    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }


    #[test]
    fn ollama_badge_absent_until_probe_success() {
        let selected = PROVIDERS
            .iter()
            .position(|p| p.id == "ollama")
            .expect("ollama provider present");
        let area = Rect { x: 0, y: 0, width: 110, height: 30 };

        for state in [None, Some(false)] {
            let mut buf = Buffer::empty(area);
            ProviderScreen {
                selected,
                ollama_detected: state,
            }
            .render(area, &mut buf);
            let rendered = buffer_text(&buf);
            assert!(
                !rendered.contains("● DETECTED"),
                "Ollama provider must not render DETECTED for {state:?}:\n{rendered}"
            );
        }
    }

    #[test]
    fn ollama_badge_present_after_probe_success() {
        let selected = PROVIDERS
            .iter()
            .position(|p| p.id == "ollama")
            .expect("ollama provider present");
        let area = Rect { x: 0, y: 0, width: 110, height: 30 };
        let mut buf = Buffer::empty(area);

        ProviderScreen {
            selected,
            ollama_detected: Some(true),
        }
        .render(area, &mut buf);

        let rendered = buffer_text(&buf);
        assert!(
            rendered.contains("● DETECTED"),
            "Ollama provider should render DETECTED after probe success:\n{rendered}"
        );
    }

    /// Render-fidelity gate for #250 (detail-over-card bleed). The detail panel
    /// writes via direct `buf`/`set_line` without padding to full region width,
    /// and its content length varies per provider. Without a per-region clear,
    /// stale glyphs from a longer prior selection survived in the lower rows
    /// when switching to a provider with a shorter panel. The per-region
    /// `theme::BG` repaint must make a re-render onto a dirty buffer
    /// byte-identical to a fresh render — i.e. zero residue.
    #[test]
    fn detail_switch_leaves_no_stale_glyph() {
        let n = PROVIDERS.len();
        let area = Rect { x: 0, y: 0, width: 110, height: 30 };

        // Find the provider pair with the largest detail-length delta by name
        // length as a proxy (longer name + config text → taller/wider panel).
        // We brute-force every ordered pair: for each (a, b), render a then b
        // onto ONE buffer, and assert it equals a fresh render of b.
        for a in 0..n {
            for b in 0..n {
                if a == b {
                    continue;
                }
                // Dirty buffer: render A, then B on top (no manual clear — the
                // screen's own per-region bg repaint is what must clean it).
                let mut dirty = Buffer::empty(area);
                ProviderScreen { selected: a, ollama_detected: None }.render(area, &mut dirty);
                ProviderScreen { selected: b, ollama_detected: None }.render(area, &mut dirty);

                // Reference: fresh buffer, render B once.
                let mut fresh = Buffer::empty(area);
                ProviderScreen { selected: b, ollama_detected: None }.render(area, &mut fresh);

                assert_eq!(
                    buffer_text(&dirty),
                    buffer_text(&fresh),
                    "stale glyph survived switching detail panel {a} -> {b} \
                     (provider {:?} -> {:?}) — per-region clear failed",
                    PROVIDERS[a].name,
                    PROVIDERS[b].name,
                );
            }
        }
    }

    /// #296 Provider fidelity: the 100-column render gate should not squeeze
    /// list + detail + hints into clipped columns. Keep the prototype's HINTS
    /// rail for wider layouts, but drop it at 100 cols so the selected-provider
    /// detail panel stays readable.
    #[test]
    fn provider_100_col_layout_drops_hints_for_detail_readability() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 30,
        };
        let mut buf = Buffer::empty(area);
        ProviderScreen { selected: 0, ollama_detected: None }.render(area, &mut buf);
        let out = buffer_text(&buf);

        assert!(
            !out.contains("HINTS"),
            "100-column Provider render must reserve width for detail, not the right HINTS rail"
        );
        assert!(
            out.contains("FLAGSHIP"),
            "100-column Provider render should still show selected-provider detail metadata"
        );
        assert!(
            out.contains(PROVIDERS[0].flagship),
            "100-column Provider render should keep flagship detail readable"
        );
    }

    /// Narrow-width path: the right HINTS column is dropped (`right_w = 0`).
    /// Switching selection must still leave no residue in the two live regions.
    #[test]
    fn detail_switch_no_residue_when_hints_column_dropped() {
        // width < 88 drops the right column.
        let area = Rect { x: 0, y: 0, width: 80, height: 26 };
        let mut dirty = Buffer::empty(area);
        ProviderScreen { selected: 0, ollama_detected: None }.render(area, &mut dirty);
        ProviderScreen { selected: PROVIDERS.len() - 1, ollama_detected: None }.render(area, &mut dirty);

        let mut fresh = Buffer::empty(area);
        ProviderScreen { selected: PROVIDERS.len() - 1, ollama_detected: None }.render(area, &mut fresh);

        assert_eq!(buffer_text(&dirty), buffer_text(&fresh),
            "stale glyph survived narrow-width detail switch — per-region clear failed");
    }

    /// Render the text of just the left card-list column (first `LEFT_W` cols).
    fn card_column_text(buf: &Buffer, left_w: u16) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..left_w.min(area.width) {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Render the text of the center detail panel (cols after the 40-wide list).
    fn detail_column_text(buf: &Buffer, left_w: u16) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in left_w.min(area.width)..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// #271 visual-parity gate (render-verified, NOT token-match).
    /// Slim cards must carry ONLY `[GLY] Name + 1-line desc` — model/pricing/key
    /// belong in the center detail panel. Asserts the selected provider's
    /// PRICE + FLAGSHIP strings render in the detail column but are ABSENT from
    /// the card-list column (no per-card meta cram → no truncation).
    #[test]
    fn slim_cards_move_model_pricing_to_detail_panel() {
        const LEFT_W: u16 = 40; // matches Constraint::Length(40) in render()
        // Wide enough that the right HINTS column is present (≥88) — the full
        // three-region layout the proto targets.
        let area = Rect { x: 0, y: 0, width: 110, height: 30 };

        // Pick a provider whose price/flagship are distinctive substrings.
        let sel = 0usize;
        let provider = &PROVIDERS[sel];

        let mut buf = Buffer::empty(area);
        ProviderScreen { selected: sel, ollama_detected: None }.render(area, &mut buf);

        let cards = card_column_text(&buf, LEFT_W);
        let detail = detail_column_text(&buf, LEFT_W);

        // The card column shows the provider NAME (slim card identity).
        assert!(
            cards.contains(provider.name),
            "slim card must still show the provider name in the list column"
        );

        // Model/pricing/key live ONLY in the detail panel, never crammed in the card.
        assert!(
            detail.contains(provider.price),
            "detail panel must render the provider price (got none): {detail}"
        );
        assert!(
            detail.contains(provider.flagship),
            "detail panel must render the provider flagship model"
        );
        assert!(
            !cards.contains(provider.price),
            "slim card MUST NOT cram pricing into the narrow list box (#271 regression): {cards}"
        );
        assert!(
            !cards.contains(provider.flagship),
            "slim card MUST NOT cram the flagship model into the narrow list box (#271 regression): {cards}"
        );
    }
}
