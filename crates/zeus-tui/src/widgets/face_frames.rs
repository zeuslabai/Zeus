//! ZeusFace animation frame data — ported verbatim from the production prototype
//! (`docs/zeus-tui-onboarding.jsx` lines 23-39, `FACE_FRAMES` + `FACE_COLORS`).
//!
//! This is **pure render-data**: the frame arrays and their per-state colors, plus
//! a frame-selector that maps a monotonic animation counter onto the current glyph.
//! It owns no loop/tick infrastructure — the `anim_tick: u64` counter is supplied
//! by the run loop (106's tick primitive). Callers do:
//!
//! ```ignore
//! let (glyph, color) = face_frames::frame(FaceState::Ready, app.anim_tick);
//! ```
//!
//! Until the tick seam lands, passing a constant `0` yields the resting frame —
//! identical to today's hardcoded static glyph, so this is a safe no-visual-change
//! port that becomes animated the moment a real counter is wired in.

use crate::theme;
use ratatui::style::Color;

/// The ZeusFace emotional states, matching the JSX `FACE_FRAMES` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceState {
    Ready,
    Listening,
    Thinking,
    Working,
    Tool,
    Success,
    Error,
    Alert,
    Queued,
    Sleeping,
}

impl FaceState {
    /// The animation frames for this state — ported verbatim from JSX `FACE_FRAMES`.
    pub fn frames(self) -> &'static [&'static str] {
        match self {
            FaceState::Ready => &[
                "(◉‿◉)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)", "(-‿-)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)",
            ],
            FaceState::Listening => &["(◉_◉)", "(◉_◉)", "(-_◉)", "(◉_◉)"],
            FaceState::Thinking => &[
                "(◉.◉)", "(◔.◉)", "(◔.◔)", "(◉.◔)", "(◉.◉)", "(◔ ◔)", "(- -)", "(◉.◉)",
            ],
            FaceState::Working => &[
                "(◣_◢)", "(◢_◣)", "(◣_◢)", "(◢_◣)", "(▰_▰)", "(◣_◢)", "(◢_◣)", "(▰_▰)",
            ],
            FaceState::Tool => &[
                "[◉_◉]", "[◉.◉]", "[◉_◉]", "[◉.◉]", "[●_●]", "[◉_◉]", "[◉.◉]", "[◉_◉]",
            ],
            FaceState::Success => &["(◉‿◉)✓", "(^‿^)✓", "(◉‿◉)✓", "(^‿^)✓"],
            FaceState::Error => &[
                "(✕_✕)", "(✕.✕)", "(✕_✕)", "(>_<)", "(✕_✕)", "(>_<)", "(✕_✕)", "(✕.✕)",
            ],
            FaceState::Alert => &["(◉ω◉)!", "(◉ω◉)!", "(◉_◉)!", "(◉ω◉)!"],
            FaceState::Queued => &["(◔‿◉)", "(◉‿◔)", "(◔‿◉)", "(◉‿◔)"],
            FaceState::Sleeping => &["(-_-) z", "(-_-) zZ", "(-_-) zZz", "(-_-) zZ"],
        }
    }

    /// The accent color for this state — ported from JSX `FACE_COLORS` (lines 35-39),
    /// mapped onto the canonical theme palette (the theme constants carry the same
    /// hex values as the JSX `C` palette, verified 1:1).
    pub fn color(self) -> Color {
        match self {
            FaceState::Ready => theme::FIRE_ORANGE, // #ff3c14
            FaceState::Listening => theme::CYAN,    // #06b6d4
            FaceState::Thinking => theme::AMBER,    // #ffa050
            FaceState::Working => theme::AMBER,     // #ffa050
            FaceState::Tool => theme::CYAN,         // #06b6d4
            FaceState::Success => theme::GREEN,     // #22c55e
            FaceState::Error => theme::RED,         // #ef4444
            FaceState::Alert => theme::YELLOW,      // #eab308
            FaceState::Queued => theme::AMBER,      // #ffa050
            FaceState::Sleeping => theme::DIM,      // #5a5650
        }
    }

    /// The resting frame (index 0) — used when no animation counter is available yet.
    pub fn resting(self) -> &'static str {
        self.frames()[0]
    }
}

/// Select the current glyph + color for a state, given a monotonic animation counter.
///
/// `anim_frame` is the shared per-tick counter supplied by the run loop (106's
/// tick primitive). The glyph is `frames[anim_frame % frames.len()]`, matching the
/// JSX `setFrame(f => (f + 1) % frames.length)` cadence. Color is constant per state.
pub fn frame(state: FaceState, anim_frame: u64) -> (&'static str, Color) {
    let frames = state.frames();
    let idx = (anim_frame % frames.len() as u64) as usize;
    (frames[idx], state.color())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_at_least_one_frame() {
        for st in [
            FaceState::Ready,
            FaceState::Listening,
            FaceState::Thinking,
            FaceState::Working,
            FaceState::Tool,
            FaceState::Success,
            FaceState::Error,
            FaceState::Alert,
            FaceState::Queued,
            FaceState::Sleeping,
        ] {
            assert!(!st.frames().is_empty(), "{st:?} has no frames");
        }
    }

    #[test]
    fn resting_frame_matches_index_zero() {
        // Resting frame is what a no-tick caller (anim_frame=0) gets — must equal
        // the historical static glyph for the no-visual-change guarantee.
        assert_eq!(FaceState::Ready.resting(), "(◉‿◉)");
        assert_eq!(FaceState::Success.resting(), "(◉‿◉)✓");
        let (glyph, _) = frame(FaceState::Ready, 0);
        assert_eq!(glyph, FaceState::Ready.resting());
    }

    #[test]
    fn frame_selector_wraps_modulo_len() {
        // ready has 8 frames; frame 8 wraps to frame 0, frame 4 is the (-‿-) blink.
        let (g0, _) = frame(FaceState::Ready, 0);
        let (g8, _) = frame(FaceState::Ready, 8);
        assert_eq!(g0, g8);
        let (g4, _) = frame(FaceState::Ready, 4);
        assert_eq!(g4, "(-‿-)");
    }

    #[test]
    fn ready_color_is_fire_orange() {
        assert_eq!(FaceState::Ready.color(), theme::FIRE_ORANGE);
        assert_eq!(FaceState::Success.color(), theme::GREEN);
        assert_eq!(FaceState::Sleeping.color(), theme::DIM);
    }
}
