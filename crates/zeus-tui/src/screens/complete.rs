use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme;

/// Truncate `text` to `max_w` DISPLAY columns (chars, never bytes — values
/// carry user-influenced multibyte ids/paths), appending `…` when clipped.
/// Mirrors the per-screen idiom in gateway/memory/fallback/chanconfig.
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
    let kept: String = text.chars().take(max_w - 1).collect();
    format!("{}…", kept)
}

// Theme aliases (JSX palette)
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

/// Status of a summary row — matches JSX status: "configured" | "skipped" | "error".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    Configured,
    Skipped,
    Error,
}

/// One row in the configuration summary stack — JSX CompleteStep summary prop
/// (docs/zeus-tui-onboarding.jsx:2150-2172).
pub struct SummaryRow {
    pub name: String,
    pub value: String,
    pub status: RowStatus,
}

/// Test-all button state — JSX testing/tested useState pair (jsx:1739-1745).
/// `Tested` = all real-checks passed; `Failed` = at least one backend check
/// failed (provider API-key invalid or gateway port unreachable). The real
/// checks run in `App` (which can reach auth/gateway state) and feed results
/// into [`CompleteScreen::run_test_all`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestState {
    Idle,
    Tested,
    Failed,
}

/// Result of one backend real-check — name + pass/fail + a short detail line
/// shown under the button when a check fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Step 18 (`id:"complete"`, top_bar `DONE`) — the review + finish screen.
/// Modeled on JSX CompleteStep (docs/zeus-tui-onboarding.jsx:1738).
/// Left: header + summary stack + [TEST ALL BACKENDS] [AWAKEN ZEUS].
/// Right (~40 cols, borderLeft): NEXT STEPS commands + SUMMARY SAVED box.
pub struct CompleteScreen {
    /// 14 summary rows built from App state on step entry.
    pub summary: Vec<SummaryRow>,
    /// Test-all button state. Enter runs real backend checks (provider API-key
    /// validation + gateway TCP reachability) and flips to Tested/Failed.
    pub test_state: TestState,
    /// Per-backend real-check results, populated by [`Self::run_test_all`].
    /// Empty until the first run; rendered under the button so failures are
    /// visible instead of silently swallowed.
    pub test_results: Vec<TestResult>,
    /// Focused button: 0 = TEST ALL BACKENDS, 1 = AWAKEN ZEUS.
    pub focused_button: usize,
    /// Set when AWAKEN's config/.env persist fails — surfaced on the screen so
    /// the user isn't dropped into a broken production UI with no explanation.
    /// `None` = no error (the happy path).
    pub persist_error: Option<String>,
    /// Monotonic animation counter mirrored from `App.anim_tick` (106's tick
    /// seam) on each render. Drives ZeusFace frame cycling.
    pub anim_tick: u64,
}

impl CompleteScreen {
    pub fn new() -> Self {
        Self {
            summary: Vec::new(),
            test_state: TestState::Idle,
            test_results: Vec::new(),
            focused_button: 0,
            persist_error: None,
            anim_tick: 0,
        }
    }

    /// Record a config/.env persist failure from the AWAKEN path. The render
    /// reads `persist_error` and shows it instead of silently failing.
    pub fn set_persist_error(&mut self, err: String) {
        self.persist_error = Some(err);
    }

    /// Enter on the test-all button — static v1 per scope approval: flips to
    /// Tested instantly (JSX runs a 2.2s timeout; no live round-trips here).
    /// Enter on the test-all button — runs real backend checks. `App` computes
    /// the per-backend results (it has access to provider/auth/gateway state)
    /// and passes them in. State flips to `Tested` only if every check passed,
    /// otherwise `Failed`. An empty result set is treated as `Failed` (nothing
    /// could be verified) so the green "all systems go" face is never shown on
    /// an unverified config.
    pub fn run_test_all(&mut self, results: Vec<TestResult>) {
        let all_passed = !results.is_empty() && results.iter().all(|r| r.passed);
        self.test_state = if all_passed {
            TestState::Tested
        } else {
            TestState::Failed
        };
        self.test_results = results;
    }

