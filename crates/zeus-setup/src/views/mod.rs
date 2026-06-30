//! TUI view modules

pub mod checklist;
pub mod deploy_table;
pub mod doctor;
pub mod main_menu;
pub mod progress;

/// View trait for rendering
pub trait View {
    fn render(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect);
}
