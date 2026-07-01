//! Render/navigation fidelity guard for the Production TUI Settings tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`SettingsTab`, JSX 1253–1375).
//! Guards the prototype two-pane subsystem editor and the #293 regression where
//! ↓ did not move Settings selection.

use crossterm::event::KeyCode;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use zeus_tui::prod::{SettingsSection, SettingsTab};
use zeus_tui::App;

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
#[test]
fn settings_tab_uses_live_config_for_channels_memory_display_system() {
    let live = serde_json::json!({
        "model": "anthropic/claude-sonnet-5-20260701",
        "fallback_models": ["openai/gpt-4.1", "groq/llama-3.3-70b"],
        "max_iterations": 123,
        "ollama": { "temperature": 0.42 },
        "channels": {
            "discord": { "token": "[redacted]" },
            "telegram": null,
            "slack": { "token": "[redacted]" },
            "email": false,
            "matrix": { "homeserver": "https://matrix.example" }
        },
        "mnemosyne": {
            "db_path": "/tmp/live-mnemosyne.db",
            "embedding_model": "text-embedding-3-small",
            "enable_fts": false
        },
        "embedding_status": { "active_provider": "openai" },
        "session_maintenance": { "max_age_days": 17 },
        "tui": {
            "theme": "midnight",
            "accent_color": "electric-blue",
            "vim_mode": true,
            "high_contrast": true,
            "animations": false
        },
        "gateway": { "host": "0.0.0.0", "port": 9090 }
    });

    let (_buf, llm) = render_settings(SettingsTab::with_config(Some(&live)));
    for expected in [
        "anthropic",
        "anthropic/claude-sonnet-5-20260701",
        "0.42",
        "123",
        "openai/gpt-4.1, groq/llama-3.3-70b",
    ] {
        assert!(
            llm.contains(expected),
            "missing live LLM value {expected:?}:\n{llm}"
        );
    }

    let (_buf, channels) = render_settings(
        SettingsTab::with_config(Some(&live)).with_active(SettingsSection::Channels),
    );
    for expected in ["Discord", "✓ enabled", "Telegram", "○ disabled", "Matrix"] {
        assert!(
            channels.contains(expected),
            "missing live channel value {expected:?}:\n{channels}"
        );
    }

    let (_buf, memory) =
        render_settings(SettingsTab::with_config(Some(&live)).with_active(SettingsSection::Memory));
    for expected in [
        "/tmp/live-mnemosyne.db",
        "openai",
        "text-embedding-3-small",
        "○ false",
        "17 days",
    ] {
        assert!(
            memory.contains(expected),
            "missing live memory value {expected:?}:\n{memory}"
        );
    }

    let (_buf, display) = render_settings(
        SettingsTab::with_config(Some(&live)).with_active(SettingsSection::Display),
    );
    for expected in ["midnight", "electric-blue", "✓ true", "○ false"] {
        assert!(
            display.contains(expected),
            "missing live display value {expected:?}:\n{display}"
        );
    }

    let (_buf, system) =
        render_settings(SettingsTab::with_config(Some(&live)).with_active(SettingsSection::System));
    for expected in ["0.0.0.0:9090", env!("CARGO_PKG_VERSION")] {
        assert!(
            system.contains(expected),
            "missing live system value {expected:?}:\n{system}"
        );
    }
}
