use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Block, Borders, Paragraph, Widget, Wrap};

use crate::theme;

/// Logo lines — matches LOGO array in JSX (line 63).
const LOGO: &[&str] = &[
    "██████╗ ███████╗██╗   ██╗███████╗",
    "╚════██╗██╔════╝██║   ██║██╔════╝",
    "  ███╔═╝█████╗  ██║   ██║███████╗",
    " ██╔══╝ ██╔══╝  ██║   ██║╚════██║",
    "███████╗███████╗╚██████╔╝███████║",
    "╚══════╝╚══════╝ ╚═════╝ ╚══════╝",
];

/// Logo line colors — matches LOGO_COLORS in JSX (line 71).
/// [accent, accent, accentBright, accentBright, accentDim, muted]
const LOGO_COLORS: &[ratatui::style::Color] = &[
    theme::FIRE_ORANGE,
    theme::FIRE_ORANGE,
    theme::ACCENT_BRIGHT,
    theme::ACCENT_BRIGHT,
    theme::ACCENT_DIM,
    theme::MUTED,
];

fn cell_width(text: &str) -> usize {
    text.chars().count()
}

fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    let len = cell_width(text);
    if len <= max_w {
        return text.to_string();
    }

    match max_w {
        0 => String::new(),
        1 => "…".to_string(),
        n => {
            let mut out: String = text.chars().take(n.saturating_sub(1)).collect();
            out.push('…');
            out
        }
    }
}

fn centered_x(area: Rect, width: u16) -> u16 {
    area.x + area.width.saturating_sub(width) / 2
}

fn fitted_width(area: Rect, desired: u16, min: u16) -> u16 {
    if area.width == 0 {
        return 0;
    }
    desired.min(area.width).max(min.min(area.width))
}

fn write_clamped(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    style: Style,
    max_w: u16,
) {
    let clamped = clamp_ellipsis(text, max_w as usize);
    buf.set_string(x, y, clamped, style);
}

/// Welcome screen — matches JSX WelcomeStep (line 446).
/// Pure display: logo + tagline + ZeusFace greeting + "Press Enter to begin".
pub struct WelcomeScreen {
    pub existing_config: bool,
    /// Monotonic animation counter from `App.anim_tick` (106's tick seam).
    /// Drives ZeusFace frame cycling: `face_frame(state, anim_tick)`.
    pub anim_tick: u64,
}