    /// Tab / arrow between the two buttons.
    pub fn focus_next_button(&mut self) {
        self.focused_button = (self.focused_button + 1) % 2;
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(40)])
            .split(area);

        self.render_left(cols[0], buf);
        self.render_right(cols[1], buf);
    }

    fn render_left(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };
        let bottom = inner.y + inner.height;
        let mut y = inner.y;

        // ── ZeusFace + divider — JSX 1751-1752 ──
        // Face now cycles on the shared anim_tick (106's tick seam). State
        // mirrors JSX: tested→Success, failed→Error, else→Ready. Glyph + color
        // come from FACE_FRAMES/FACE_COLORS via face_frame(state, anim_tick);
        // label italic dim. Failed maps to the Error face (red ✗) so a backend
        // real-check failure is visible in the face, not just the result rows.
        let (face_state, face_label) = match self.test_state {
            TestState::Tested => (crate::widgets::FaceState::Success, "all systems go"),
            TestState::Failed => (crate::widgets::FaceState::Error, "check failed"),
            TestState::Idle => (crate::widgets::FaceState::Ready, "ready to wake"),
        };
        let (face_glyph, face_color) = crate::widgets::face_frame(face_state, self.anim_tick);
        if y < bottom {
            let line = Line::from(vec![
                Span::styled(
                    face_glyph,
                    Style::default().fg(face_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    face_label,
                    Style::default()
                        .fg(face_color)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }
        // Vertical divider rule under the face — JSX `width:1 background:muted`
        // (the ZeusFace ⟷ header separator, rendered as a short horizontal rule
        // in the TUI's row-oriented layout).
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "─".repeat(inner.width.min(20) as usize),
                    Style::default().fg(theme::MUTED),
                )),
                inner.width,
            );
            y += 1;
        }

        // Header — JSX 1754-1755
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "✓ Configuration complete",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 1;
        }
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(vec![
                    Span::styled(
                        "Review your setup before launch. All settings persist to ",
                        Style::default().fg(theme::DIM),
                    ),
                    Span::styled(
                        "~/.zeus/config.toml",
                        Style::default().fg(theme::ACCENT_BRIGHT),
                    ),
                ]),
                inner.width,
            );
            y += 2;
        }

        // Summary stack — JSX 1760-1779
        let status_w: u16 = 14;
        for row in &self.summary {
            if y >= bottom {
                break;
            }
            // JSX 1766-1776: dot bg = green/muted/red; badge = ✓READY/⏭SKIPPED/✕ERROR.
            let (dot_color, status_text, status_color) = match row.status {
                RowStatus::Configured => (theme::GREEN, "✓ READY", theme::GREEN),
                RowStatus::Skipped => (theme::MUTED, "⏭ SKIPPED", theme::MUTED),
                RowStatus::Error => (theme::RED, "✕ ERROR", theme::RED),
            };
            let name_w: u16 = 18;
            let value_w = inner
                .width
                .saturating_sub(2 + 1 + name_w + 1 + status_w) as usize;

            // Truncate by DISPLAY width (chars), never bytes — `value` carries
            // user-influenced ids/paths that may be multibyte; a byte-based
            // `truncate()` on a non-char-boundary would panic.
            let mut value = row.value.clone();
            if value.chars().count() > value_w && value_w > 1 {
                value = value
                    .chars()
                    .take(value_w.saturating_sub(1))
                    .collect::<String>();
                value.push('…');
            }

            // Clamp the NAME to its column with ellipsis — an unclamped
            // `{:<name_w}` lets a long name spill the whole `set_line` budget,
            // chopping the value RAW and dropping the right-anchored status
            // badge (✓READY/⏭SKIPPED/✕ERROR) entirely — affordance-loss, not
            // just truncation. Clamp first so name → value → badge always fit.
            let name = clamp_ellipsis(&row.name, name_w as usize);
            let line = Line::from(vec![
                Span::styled("● ", Style::default().fg(dot_color)),
                Span::styled(
                    format!("{:<width$}", name, width = name_w as usize),
                    Style::default().fg(FG).add_modifier(Modifier::BOLD).bg(BG2),
                ),
                Span::styled(
                    format!(" {:<width$}", value, width = value_w),
                    Style::default().fg(theme::DIM).bg(BG2),
                ),
                Span::styled(
                    format!("{:>width$}", status_text, width = status_w as usize),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD)
                        .bg(BG2),
                ),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }
        y += 1;

        // Buttons — JSX 1782-1797
        if y < bottom {
            // Focus must be UNMISTAKABLE in every test state: the focused
            // button gets a bright SOLID fill (fg=BG on a bright bg) plus a
            // leading ▶ marker; the unfocused button drops the fill entirely
            // (dim/outline fg, no bg) and shows no marker. A filled-bg +
            // Modifier::REVERSED reads as no distinction at all, which is the
            // bug this replaces. Exactly one button reads as active.
            //
            // `accent` is the per-state base hue: orange (Idle), green
            // (Tested), yellow (Failed). The helper applies the same
            // focused/unfocused treatment uniformly so focus is visible even
            // after "✓ PASSED" / "✗ FAILED".
            let button_span = |label: &str, accent: Color, focused: bool| {
                if focused {
                    Span::styled(
                        format!(" ▶ {label} "),
                        Style::default()
                            .fg(theme::BG)
                            .bg(accent)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        format!("   {label} "),
                        Style::default().fg(theme::DIM),
                    )
                }
            };

            let test_label = match self.test_state {
                TestState::Idle => "TEST ALL BACKENDS",
                TestState::Tested => "✓ ALL BACKENDS PASSED",
                TestState::Failed => "✗ BACKEND CHECK FAILED",
            };
            let test_accent = match self.test_state {
                TestState::Idle => theme::ACCENT_BRIGHT,
                TestState::Tested => theme::GREEN,
                TestState::Failed => theme::YELLOW,
            };
            let test_focused = self.focused_button == 0;
            let awaken_focused = self.focused_button == 1;

            let line = Line::from(vec![
                button_span(test_label, test_accent, test_focused),
                Span::raw("  "),
                button_span("AWAKEN ZEUS", theme::ACCENT_BRIGHT, awaken_focused),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }

        // Persist failure — surfaced in red below the buttons so AWAKEN never
        // drops the user into a broken production UI with no explanation.
        if let Some(err) = &self.persist_error
            && y < bottom
        {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    format!("⚠ Save failed: {err}"),
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
        }
    }

    fn render_right(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // borderLeft: 1px solid muted — JSX 1800
        for dy in 0..area.height {
            buf.set_line(
                area.x,
                area.y + dy,
                &Line::from(Span::styled("│", Style::default().fg(theme::MUTED))),
                1,
            );
        }
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(3),
            height: area.height.saturating_sub(2),
        };
        let bottom = inner.y + inner.height;
        let mut y = inner.y;

        // NEXT STEPS — JSX 1801-1820
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "N E X T   S T E P S",
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 2;
        }
        let next_steps: &[(&str, &str)] = &[
            ("$ zeus start", "Launches gateway + agent loop"),
            ("$ zeus chat", "Interactive chat with your agent"),
            ("$ zeus pantheon", "Multi-agent coordination chat"),
            ("$ zeus onboard --resume", "Re-run wizard for skipped sections"),
        ];
        for (cmd, desc) in next_steps {
            if y >= bottom {
                break;
            }
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    *cmd,
                    Style::default().fg(theme::ACCENT_BRIGHT),
                )),
                inner.width,
            );
            y += 1;
            if y < bottom {
                // Clamp the desc to the panel interior — bare `set_line` hard
                // chops mid-word with no ellipsis at the fixed 40-col right
                // panel ("Launches ga", "Interactive"). Honest `…` instead.
                let desc_w = inner.width.saturating_sub(2) as usize;
                buf.set_line(
                    inner.x + 2,
                    y,
                    &Line::from(Span::styled(
                        clamp_ellipsis(desc, desc_w),
                        Style::default().fg(theme::DIM),
                    )),
                    inner.width.saturating_sub(2),
                );
                y += 1;
            }
        }
        y += 1;

        // SUMMARY SAVED box — JSX 1822-1826
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "SUMMARY SAVED",
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD)
                        .bg(BG2),
                )),
                inner.width,
            );
            y += 1;
        }
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "~/.zeus/onboarding-summary.md",
                    Style::default().fg(theme::ACCENT_BRIGHT).bg(BG2),
                )),
                inner.width,
            );
            y += 1;
        }
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "Diff against future runs.",
                    Style::default().fg(theme::DIM).bg(BG2),
                )),
                inner.width,
            );
        }
    }
}

