#![allow(dead_code)]
//! Screen modules — currently unused (chat-first UI renders directly in ui.rs)
//! Kept as scaffolding for future screens.

use ratatui::Frame;
use ratatui::layout::Rect;
use crate::app::App;

/// Result of handling a key event
pub enum Action {
    Continue,
    SwitchTab(usize),
    Quit,
    SendMessage(String),
}

/// Screen trait for future use
pub mod settings;
pub mod settings_fields;

pub trait Screen {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App);
    fn handle_input(&self, key: crossterm::event::KeyEvent, app: &mut App) -> Action;
}
