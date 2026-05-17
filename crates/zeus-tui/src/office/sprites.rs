//! Agent sprite system — 10x14 pixel sprites ported from office-reference-design.jsx.
//!
//! Sprites match the JSX reference exactly: 10 wide × 14 tall, with walk animation
//! alternating arm position per frame (frame % 4 < 2 = arms out).

use ratatui::style::Color;

/// Sprite width in pixels.
pub const SPRITE_W: usize = 10;
/// Sprite height in pixels.
pub const SPRITE_H: usize = 14;

/// Full color palette for a single agent sprite (matches JSX CHARS entries).
#[derive(Clone, Debug)]
pub struct SpriteColors {
    pub hair:   Color,
    pub skin:   Color,
    pub eye:    Color,
    pub shirt:  Color,
    pub shirt2: Color,
    pub belt:   Color,
    pub pants:  Color,
    pub pants2: Color,
    pub shoe:   Color,
    pub badge:  Color,
}

/// Pre-defined palettes — ported directly from the JSX `CHARS` object.
pub fn fleet_palettes() -> Vec<(&'static str, SpriteColors)> {
    vec![
        // zeus / Zeus100 — red shirt, dark navy pants, red badge
        ("zeus100", SpriteColors {
            hair:   Color::Rgb(42, 18, 8),
            skin:   Color::Rgb(232, 200, 160),
            eye:    Color::Rgb(26, 26, 46),
            shirt:  Color::Rgb(160, 48, 24),
            shirt2: Color::Rgb(128, 32, 16),
            belt:   Color::Rgb(58, 42, 26),
            pants:  Color::Rgb(26, 26, 46),
            pants2: Color::Rgb(20, 20, 42),
            shoe:   Color::Rgb(26, 18, 8),
            badge:  Color::Rgb(255, 60, 20),
        }),
        // hermes — blue shirt, dark blue-grey pants, blue badge
        ("zeus106", SpriteColors {
            hair:   Color::Rgb(74, 56, 40),
            skin:   Color::Rgb(216, 184, 144),
            eye:    Color::Rgb(26, 48, 72),
            shirt:  Color::Rgb(42, 90, 138),
            shirt2: Color::Rgb(26, 74, 122),
            belt:   Color::Rgb(42, 42, 58),
            pants:  Color::Rgb(26, 42, 58),
            pants2: Color::Rgb(20, 36, 48),
            shoe:   Color::Rgb(26, 18, 8),
            badge:  Color::Rgb(59, 130, 246),
        }),
        // athena — green shirt, dark green pants, cyan badge
        ("zeus107", SpriteColors {
            hair:   Color::Rgb(106, 74, 42),
            skin:   Color::Rgb(224, 192, 160),
            eye:    Color::Rgb(45, 90, 40),
            shirt:  Color::Rgb(45, 104, 40),
            shirt2: Color::Rgb(30, 90, 26),
            belt:   Color::Rgb(58, 58, 26),
            pants:  Color::Rgb(26, 42, 26),
            pants2: Color::Rgb(20, 42, 20),
            shoe:   Color::Rgb(26, 18, 8),
            badge:  Color::Rgb(6, 182, 212),
        }),
        // prometheus — gold shirt, dark olive pants, yellow badge
        ("zeus112", SpriteColors {
            hair:   Color::Rgb(58, 40, 24),
            skin:   Color::Rgb(216, 176, 136),
            eye:    Color::Rgb(74, 58, 16),
            shirt:  Color::Rgb(106, 90, 16),
            shirt2: Color::Rgb(90, 74, 8),
            belt:   Color::Rgb(74, 58, 42),
            pants:  Color::Rgb(42, 42, 26),
            pants2: Color::Rgb(36, 36, 24),
            shoe:   Color::Rgb(26, 18, 8),
            badge:  Color::Rgb(234, 179, 8),
        }),
    ]
}

/// Default palette (red shirt, generic) used when no fleet entry matches.
pub fn default_colors() -> SpriteColors {
    SpriteColors {
        hair:   Color::Rgb(60, 40, 20),
        skin:   Color::Rgb(200, 170, 140),
        eye:    Color::Rgb(26, 26, 46),
        shirt:  Color::Rgb(120, 60, 30),
        shirt2: Color::Rgb(100, 45, 20),
        belt:   Color::Rgb(60, 45, 30),
        pants:  Color::Rgb(40, 40, 50),
        pants2: Color::Rgb(32, 32, 42),
        shoe:   Color::Rgb(26, 18, 8),
        badge:  Color::Rgb(160, 160, 160),
    }
}

/// Look up a palette by agent ID (case-insensitive partial match).
pub fn palette_for(agent_id: &str) -> SpriteColors {
    let lower = agent_id.to_lowercase();
    for (key, colors) in fleet_palettes() {
        if lower.contains(key) {
            return colors;
        }
    }
    default_colors()
}

