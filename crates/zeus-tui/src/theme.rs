#![allow(dead_code)]
//! Theme — warm-neutral palette, Tailwind status ramp + red-orange ember accent.
//! Owner: zeus-freebsd (feat/tui-chat-fidelity)
//! Reference: docs/prd/zeus-tui-production.jsx + zeus-tui-onboarding.jsx `C` object
//!            (both prototypes carry a byte-identical 18-token palette).
//!
//! DESIGN-FIDELITY GATE: every const below is the prototype hex, verbatim.
//! Do not "tune" these toward the old neon palette — that drift is the
//! "looks like main" bug. RGB here == hex in the JSX, token for token.

use ratatui::style::{Color, Modifier, Style};

// ── Backgrounds (cool-black → warm-neutral panels) ──
pub const BG: Color = Color::Rgb(10, 10, 15); // #0a0a0f
pub const BG_PANEL: Color = Color::Rgb(18, 16, 14); // #12100e (bg2)
pub const BG_HIGHLIGHT: Color = Color::Rgb(26, 22, 16); // #1a1610 (bg3)
pub const DARK: Color = Color::Rgb(42, 36, 32); // #2a2420
pub const BORDER: Color = Color::Rgb(58, 54, 50); // #3a3632 (muted)
pub const MUTED: Color = Color::Rgb(58, 54, 50); // #3a3632

// ── Text ──
pub const TEXT: Color = Color::Rgb(212, 207, 200); // #d4cfc8 (fg)
pub const TEXT_BRIGHT: Color = Color::Rgb(240, 236, 230); // #f0ece6 (white)
pub const WHITE: Color = Color::Rgb(240, 236, 230); // #f0ece6
pub const DIM: Color = Color::Rgb(90, 86, 80); // #5a5650

// ── Accent (red-orange ember, NOT pure orange) ──
/// Accent — red-leaning ember. Prototype `accent` #ff3c14.
pub const FIRE_ORANGE: Color = Color::Rgb(255, 60, 20); // #ff3c14
pub const ACCENT_DIM: Color = Color::Rgb(160, 48, 26); // #a0301a
pub const ACCENT_BRIGHT: Color = Color::Rgb(255, 104, 66); // #ff6842
pub const ACCENT_FAINT: Color = Color::Rgb(64, 16, 8); // #401008
/// accentSoft rgba(255,60,20,0.05) flattened over BG #0a0a0f — terminals can't
/// alpha-blend, so this is the pre-blended opaque solid (bg*0.95 + accent*0.05):
/// R 10*.95+255*.05=22, G 10*.95+60*.05=12, B 15*.95+20*.05=15 → #160c0f.
/// Flattened-approximation, by design (the only rgba token; accentFaint is a solid hex).
pub const ACCENT_SOFT: Color = Color::Rgb(22, 12, 15);

// ── Status ramp (Tailwind 500-series + warm amber) ──
pub const RED: Color = Color::Rgb(239, 68, 68); // #ef4444
pub const RED_DIM: Color = Color::Rgb(74, 26, 26); // #4a1a1a
pub const GREEN: Color = Color::Rgb(34, 197, 94); // #22c55e
pub const GREEN_DIM: Color = Color::Rgb(26, 74, 46); // #1a4a2e
pub const YELLOW: Color = Color::Rgb(234, 179, 8); // #eab308
pub const YELLOW_DIM: Color = Color::Rgb(107, 90, 16); // #6b5a10
pub const BLUE: Color = Color::Rgb(59, 130, 246); // #3b82f6
pub const BLUE_DIM: Color = Color::Rgb(26, 42, 74); // #1a2a4a
pub const CYAN: Color = Color::Rgb(6, 182, 212); // #06b6d4
pub const CYAN_DIM: Color = Color::Rgb(22, 78, 99); // #164e63
pub const AMBER: Color = Color::Rgb(255, 160, 80); // #ffa050
pub const AMBER_DIM: Color = Color::Rgb(90, 48, 16); // #5a3010
pub const PURPLE: Color = Color::Rgb(168, 85, 247); // #a855f7
pub const PURPLE_DIM: Color = Color::Rgb(58, 26, 74); // #3a1a4a

