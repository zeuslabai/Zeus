//! StepIndicator — the windowed top-progress rail.
//!
//! Matches the JSX `StepIndicator` (zeus-tui-onboarding.jsx line 226): a single
//! horizontal band that shows a *window* of the 19 onboarding steps centered on
//! the current step, with `···` ellipses collapsing the hidden runs and `›`
//! separators between adjacent visible steps.
//!
//! Windowing rule (matches JSX `.filter`): a step is visible iff
//! `|idx - current| <= 4 || idx == 0 || idx == LAST`. So the first + last steps
//! are pinned (orientation anchors) and a ±4 window rides the current step.
//!
//! Markers are **current-derivable** — no new App state (per the rail-first scope):
//!   - `idx <  current` → completed → `✓`  (accent on faint fill)
//!   - `idx == current` → current   → `NN` (bg on accent fill — the highlight)
//!   - `idx >  current` → future    → `NN` (muted, no fill)
//!
//! The `⏭` skipped marker from the JSX is intentionally deferred: App tracks no
//! live skip-set (RowStatus is computed only at the Complete screen), so a
//! skipped-vs-completed distinction would need new state. That's a follow-up.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;
use crate::widgets::top_bar::STEPS;

/// How far the visible window extends on each side of the current step.
const WINDOW_RADIUS: usize = 4;

/// A step's render classification, derived purely from `idx` vs `current`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Completed,
    Current,
    Future,
}

/// Classify a step index against the current step. Pure — drives both the glyph
/// and the styling, and is the unit-test seam for the marker logic.
fn classify(idx: usize, current: usize) -> Marker {
    if idx < current {
        Marker::Completed
    } else if idx == current {
        Marker::Current
    } else {
        Marker::Future
    }
}

/// Is `idx` inside the visible window? `|idx - current| <= RADIUS`, plus the
/// first and last steps are always pinned. Pure — the windowing test seam.
fn is_visible(idx: usize, current: usize, last: usize) -> bool {
    let dist = idx.abs_diff(current);
    dist <= WINDOW_RADIUS || idx == 0 || idx == last
}

/// Compute the ordered list of visible step indices for a given `current`.
/// Extracted so the windowing + ellipsis math is testable without a Buffer.
fn visible_indices(current: usize) -> Vec<usize> {
    if STEPS.is_empty() {
        return Vec::new();
    }
    let last = STEPS.len() - 1;
    (0..STEPS.len())
        .filter(|&i| is_visible(i, current, last))
        .collect()
}

/// The two-character marker text for a step (zero-padded number, or `✓`).
fn marker_text(idx: usize, marker: Marker) -> String {
    match marker {
        Marker::Completed => "✓".to_string(),
        Marker::Current | Marker::Future => format!("{:02}", idx + 1),
    }
}

/// StepIndicator widget — windowed progress rail. Pure render off `current`.
pub struct StepIndicator {
    pub current: usize,
}

impl Widget for StepIndicator {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || STEPS.is_empty() {
            return;
        }

        // Panel background fill (bg2 in the JSX), so the band reads as a distinct
        // chrome strip between the TopBar and the body.
        let fill_style = Style::default().bg(theme::BG_PANEL);
        for x in area.x..area.x + area.width {
            buf.set_style(Rect::new(x, area.y, 1, 1), fill_style);
        }

        let current = self.current.min(STEPS.len() - 1);
        let visible = visible_indices(current);

        let mut spans: Vec<Span> = Vec::new();
        let mut prev_idx: Option<usize> = None;