impl Widget for WelcomeScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Center the full welcome stack. Existing-config mode includes the
        // amber warning box, so it needs a taller target than the clean start.
        let content_height: u16 = if self.existing_config { 32 } else { 27 };
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(content_height.min(area.height)),
                Constraint::Fill(1),
            ])
            .split(area);

        let inner = v_chunks[1];

        // Logo
        let mut y = inner.y;
        for (i, line) in LOGO.iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            let color = LOGO_COLORS.get(i).copied().unwrap_or(theme::MUTED);
            let style = if i < 2 {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            // Center horizontally by DISPLAY width (chars), not byte length —
            // the block/box-drawing glyphs are multi-byte UTF-8, so line.len()
            // (bytes) differs per row and staggers the logo. All glyphs are
            // width-1 cells, so chars().count() is the correct display width.
            let logo = clamp_ellipsis(line, inner.width as usize);
            let x = centered_x(inner, cell_width(&logo) as u16);
            buf.set_string(x, y, logo, style);
            y += 1;
        }

        y += 1; // spacing

        // "OPERATING SYSTEM" subtitle
        if y >= area.bottom() {
            return;
        }

        let op_sys = clamp_ellipsis("O P E R A T I N G   S Y S T E M", inner.width as usize);
        let x = centered_x(inner, cell_width(&op_sys) as u16);
        buf.set_string(
            x,
            y,
            op_sys,
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;

        // Tagline
        if y >= area.bottom() {
            return;
        }

        let tagline = clamp_ellipsis("Autonomous AI agents on your hardware", inner.width as usize);
        let x = centered_x(inner, cell_width(&tagline) as u16);
        buf.set_string(x, y, tagline, Style::default().fg(theme::DIM));
        y += 2;

        // ZeusFace greeting box — face cycles on the shared anim_tick (Ready state).
        let greeting = "\"Let's wake the fleet. This won't take long.\"";
        let (face, _face_color) = crate::widgets::face_frame(
            crate::widgets::FaceState::Ready,
            self.anim_tick,
        );
        let box_content = format!("  {}  {}", face, greeting);
        let box_w = fitted_width(inner, cell_width(&box_content) as u16 + 2, 4);
        let x = centered_x(inner, box_w);
        let greeting_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .style(Style::default().bg(theme::BG_PANEL));
        let greeting_area = Rect::new(x, y, box_w, 3);
        if y + 3 > area.bottom() {
            return;
        }
        greeting_block.render(greeting_area, buf);
        write_clamped(
            buf,
            x + 1,
            y + 1,
            &box_content,
            Style::default().fg(theme::DIM),
            box_w.saturating_sub(2),
        );
        y += 4;

        // Existing-config box (JSX 462–469): amber box, "pre-populate" copy.
        if self.existing_config {
            let header = "↻ EXISTING CONFIG DETECTED";
            let body =
                "Welcome back. Re-running will pre-populate fields from your current config.";
            let desired_w = cell_width(body).max(cell_width(header)) as u16 + 4;
            let box_w = fitted_width(inner, desired_w, 4);
            let bx = centered_x(inner, box_w);
            let amber_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::AMBER))
                .style(Style::default().bg(theme::BG_PANEL));
            let amber_area = Rect::new(bx, y, box_w, 4);
            if y + 4 > area.bottom() {
                return;
            }
            amber_block.render(amber_area, buf);
            buf.set_string(
                bx + 2,
                y + 1,
                header,
                Style::default()
                    .fg(theme::AMBER)
                    .add_modifier(Modifier::BOLD),
            );
            // Body, clipped with a visible ellipsis at narrow widths.
            let interior = box_w.saturating_sub(4);
            write_clamped(buf, bx + 2, y + 2, body, Style::default().fg(theme::TEXT), interior);
            y += 5;
        }

        // INITIATE card (JSX 470–500): 480w → 60 cols here. Header row,
        // body blurb, 3 stat rows, footer.
        let card_w = fitted_width(inner, 60, 4);
        let cx = centered_x(inner, card_w);
        let card_h: u16 = 11;
        if y + card_h > area.bottom() {
            return;
        }

        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED));
        let card_area = Rect::new(cx, y, card_w, card_h);
        card_block.render(card_area, buf);

        // Header: ▸ INITIATE (left) + version meta (right)
        let hy = y + 1;
        let title = "▸ INITIATE";
        let title_w = cell_width(title) as u16;
        write_clamped(
            buf,
            cx + 2,
            hy,
            title,
            Style::default()
                .fg(theme::FIRE_ORANGE)
                .add_modifier(Modifier::BOLD),
            card_w.saturating_sub(4),
        );
        let meta = "v0.4.7 · 391,269 LOC · 365 tools";
        let meta_budget = card_w.saturating_sub(2 + title_w + 2 + 2);
        if meta_budget > 0 {
            let meta = clamp_ellipsis(meta, meta_budget as usize);
            let meta_x = cx + card_w.saturating_sub(2 + cell_width(&meta) as u16);
            buf.set_string(meta_x, hy, meta, Style::default().fg(theme::MUTED));
        }

        // Body blurb (wrapped to interior width).
        let blurb = "This wizard configures every system on your Zeus \
                     deployment. Skip optional sections — your fleet lands \
                     at a working baseline regardless.";
        let interior_w = card_w.saturating_sub(4);
        let blurb_para = Paragraph::new(blurb)
            .style(Style::default().fg(theme::TEXT))
            .wrap(Wrap { trim: true });
        let blurb_area = Rect::new(cx + 2, y + 3, interior_w, 3);
        blurb_para.render(blurb_area, buf);

        // 3 stat rows: label (left, accent) · value (right, dim)
        let stats: &[(&str, &str)] = &[
            ("19 STEPS", "10 required, 9 optional"),
            ("~5 MIN", "QuickStart path"),
            ("~25 MIN", "Full configuration"),
        ];
        for (i, (label, value)) in stats.iter().enumerate() {
            let sy = y + 6 + i as u16;
            let interior = card_w.saturating_sub(4);
            let label_w = cell_width(label) as u16;
            write_clamped(
                buf,
                cx + 2,
                sy,
                label,
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
                interior,
            );
            let value_budget = interior.saturating_sub(label_w + 2);
            if value_budget > 0 {
                let value = clamp_ellipsis(value, value_budget as usize);
                let vx = cx + card_w.saturating_sub(2 + cell_width(&value) as u16);
                buf.set_string(vx, sy, value, Style::default().fg(theme::DIM));
            }
        }

        // Footer: ↵ Continue · N Exit · (right) build <sha> · main
        let fy = y + card_h.saturating_sub(2);
        let footer = "↵ Continue   N Exit";
        let interior = card_w.saturating_sub(4);
        write_clamped(buf, cx + 2, fy, footer, Style::default().fg(theme::DIM), interior);
        // No build-SHA infra in this crate (no build.rs/vergen). Use GIT_HASH
        // if the build env happens to set it, else the JSX placeholder. Both
        // compile-safe — option_env! resolves at compile time, never errors.
        let sha = option_env!("GIT_HASH").unwrap_or("a1c4f29");
        let build = format!("build {} · main", sha);
        let build_budget = interior.saturating_sub(cell_width(footer) as u16 + 2);
        if build_budget > 0 {
            let build = clamp_ellipsis(&build, build_budget as usize);
            let build_x = cx + card_w.saturating_sub(2 + cell_width(&build) as u16);
            buf.set_string(build_x, fy, build, Style::default().fg(theme::MUTED));
        }
        y += card_h + 1;

        // "Press Enter to begin"
        if y >= area.bottom() {
            return;
        }

        let prompt = clamp_ellipsis("Press ↵ Enter to begin", inner.width as usize);
        let x = centered_x(inner, cell_width(&prompt) as u16);
        buf.set_string(x, y, prompt, Style::default().fg(theme::MUTED));
    }
}