impl Default for CompleteScreen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the screen at width `w` and return each buffer row as a String.
    fn render_rows(s: &CompleteScreen, w: u16) -> Vec<String> {
        let area = Rect::new(0, 0, w, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        s.render(area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn screen_with_long_summary() -> CompleteScreen {
        let mut s = CompleteScreen::new();
        s.summary = vec![SummaryRow {
            name: "LLM Provider with a very long descriptive name".into(),
            value: "anthropic/claude-opus-4-8-extended".into(),
            status: RowStatus::Configured,
        }];
        s
    }

    #[test]
    fn long_summary_name_clamps_with_ellipsis_value_and_badge_survive() {
        // At a NORMAL width the long-name row must: (a) clamp the name with an
        // ellipsis, (b) still render the value, (c) KEEP the ✓READY badge.
        // Pre-fix the unclamped `{:<18}` name spilled the line, chopping the
        // value raw and dropping the badge entirely (affordance loss).
        let s = screen_with_long_summary();
        let rows = render_rows(&s, 100);
        let row = rows
            .iter()
            .find(|r| r.contains("LLM Provider"))
            .expect("summary row must render");
        assert!(
            row.contains('…'),
            "long name must clamp with ellipsis, got: {row:?}"
        );
        assert!(
            row.contains("anthropic/"),
            "value must survive next to the clamped name, got: {row:?}"
        );
        assert!(
            row.contains("✓ READY"),
            "status badge must NOT be dropped by a long name, got: {row:?}"
        );
        assert!(
            !row.contains("descriptive name"),
            "name was not clamped, got: {row:?}"
        );
    }

    #[test]
    fn right_panel_desc_clamps_with_ellipsis_not_midword() {
        // The fixed 40-col right panel hard-chops the next-step desc lines
        // mid-word with bare set_line ("Launches ga"). They must clamp with
        // an ellipsis instead.
        let s = CompleteScreen::new();
        let rows = render_rows(&s, 56);
        let has_clamped_desc = rows.iter().any(|r| {
            if let Some(idx) = r.find('│') {
                let right = &r[idx..];
                right.contains("Launches") && right.contains('…')
            } else {
                false
            }
        });
        assert!(
            has_clamped_desc,
            "right-panel desc must clamp with ellipsis at narrow width; rows: {rows:?}"
        );
    }

    #[test]
    fn test_all_passes_when_all_checks_pass() {
        let mut s = CompleteScreen::new();
        assert_eq!(s.test_state, TestState::Idle);
        s.run_test_all(vec![
            TestResult { name: "Provider".into(), passed: true, detail: String::new() },
            TestResult { name: "Gateway".into(), passed: true, detail: String::new() },
        ]);
        assert_eq!(s.test_state, TestState::Tested);
    }

    #[test]
    fn test_all_fails_when_any_check_fails() {
        let mut s = CompleteScreen::new();
        s.run_test_all(vec![
            TestResult { name: "Provider".into(), passed: true, detail: String::new() },
            TestResult { name: "Gateway".into(), passed: false, detail: ":8080 unreachable".into() },
        ]);
        assert_eq!(s.test_state, TestState::Failed);
    }

    #[test]
    fn test_all_empty_results_is_failed() {
        // Nothing verifiable -> never show the green "all systems go" face.
        let mut s = CompleteScreen::new();
        s.run_test_all(vec![]);
        assert_eq!(s.test_state, TestState::Failed);
    }

    #[test]
    fn button_focus_cycles() {
        let mut s = CompleteScreen::new();
        assert_eq!(s.focused_button, 0);
        s.focus_next_button();
        assert_eq!(s.focused_button, 1);
        s.focus_next_button();
        assert_eq!(s.focused_button, 0);
    }

    #[test]
    fn render_smoke() {
        let mut s = CompleteScreen::new();
        s.summary = vec![
            SummaryRow {
                name: "LLM Provider".into(),
                value: "anthropic/claude-opus-4-7".into(),
                status: RowStatus::Configured,
            },
            SummaryRow {
                name: "Voice (TTS)".into(),
                value: "none".into(),
                status: RowStatus::Skipped,
            },
        ];
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        s.render(area, &mut buf);
    }

    #[test]
    fn persist_error_renders_warning() {
        let mut s = CompleteScreen::new();
        assert!(s.persist_error.is_none());
        s.set_persist_error("save config.toml: disk full".to_string());
        assert_eq!(
            s.persist_error.as_deref(),
            Some("save config.toml: disk full")
        );

        let area = Rect::new(0, 0, 120, 40);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        s.render(area, &mut buf);

        // The error must be surfaced on-screen (not swallowed) so AWAKEN never
        // drops the user into a broken production UI silently.
        let rendered: String = (0..area.height)
            .flat_map(|row| {
                (0..area.width).map(move |col| (col, row))
            })
            .map(|(col, row)| buf[(col, row)].symbol().to_string())
            .collect();
        assert!(
            rendered.contains("Save failed") && rendered.contains("disk full"),
            "persist error must render on the Complete screen"
        );
    }

    /// Render the Complete screen and return, for the button row, the column
    /// span of each `▶` focus marker and the set of background colors present
    /// on that row. The focus marker is the unmistakable per-state indicator;
    /// exactly one button carries it, and the focused button is the only one
    /// with a bright solid fill (a non-default background).
    fn render_button_row(state: TestState, focused_button: usize) -> (Vec<u16>, Vec<Color>) {
        let mut s = CompleteScreen::new();
        s.test_state = state;
        s.focused_button = focused_button;
        s.summary = vec![SummaryRow {
            name: "LLM Provider".into(),
            value: "anthropic/claude-opus-4-7".into(),
            status: RowStatus::Configured,
        }];

        let area = Rect::new(0, 0, 120, 40);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        s.render(area, &mut buf);

        // Locate the button row: the single row that contains a ▶ marker.
        let row = (0..area.height)
            .find(|&r| {
                (0..area.width).any(|c| buf[(c, r)].symbol() == "▶")
            })
            .expect("button row with a ▶ focus marker must render");

        let marker_cols: Vec<u16> = (0..area.width)
            .filter(|&c| buf[(c, row)].symbol() == "▶")
            .collect();
        let bgs: Vec<Color> = (0..area.width)
            .map(|c| buf[(c, row)].style().bg.unwrap_or(Color::Reset))
            .collect();
        (marker_cols, bgs)
    }

    #[test]
    fn focused_button_carries_distinct_marker_and_fill() {
        // For every test state, focus must be UNMISTAKABLE: exactly one ▶
        // marker on the row, and it moves with `focused_button`. The focused
        // button is the only one with a bright solid fill.
        for state in [TestState::Idle, TestState::Tested, TestState::Failed] {
            let (m0, bg0) = render_button_row(state, 0);
            let (m1, bg1) = render_button_row(state, 1);

            // Exactly one focus marker per render — never zero (the old bug:
            // Tested/Failed ignored focus) and never both buttons filled.
            assert_eq!(
                m0.len(),
                1,
                "{state:?}: focused-button==0 must render exactly one ▶ marker"
            );
            assert_eq!(
                m1.len(),
                1,
                "{state:?}: focused-button==1 must render exactly one ▶ marker"
            );

            // The marker — and thus the active button — must MOVE when focus
            // changes. If both renders put the marker at the same column there
            // is no visible focus distinction.
            assert_ne!(
                m0[0], m1[0],
                "{state:?}: the ▶ focus marker must move when focus shifts \
                 (focused 0 at col {}, focused 1 at col {})",
                m0[0], m1[0]
            );

            // The focused button is the ONLY one with a bright solid fill, so
            // the set of filled (non-Reset) cells differs between the two
            // focus positions — proving the fill follows focus, not a static
            // both-filled render (the screenshot bug).
            let fill0: Vec<usize> = (0..bg0.len()).filter(|&i| bg0[i] != Color::Reset).collect();
            let fill1: Vec<usize> = (0..bg1.len()).filter(|&i| bg1[i] != Color::Reset).collect();
            assert_ne!(
                fill0, fill1,
                "{state:?}: the solid fill must follow focus — both focus \
                 positions produced identical fills (no visible distinction)"
            );
        }
    }
}
