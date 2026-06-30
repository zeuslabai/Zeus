//! Render/navigation fidelity guard for the Production TUI Settings tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`SettingsTab`, JSX 1253–1375).
//! Guards the prototype two-pane subsystem editor and the #293 regression where
//! ↓ did not move Settings selection.

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use zeus_tui::App;
use zeus_tui::prod::{SettingsSection, SettingsTab};

fn render_settings(widget: SettingsTab<'_>) -> (Buffer, String) {
    let backend = TestBackend::new(120, 34);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| widget.render(f.area(), f.buffer_mut()))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    (buf.clone(), dump_buffer(&buf))
}

fn dump_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

#[test]
fn settings_tab_matches_prototype_groups_and_fields() {
    let (_buf, dump) = render_settings(SettingsTab::new());
    println!("{dump}");

    for expected in [
        "SUBSYSTEM",
        "◇  LLM",
        "⇌  Channels",
        "▤  Memory",
        "🛡  Security",
        "⚙  Tools",
        "▦  Display",
        "⊕  System",
        "LLM",
        "5 settings",
        "Provider",
        "anthropic",
        "Primary LLM provider",
        "Model",
        "claude-opus-4-7",
        "changes save on Enter · Esc to discard",
        "EDIT",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }
}

#[test]
fn settings_tab_renders_selected_group_fields() {
    let (_buf, dump) = render_settings(SettingsTab::new().with_active(SettingsSection::Memory));

    for expected in [
        "Memory",
        "5 settings",
        "DB path",
        "~/.zeus/mnemosyne.db",
        "Embedding provider",
        "ollama",
        "FTS enabled",
        "Auto-prune",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }
    assert!(dump.contains("●"), "dirty marker missing:\n{dump}");
}

#[test]
fn settings_tab_overlays_live_config_values() {
    let live = serde_json::json!({
        "llm": {
            "provider": "openai",
            "model": "gpt-4.1",
            "temperature": 0.2,
            "max_iterations": 80
        }
    });
    let (_buf, dump) = render_settings(SettingsTab::with_config(Some(&live)));

    for expected in [
        "Provider",
        "openai",
        "Model",
        "gpt-4.1",
        "Temperature",
        "0.2",
        "Max iterations",
        "80",
    ] {
        assert!(
            dump.contains(expected),
            "missing live value {expected:?}:\n{dump}"
        );
    }
}

#[test]
fn settings_arrow_navigation_moves_selection_regression_293() {
    let mut app = App::new();
    app.onboarding_complete = true;

    // BackTab from chat wraps to advanced, then settings without getting
    // caught by Office's Tab focus cycling.
    app.handle_key(KeyCode::BackTab);
    app.handle_key(KeyCode::BackTab);

    assert_eq!(app.prod_settings_section, SettingsSection::Llm);
    app.handle_key(KeyCode::Down);
    assert_eq!(app.prod_settings_section, SettingsSection::Channels);
    app.handle_key(KeyCode::Down);
    assert_eq!(app.prod_settings_section, SettingsSection::Memory);
    app.handle_key(KeyCode::Up);
    assert_eq!(app.prod_settings_section, SettingsSection::Channels);
}