/// Generate a 10×14 sprite pixel grid for the given colors and animation frame.
///
/// Walk cycle: `frame % 4 < 2` = arms extended (matches JSX `a = frame % 4 < 2`).
///
/// Layout (rows 0–13):
/// ```text
/// 0   _  _  _  H  H  H  H  _  _  _    hair dome
/// 1   _  _  H  H  H  H  H  H  _  _    hair brow
/// 2   _  H  H  H  H  H  H  H  H  _    hair sides
/// 3   _  H  sk sk sk sk sk sk  H  _    face top
/// 4   _  _  sk ey sk sk ey sk  _  _    eyes
/// 5   _  _  sk sk sk sk sk sk  _  _    face bottom
/// 6   _  _  _  sk sk sk sk  _  _  _    chin
/// 7   _  _  sh sh bg sh sh sh  _  _    shirt + badge
/// 8   _  A  sh s2 sh sh s2 sh  A' _    arms (animated)
/// 9   _  _  sh sh sh sh sh sh  _  _    shirt hem
/// 10  _  _  bl bl bl bl bl bl  _  _    belt
/// 11  _  _  pt pt  _  _  pt pt _  _    legs upper
/// 12  _  _  p2 p2  _  _  p2 p2 _  _   legs lower
/// 13  _  _  so  ?  _   ? so  _  _  _   shoes (alt per frame)
/// ```
pub fn make_sprite(colors: &SpriteColors, frame: u32) -> Vec<Vec<Option<Color>>> {
    let h  = Some(colors.hair);
    let sk = Some(colors.skin);
    let ey = Some(colors.eye);
    let sh = Some(colors.shirt);
    let s2 = Some(colors.shirt2);
    let bl = Some(colors.belt);
    let pt = Some(colors.pants);
    let p2 = Some(colors.pants2);
    let so = Some(colors.shoe);
    let bg = Some(colors.badge);
    let __ = None;

    // Walk cycle: arms out when frame % 4 < 2 (matches JSX `a = frame % 4 < 2`)
    let a = frame % 4 < 2;
    let arm_l: Option<Color> = if a { Some(colors.skin) } else { Some(colors.shirt) };
    let arm_r: Option<Color> = if a { None }              else { Some(colors.skin) };
    // shoe alternation: left shoe forward on even walk cycle
    let shoe_l = if a { so } else { __ };
    let shoe_r = if a { __ } else { so };

    vec![
        vec![__, __, __, h,  h,  h,  h,  __, __, __],  // 0  hair dome
        vec![__, __, h,  h,  h,  h,  h,  h,  __, __],  // 1  hair brow
        vec![__, h,  h,  h,  h,  h,  h,  h,  h,  __],  // 2  hair sides
        vec![__, h,  sk, sk, sk, sk, sk, sk, h,  __],  // 3  face top
        vec![__, __, sk, ey, sk, sk, ey, sk, __, __],  // 4  eyes
        vec![__, __, sk, sk, sk, sk, sk, sk, __, __],  // 5  face bottom
        vec![__, __, __, sk, sk, sk, sk, __, __, __],  // 6  chin
        vec![__, __, sh, sh, bg, sh, sh, sh, __, __],  // 7  shirt + badge
        vec![__, arm_l, sh, s2, sh, sh, s2, sh, arm_r, __],  // 8  arms
        vec![__, __, sh, sh, sh, sh, sh, sh, __, __],  // 9  shirt hem
        vec![__, __, bl, bl, bl, bl, bl, bl, __, __],  // 10 belt
        vec![__, __, pt, pt, __, __, pt, pt, __, __],  // 11 legs upper
        vec![__, __, p2, p2, __, __, p2, p2, __, __],  // 12 legs lower
        vec![__, __, so, shoe_l, __, shoe_r, so, __, __, __],  // 13 shoes
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_dimensions() {
        let c = default_colors();
        let s = make_sprite(&c, 0);
        assert_eq!(s.len(), SPRITE_H, "height should be {SPRITE_H}");
        assert!(s.iter().all(|r| r.len() == SPRITE_W), "all rows should be {SPRITE_W} wide");
    }

    #[test]
    fn walk_arm_alternates() {
        let c = default_colors();
        let f0 = make_sprite(&c, 0); // arms out
        let f2 = make_sprite(&c, 2); // arms in (frame%4 >= 2)
        // Row 8 col 1 is the left arm slot — should differ
        assert_ne!(f0[8][1], f2[8][1], "arm should alternate between walk phases");
    }

    #[test]
    fn zeus100_palette_lookup() {
        let c = palette_for("zeus100");
        assert_eq!(c.shirt, Color::Rgb(160, 48, 24));
        assert_eq!(c.badge, Color::Rgb(255, 60, 20));
    }

    #[test]
    fn zeus106_palette_lookup() {
        let c = palette_for("zeus106");
        assert_eq!(c.shirt, Color::Rgb(42, 90, 138));
        assert_eq!(c.badge, Color::Rgb(59, 130, 246));
    }

    #[test]
    fn zeus107_palette_lookup() {
        let c = palette_for("zeus107");
        assert_eq!(c.badge, Color::Rgb(6, 182, 212)); // cyan
    }

    #[test]
    fn unknown_agent_gets_default() {
        let c = palette_for("some-random-agent");
        assert_eq!(c.badge, default_colors().badge);
    }

    #[test]
    fn fleet_has_four_agents() {
        assert_eq!(fleet_palettes().len(), 4);
    }
}
