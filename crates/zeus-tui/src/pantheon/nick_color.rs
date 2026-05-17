//! Deterministic nick coloring for Pantheon.
//!
//! Every nick maps to a single stable ratatui `Color`. Known Zeus fleet
//! agents get hand-picked fixed colors for at-a-glance identification;
//! unknown nicks fall through to a hash-based palette that distributes
//! evenly across 12 distinct terminal colors.
//!
//! The same nick always produces the same color within a session and
//! across sessions — the mapping is purely a function of the nick string.

use ratatui::style::Color;

// ── Fixed agent colors ──────────────────────────────────────────────────────
//
// Zeus fleet agents have distinctive colors so the coordinator, workers,
// and the human principal are instantly distinguishable in a busy channel.

/// Return a fixed color for a known Zeus agent nick, or `None` to fall
/// through to the hash-based palette.
fn fixed_agent_color(nick: &str) -> Option<Color> {
    // Case-insensitive match — agents may connect with varying casing.
    match nick.to_lowercase().as_str() {
        "zeus100" => Some(Color::LightRed),    // coordinator — stands out
        "zeus106" => Some(Color::LightGreen),  // Mac Studio worker
        "zeus107" => Some(Color::LightCyan),   // Mac mini worker
        "zeusmolty" => Some(Color::LightMagenta), // M5 Max (this agent)
        "merakizzz" | "mewndude" => Some(Color::LightYellow), // human principal
        "assistant" => Some(Color::Yellow),     // Ollama/Gemma agent on M5 Max
        _ => None,
    }
}

// ── Hash-based palette ──────────────────────────────────────────────────────

/// 12-color palette for unknown nicks. Chosen for readability on dark
/// terminal backgrounds (no pure black/white, no dark gray).
const NICK_PALETTE: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightRed,
    Color::LightGreen,
    Color::LightBlue,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightYellow,
    Color::White,
];

/// DJB2-style hash — simple, fast, produces good distribution for short
/// ASCII strings (which nicks invariably are).
fn nick_hash(nick: &str) -> u64 {
    let mut hash: u64 = 5381;
    for byte in nick.to_lowercase().as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(*byte as u64);
    }
    hash
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Return a deterministic `Color` for the given nick.
///
/// Known Zeus fleet agents get a hand-picked fixed color. All other
/// nicks are hashed into a 12-color palette. The mapping is stable
/// across calls and sessions.
pub fn nick_color(nick: &str) -> Color {
    fixed_agent_color(nick).unwrap_or_else(|| {
        let idx = (nick_hash(nick) as usize) % NICK_PALETTE.len();
        NICK_PALETTE[idx]
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_agents_get_fixed_colors() {
        assert_eq!(nick_color("Zeus100"), Color::LightRed);
        assert_eq!(nick_color("zeus100"), Color::LightRed); // case-insensitive
        assert_eq!(nick_color("zeus106"), Color::LightGreen);
        assert_eq!(nick_color("zeus107"), Color::LightCyan);
        assert_eq!(nick_color("ZeusMolty"), Color::LightMagenta);
        assert_eq!(nick_color("merakizzz"), Color::LightYellow);
        assert_eq!(nick_color("mewndude"), Color::LightYellow);
        assert_eq!(nick_color("ASSISTANT"), Color::Yellow);
    }

    #[test]
    fn test_unknown_nicks_use_hash_palette() {
        // Unknown nicks should not return any of the fixed agent colors
        // by coincidence (the fixed-color check runs first, so this is
        // really checking that the fallback path produces a palette color).
        let color = nick_color("random_user_42");
        assert!(NICK_PALETTE.contains(&color));
    }

    #[test]
    fn test_nick_color_is_deterministic() {
        let c1 = nick_color("some_user");
        let c2 = nick_color("some_user");
        assert_eq!(c1, c2, "same nick must always produce the same color");
    }

    #[test]
    fn test_nick_color_case_insensitive_for_unknown() {
        assert_eq!(
            nick_color("FooBar"),
            nick_color("foobar"),
            "hash-based coloring should be case-insensitive"
        );
    }

    #[test]
    fn test_different_nicks_can_get_different_colors() {
        // Not guaranteed for any two specific nicks (hash collisions exist),
        // but across a large enough sample, colors should vary.
        let colors: std::collections::HashSet<_> = (0..50)
            .map(|i| nick_color(&format!("user_{}", i)))
            .collect();
        assert!(
            colors.len() > 3,
            "50 different nicks should produce more than 3 distinct colors, got {}",
            colors.len()
        );
    }
}
