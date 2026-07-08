//! 1:1 behavior tests for onboarding Instance screen (INST).
//!
//! Phase 1 is intentionally UI-only/no-new-schema: default press-through keeps
//! using the current `~/.zeus`; named instances only preview the target path and
//! launch command until the later Sessions/Fleet slices add durable persistence.

use crossterm::event::KeyCode;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use zeus_tui::app::frame;
use zeus_tui::App;

const INSTANCE_STEP: usize = 2;

fn render_text(app: &App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn instance_step_renders_default_target_and_preview() {
    let mut app = App::new();
    app.current_step = INSTANCE_STEP;
    app.on_step_enter();

    let dump = render_text(&app);
    assert!(dump.contains("Choose instance"), "header should identify the screen:\n{dump}");
    assert!(dump.contains("Default instance"), "default card missing:\n{dump}");
    assert!(dump.contains("~/.zeus"), "default path missing:\n{dump}");
    assert!(dump.contains("Named instance"), "named card missing:\n{dump}");
    assert!(dump.contains("zeus gateway"), "launch preview missing:\n{dump}");
}

#[test]
fn instance_named_target_accepts_safe_name_and_keeps_step_local() {
    let mut app = App::new();
    app.current_step = INSTANCE_STEP;
    app.on_step_enter();

    app.handle_key(KeyCode::Char(' '));
    app.handle_key(KeyCode::Tab);
    for c in "alpha-1".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    app.handle_key(KeyCode::Char('!'));

    assert_eq!(app.current_step, INSTANCE_STEP, "typing must not advance the wizard");
    let dump = render_text(&app);
    assert!(dump.contains("~/.zeus/instances/alpha-1"), "named path preview missing:\n{dump}");
    assert!(dump.contains("zeus gateway --instance alpha-1"), "launch preview missing:\n{dump}");
    assert!(!dump.contains("alpha-1!"), "unsafe punctuation should be ignored:\n{dump}");
}