#[cfg(test)]
mod tests {
    use super::WelcomeScreen;
    use ratatui::widgets::Widget;
    use ratatui::{backend::TestBackend, Terminal};

    /// Render the screen and scrape the buffer into one string for assertions.
    fn render_to_string(existing_config: bool) -> String {
        render_to_string_at(120, 44, existing_config)
    }

    fn render_to_string_at(width: u16, height: u16, existing_config: bool) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| {
            WelcomeScreen { existing_config, anim_tick: 0 }.render(f.area(), f.buffer_mut());
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

    fn assert_no_line_overflows(rendered: &str, width: usize) {
        for (idx, line) in rendered.lines().enumerate() {
            let cells = line.chars().count();
            assert!(cells <= width, "line {} overflows {width} cols ({cells}):
{rendered}", idx + 1);
        }
    }

    #[test]
    fn render_smoke_100x30_clean_does_not_panic() {
        let r = render_to_string_at(100, 30, false);
        assert_no_line_overflows(&r, 100);
    }

    #[test]
    fn render_smoke_100x30_existing_config_does_not_panic() {
        let r = render_to_string_at(100, 30, true);
        assert_no_line_overflows(&r, 100);
    }

    #[test]
    fn render_dump_narrow_width_clamps_welcome_regions() {
        let r = render_to_string_at(56, 36, true);
        assert_no_line_overflows(&r, 56);
        assert!(r.contains("██████╗ ███████╗██╗"), "missing logo in narrow dump:
{r}");
        assert!(r.contains("O P E R A T I N G   S Y S T E M"), "missing subtitle in narrow dump:
{r}");
        assert!(r.contains("Autonomous AI agents on your hardware"), "missing tagline in narrow dump:
{r}");
        assert!(r.contains("Let's wake the fleet"), "missing greeting in narrow dump:
{r}");
        assert!(r.contains("↻ EXISTING CONFIG DETECTED"), "missing existing-config header in narrow dump:
{r}");
        assert!(r.contains("Welcome back. Re-running will pre-populate fields f…"), "missing clamped existing-config copy in narrow dump:
{r}");
        assert!(r.contains("▸ INITIATE"), "missing initiate card in narrow dump:
{r}");
        assert!(r.contains("v0.4.7 · 391,269 LOC"), "missing clamped version meta in narrow dump:
{r}");
        assert!(r.contains("QuickStart path"), "missing stat value in narrow dump:
{r}");
        assert!(r.contains("Press ↵ Enter to begin"), "missing prompt in narrow dump:
{r}");
        assert!(r.contains('…'), "narrow dump should exercise clamp ellipsis:
{r}");
    }

    #[test]
    fn render_dump_normal_width_preserves_full_welcome_copy() {
        let r = render_to_string_at(100, 40, true);
        assert_no_line_overflows(&r, 100);
        assert!(r.contains("██████╗ ███████╗██╗   ██╗███████╗"), "missing full logo in normal dump:
{r}");
        assert!(r.contains("O P E R A T I N G   S Y S T E M"), "missing subtitle in normal dump:
{r}");
        assert!(r.contains("Autonomous AI agents on your hardware"), "missing tagline in normal dump:
{r}");
        assert!(r.contains("Let's wake the fleet. This won't take long."), "missing full greeting in normal dump:
{r}");
        assert!(r.contains("Welcome back. Re-running will pre-populate fields from your current config."), "missing full existing-config copy in normal dump:
{r}");
        assert!(r.contains("v0.4.7 · 391,269 LOC · 365 tools"), "missing full version meta in normal dump:
{r}");
        assert!(r.contains("10 required, 9 optional"), "missing stat detail in normal dump:
{r}");
        assert!(r.contains("build a1c4f29 · main") || r.contains("· main"), "missing build footer in normal dump:
{r}");
        assert!(r.contains("Press ↵ Enter to begin"), "missing prompt in normal dump:
{r}");
    }

    #[test]
    fn renders_without_panic_both_states() {
        let _ = render_to_string(false);
        let _ = render_to_string(true);
    }

    #[test]
    fn initiate_card_present() {
        // JSX 470–500: the INITIATE card header + version meta + stat rows.
        let r = render_to_string(false);
        assert!(r.contains("▸ INITIATE"), "missing INITIATE header:\n{r}");
        assert!(
            r.contains("v0.4.7 · 391,269 LOC · 365 tools"),
            "missing version meta row:\n{r}"
        );
        assert!(r.contains("19 STEPS"), "missing `19 STEPS` stat row:\n{r}");
        assert!(r.contains("~5 MIN"), "missing `~5 MIN` stat row:\n{r}");
        assert!(r.contains("~25 MIN"), "missing `~25 MIN` stat row:\n{r}");
        assert!(r.contains("· main"), "missing footer build line:\n{r}");
    }

    #[test]
    fn existing_config_amber_box_copy() {
        // JSX 462–469: amber box, "pre-populate" wording — NOT "overwrite".
        let r = render_to_string(true);
        assert!(
            r.contains("↻ EXISTING CONFIG DETECTED"),
            "missing amber-box header:\n{r}"
        );
        assert!(
            r.contains("pre-populate"),
            "existing-config copy must say `pre-populate`:\n{r}"
        );
        assert!(
            !r.contains("overwrite"),
            "stale `overwrite` copy must be gone:\n{r}"
        );
    }

    #[test]
    fn no_existing_config_hides_amber_box() {
        let r = render_to_string(false);
        assert!(
            !r.contains("EXISTING CONFIG DETECTED"),
            "amber box must not render when existing_config=false:\n{r}"
        );
    }
}
