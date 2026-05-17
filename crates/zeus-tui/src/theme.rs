#![allow(dead_code)]
//! Theme — Cyberpunk dark red color constants
//! Owner: zeus107 (feat/s68-tui-core)
//! Reference: theme_ref.rs

use ratatui::style::{Color, Modifier, Style};

// Cyberpunk dark red theme
pub const BG: Color = Color::Rgb(10, 0, 8);
pub const BG_PANEL: Color = Color::Rgb(16, 8, 16);
pub const BG_HIGHLIGHT: Color = Color::Rgb(26, 16, 32);
pub const BORDER: Color = Color::Rgb(42, 16, 32);
pub const RED: Color = Color::Rgb(255, 0, 60);
pub const RED_SOFT: Color = Color::Rgb(255, 107, 129);
pub const GREEN: Color = Color::Rgb(0, 255, 136);
pub const YELLOW: Color = Color::Rgb(255, 170, 0);
pub const PURPLE: Color = Color::Rgb(170, 0, 255);
pub const DIM: Color = Color::Rgb(90, 64, 96);
pub const TEXT: Color = Color::Rgb(160, 128, 144);
pub const TEXT_BRIGHT: Color = Color::Rgb(200, 180, 190);
pub const CYAN: Color = Color::Rgb(0, 210, 220);
pub const MUTED: Color = Color::Rgb(58, 32, 48);

// Semantic aliases
pub const ACCENT: Color = RED;
pub const ERROR: Color = RED;
pub const SUCCESS: Color = GREEN;
pub const WARNING: Color = YELLOW;
pub const HIGHLIGHT: Color = PURPLE;
pub const LABEL: Color = DIM;

pub fn title() -> Style { Style::default().fg(RED).add_modifier(Modifier::BOLD) }
pub fn label() -> Style { Style::default().fg(DIM) }
pub fn text() -> Style { Style::default().fg(TEXT) }
pub fn bright() -> Style { Style::default().fg(TEXT_BRIGHT) }
pub fn accent() -> Style { Style::default().fg(RED) }
pub fn success() -> Style { Style::default().fg(GREEN) }
pub fn warning() -> Style { Style::default().fg(YELLOW) }
pub fn muted() -> Style { Style::default().fg(MUTED) }
pub fn border() -> Style { Style::default().fg(BORDER) }
pub fn border_active() -> Style { Style::default().fg(RED) }