        for &idx in &visible {
            // `···` ellipsis when there's a hidden run between this and the prev
            // visible step; `›` separator when they're adjacent. Nothing before
            // the first visible step.
            if let Some(p) = prev_idx {
                if idx - p > 1 {
                    spans.push(Span::styled(
                        " ··· ",
                        Style::default().fg(theme::MUTED),
                    ));
                } else {
                    spans.push(Span::styled(
                        " › ",
                        Style::default().fg(theme::MUTED),
                    ));
                }
            }

            let marker = classify(idx, current);
            let (name, _code) = STEPS[idx];
            let mtext = marker_text(idx, marker);

            // Marker box — current is accent-filled (bg-on-accent highlight),
            // completed is accent-on-faint, future is muted.
            let box_style = match marker {
                Marker::Current => Style::default()
                    .bg(theme::ACCENT)
                    .fg(theme::BG)
                    .add_modifier(Modifier::BOLD),
                Marker::Completed => Style::default()
                    .bg(theme::ACCENT_FAINT)
                    .fg(theme::ACCENT),
                Marker::Future => Style::default().fg(theme::MUTED),
            };
            spans.push(Span::styled(format!(" {mtext} "), box_style));

            // Step name label beside the marker — current is bright/bold,
            // completed dim, future muted.
            let label_style = match marker {
                Marker::Current => Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
                Marker::Completed => Style::default().fg(theme::DIM),
                Marker::Future => Style::default().fg(theme::MUTED),
            };
            spans.push(Span::styled(format!(" {name}"), label_style));

            prev_idx = Some(idx);
        }

        let line = Line::from(spans);
        buf.set_line(area.x + 1, area.y, &line, area.width.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windowing_mid_shows_radius_plus_anchors() {
        // current=9 (mid): window 5..=13, plus pinned 0 and 18.
        let v = visible_indices(9);
        assert!(v.contains(&0), "first step pinned");
        assert!(v.contains(&18), "last step pinned");
        for i in 5..=13 {
            assert!(v.contains(&i), "step {i} in ±4 window");
        }
        assert!(!v.contains(&4), "step 4 outside window (dist 5)");
        assert!(!v.contains(&14), "step 14 outside window (dist 5)");
    }

    #[test]
    fn windowing_near_start_no_low_ellipsis() {
        // current=0: window 0..=4 + pinned last. 0 is both anchor and current.
        let v = visible_indices(0);
        assert_eq!(v[0], 0, "starts at 0");
        for i in 0..=4 {
            assert!(v.contains(&i), "step {i} visible near start");
        }
        assert!(v.contains(&18), "last pinned");
        assert!(!v.contains(&5), "step 5 outside (dist 5)");
    }

    #[test]
    fn windowing_near_end_no_high_ellipsis() {
        // current=18 (last): window 14..=18 + pinned first.
        let v = visible_indices(18);
        assert!(v.contains(&0), "first pinned");
        for i in 14..=18 {
            assert!(v.contains(&i), "step {i} visible near end");
        }
        assert!(!v.contains(&13), "step 13 outside (dist 5)");
    }

    #[test]
    fn ellipsis_gap_exists_when_window_detached_from_anchor() {
        // At current=9, between pinned 0 and window-start 5 there's a >1 gap
        // (0 -> 5), so an ellipsis is warranted; between 5 and 6 it's adjacent.
        let v = visible_indices(9);
        // find the position of 0 and the next visible
        let pos0 = v.iter().position(|&x| x == 0).unwrap();
        let next = v[pos0 + 1];
        assert!(next > 1, "gap after pinned-first triggers ellipsis (0 -> {next})");
    }

    #[test]
    fn classify_partitions_past_current_future() {
        assert_eq!(classify(3, 5), Marker::Completed, "past = completed");
        assert_eq!(classify(5, 5), Marker::Current, "self = current");
        assert_eq!(classify(7, 5), Marker::Future, "ahead = future");
    }

    #[test]
    fn marker_text_completed_is_check_others_padded() {
        assert_eq!(marker_text(3, Marker::Completed), "✓");
        assert_eq!(marker_text(0, Marker::Current), "01", "zero-padded 1-based");
        assert_eq!(marker_text(8, Marker::Future), "09");
        assert_eq!(marker_text(17, Marker::Future), "18");
    }

    #[test]
    fn renders_without_panic_across_all_steps() {
        // Smoke: every current value renders into a realistic-width buffer.
        for current in 0..STEPS.len() {
            let mut buf = Buffer::empty(Rect::new(0, 0, 120, 1));
            StepIndicator { current }.render(Rect::new(0, 0, 120, 1), &mut buf);
        }
    }

    #[test]
    fn zero_height_area_is_noop() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 0));
        StepIndicator { current: 5 }.render(Rect::new(0, 0, 120, 0), &mut buf);
        // No panic = pass.
    }
}
