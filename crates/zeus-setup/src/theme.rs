//! Theme definitions (self-contained copy for standalone builds)
//!
//! This is a standalone copy of zeus-tui's theme module so that zeus-setup
//! can build without pulling in the full agent/voice/channel dependency tree.

use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub highlight: Color,
    pub accent: Color,
    pub muted: Color,
    pub error: Color,
    pub success: Color,
    pub warning: Color,
    pub border: Color,
    pub title: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(22, 22, 30),
            fg: Color::Rgb(220, 220, 235),
            highlight: Color::Rgb(100, 150, 255),
            accent: Color::Rgb(180, 140, 255),
            muted: Color::Rgb(85, 85, 105),
            error: Color::Rgb(255, 90, 90),
            success: Color::Rgb(80, 220, 130),
            warning: Color::Rgb(255, 190, 80),
            border: Color::Rgb(55, 55, 75),
            title: Color::Rgb(140, 180, 255),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(250, 250, 252),
            fg: Color::Rgb(30, 30, 40),
            highlight: Color::Rgb(60, 100, 200),
            accent: Color::Rgb(120, 80, 200),
            muted: Color::Rgb(140, 140, 160),
            error: Color::Rgb(200, 60, 60),
            success: Color::Rgb(60, 160, 100),
            warning: Color::Rgb(200, 140, 60),
            border: Color::Rgb(200, 200, 210),
            title: Color::Rgb(40, 80, 180),
        }
    }

    pub fn solarized_dark() -> Self {
        Self {
            bg: Color::Rgb(0, 43, 54),
            fg: Color::Rgb(131, 148, 150),
            highlight: Color::Rgb(38, 139, 210),
            accent: Color::Rgb(108, 113, 196),
            muted: Color::Rgb(88, 110, 117),
            error: Color::Rgb(220, 50, 47),
            success: Color::Rgb(133, 153, 0),
            warning: Color::Rgb(181, 137, 0),
            border: Color::Rgb(0, 54, 66),
            title: Color::Rgb(42, 161, 152),
        }
    }

    pub fn monokai() -> Self {
        Self {
            bg: Color::Rgb(39, 40, 34),
            fg: Color::Rgb(248, 248, 242),
            highlight: Color::Rgb(102, 217, 239),
            accent: Color::Rgb(174, 129, 255),
            muted: Color::Rgb(117, 113, 94),
            error: Color::Rgb(249, 38, 114),
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(253, 151, 31),
            border: Color::Rgb(62, 63, 54),
            title: Color::Rgb(230, 219, 116),
        }
    }

    pub fn nord() -> Self {
        Self {
            bg: Color::Rgb(46, 52, 64),
            fg: Color::Rgb(216, 222, 233),
            highlight: Color::Rgb(136, 192, 208),
            accent: Color::Rgb(180, 142, 173),
            muted: Color::Rgb(76, 86, 106),
            error: Color::Rgb(191, 97, 106),
            success: Color::Rgb(163, 190, 140),
            warning: Color::Rgb(235, 203, 139),
            border: Color::Rgb(59, 66, 82),
            title: Color::Rgb(129, 161, 193),
        }
    }

    pub fn dracula() -> Self {
        Self {
            bg: Color::Rgb(40, 42, 54),
            fg: Color::Rgb(248, 248, 242),
            highlight: Color::Rgb(139, 233, 253),
            accent: Color::Rgb(189, 147, 249),
            muted: Color::Rgb(98, 114, 164),
            error: Color::Rgb(255, 85, 85),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            border: Color::Rgb(68, 71, 90),
            title: Color::Rgb(255, 121, 198),
        }
    }

    pub fn gruvbox() -> Self {
        Self {
            bg: Color::Rgb(40, 40, 40),
            fg: Color::Rgb(235, 219, 178),
            highlight: Color::Rgb(131, 165, 152),
            accent: Color::Rgb(211, 134, 155),
            muted: Color::Rgb(146, 131, 116),
            error: Color::Rgb(251, 73, 52),
            success: Color::Rgb(184, 187, 38),
            warning: Color::Rgb(250, 189, 47),
            border: Color::Rgb(80, 73, 69),
            title: Color::Rgb(254, 128, 25),
        }
    }

    pub fn catppuccin() -> Self {
        Self {
            bg: Color::Rgb(30, 30, 46),
            fg: Color::Rgb(205, 214, 244),
            highlight: Color::Rgb(137, 180, 250),
            accent: Color::Rgb(203, 166, 247),
            muted: Color::Rgb(108, 112, 134),
            error: Color::Rgb(243, 139, 168),
            success: Color::Rgb(166, 227, 161),
            warning: Color::Rgb(249, 226, 175),
            border: Color::Rgb(49, 50, 68),
            title: Color::Rgb(180, 190, 254),
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            bg: Color::Rgb(26, 27, 38),
            fg: Color::Rgb(169, 177, 214),
            highlight: Color::Rgb(125, 207, 255),
            accent: Color::Rgb(187, 154, 247),
            muted: Color::Rgb(84, 91, 112),
            error: Color::Rgb(247, 118, 142),
            success: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            border: Color::Rgb(41, 46, 66),
            title: Color::Rgb(122, 162, 247),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "solarized" => Self::solarized_dark(),
            "monokai" => Self::monokai(),
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            "gruvbox" => Self::gruvbox(),
            "catppuccin" => Self::catppuccin(),
            "tokyo-night" => Self::tokyo_night(),
            _ => Self::dark(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