// ── Compatibility aliases (kept so existing `theme::` references don't break) ──
/// Was a near-black warm-bg select; map to the warm panel highlight.
pub const BG_SELECTED: Color = BG_HIGHLIGHT;
/// Legacy "teal" → prototype cyan.
pub const TEAL: Color = CYAN;
/// Legacy "warm yellow" → prototype amber.
pub const WARM_YELLOW: Color = AMBER;
/// Legacy red-soft → prototype red-dim.
pub const RED_SOFT: Color = RED_DIM;
/// Provider badge — neutral grey for non-Kimi providers.
/// Kimi family uses FIRE_ORANGE (house default) — see ui::provider_badge.
pub const PROVIDER_NEUTRAL: Color = Color::Rgb(140, 140, 140);

// ── Semantic aliases ──
pub const ACCENT: Color = FIRE_ORANGE;
pub const ERROR: Color = RED;
pub const SUCCESS: Color = GREEN;
pub const WARNING: Color = YELLOW;
pub const HIGHLIGHT: Color = PURPLE;
pub const LABEL: Color = DIM;

/// Theme struct used by tab renderers (approvals, etc.) for field-based color access.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub bg_panel: Color,
    pub bg_highlight: Color,
    pub border: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub amber: Color,
    pub purple: Color,
    pub dim: Color,
    pub text: Color,
    pub text_bright: Color,
    pub cyan: Color,
    pub muted: Color,
}

impl Theme {
    pub fn default_dark() -> Self {
        Self {
            bg: BG,
            bg_panel: BG_PANEL,
            bg_highlight: BG_HIGHLIGHT,
            border: BORDER,
            red: RED,
            green: GREEN,
            yellow: YELLOW,
            amber: AMBER,
            purple: PURPLE,
            dim: DIM,
            text: TEXT,
            text_bright: TEXT_BRIGHT,
            cyan: CYAN,
            muted: MUTED,
        }
    }
}

pub fn title() -> Style {
    Style::default()
        .fg(FIRE_ORANGE)
        .add_modifier(Modifier::BOLD)
}
pub fn label() -> Style {
    Style::default().fg(DIM)
}
pub fn text() -> Style {
    Style::default().fg(TEXT)
}
pub fn bright() -> Style {
    Style::default().fg(TEXT_BRIGHT)
}
pub fn accent() -> Style {
    Style::default().fg(FIRE_ORANGE)
}
pub fn success() -> Style {
    Style::default().fg(GREEN)
}
pub fn warning() -> Style {
    Style::default().fg(YELLOW)
}
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
pub fn border() -> Style {
    Style::default().fg(BORDER)
}
pub fn border_active() -> Style {
    Style::default().fg(FIRE_ORANGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_uses_fire_orange() {
        assert_eq!(accent().fg, Some(FIRE_ORANGE));
    }

    #[test]
    fn title_uses_fire_orange() {
        assert_eq!(title().fg, Some(FIRE_ORANGE));
    }

    /// Design-fidelity gate: accent is the prototype red-orange ember (#ff3c14),
    /// NOT the old pure-orange (#ff6600). Guards against neon-palette regression.
    #[test]
    fn accent_matches_prototype_hex() {
        assert_eq!(FIRE_ORANGE, Color::Rgb(255, 60, 20));
    }

    /// Status ramp matches Tailwind 500-series exactly (prototype `C`).
    #[test]
    fn status_ramp_matches_prototype() {
        assert_eq!(RED, Color::Rgb(239, 68, 68)); // #ef4444
        assert_eq!(GREEN, Color::Rgb(34, 197, 94)); // #22c55e
        assert_eq!(YELLOW, Color::Rgb(234, 179, 8)); // #eab308
        assert_eq!(BLUE, Color::Rgb(59, 130, 246)); // #3b82f6
        assert_eq!(CYAN, Color::Rgb(6, 182, 212)); // #06b6d4
        assert_eq!(PURPLE, Color::Rgb(168, 85, 247)); // #a855f7
    }
}
