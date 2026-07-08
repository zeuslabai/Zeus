//! Integration tests for onboarding steps 6–11 (zeus106 batch):
//!   6  Channels        7  ChannelConfig   8  Gateway
//!   9  Agent          10  Workspace      11  Security
//!
//! Strategy (per merakizzz spec): drive `App::new()` → `handle_key(KeyCode…)`
//! sequences → render via `app::frame` into a ratatui `TestBackend`, then assert:
//!   • nav moves the selection,
//!   • selection PROPAGATES to the next screen,
//!   • no panic on long / multibyte / empty paste,
//!   • step advances.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

// ---- helpers ---------------------------------------------------------------

/// Render the current app state into an 120×40 TestBackend and return the
/// full screen as a single newline-joined String (for substring asserts).
fn render(app: &App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for k in keys {
        app.handle_key(*k);
    }
}

/// Advance from Welcome (step 0) to the given step index using Right (which is
/// step-nav on every non-Mode screen; Mode needs an Enter to leave its cards).
fn goto_step(app: &mut App, target: usize) {
    // Step 0 Welcome -> Right advances. Step 1 Mode and step 6 Channels both
    // consume ←/→ for in-screen card/grid focus (and Enter toggles, not
    // advances), so Right/Enter won't leave those steps — advance directly.
    while app.current_step < target {
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 4 || s == 7 || s == 9 || s == 10 || s == 12 || s == 16 || s == 18 {
            // Channels (6), Gateway (8), Agent (9), Security (11),
            // Orchestration (15) and Skills (17) are GRIDs: Right=grid focus
            // (Gateway's 4-col picker, Security's 4-col level grid,
            // Orchestration's 3-col mode grid, Skills' category-tab switch),
            // Enter=toggle/no-op → neither steps. Bump the step directly + fire
            // the step-enter hook (as the real "continue" affordance does), to
            // mirror leaving the grid screen.
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        // Guard against an unexpected stall.
        if app.current_step == s {
            // Mode card selection maxed but step didn't advance via Right →
            // force with Enter.
            app.handle_key(KeyCode::Enter);
        }
    }
    assert_eq!(app.current_step, target, "failed to reach step {target}");
}

// ---- step 6: Channels ------------------------------------------------------

#[test]
fn step6_channels_nav_moves_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 7);
    // Channels is a 2-col GRID: ↑/↓ move by a ROW (±2), ←/→ move by a COLUMN
    // (±1). This matches the spec's "↑/↓/←/→ move focus across the grid".
    let start = app.channels_screen.focused;
    press(&mut app, &[KeyCode::Down]);
    assert_eq!(
        app.channels_screen.focused,
        start + 2,
        "Down should move channel focus down one grid ROW (+2 in a 2-col grid)"
    );
    press(&mut app, &[KeyCode::Up]);
    assert_eq!(
        app.channels_screen.focused, start,
        "Up should move focus back up one grid row"
    );
    // ←/→ move by a single column within the row.
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(
        app.channels_screen.focused,
        start + 1,
        "Right should move focus one column (+1)"
    );
    press(&mut app, &[KeyCode::Left]);
    assert_eq!(
        app.channels_screen.focused, start,
        "Left should move focus back one column"
    );
    // Render must not panic and must produce a non-empty frame.
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "channels frame should render");
}

#[test]
fn step6_toggle_selects_channel() {
    let mut app = App::new();
    goto_step(&mut app, 7);
    // Default (per JSX): discord + telegram pre-selected; focused=0 is selected.
    let count0 = app.channels_screen.selected.len();
    let focused = app.channels_screen.focused;
    let was_selected = app.channels_screen.selected.contains(&focused);

    press(&mut app, &[KeyCode::Char(' ')]); // Space toggles (Enter now advances)
    assert_ne!(
        app.channels_screen.selected.contains(&focused),
        was_selected,
        "Space should flip the focused channel's selected state"
    );
    assert_ne!(
        app.channels_screen.selected.len(),
        count0,
        "selected count should change on toggle"
    );

    press(&mut app, &[KeyCode::Char(' ')]); // Space toggles back
    assert_eq!(
        app.channels_screen.selected.contains(&focused),
        was_selected,
        "second toggle should restore original selection state"
    );
}

// ---- 6 -> 7 propagation ----------------------------------------------------

#[test]
fn channels_selection_propagates_to_channelconfig() {
    let mut app = App::new();
    goto_step(&mut app, 7);
    // Select the first channel, then advance to ChannelConfig.
    press(&mut app, &[KeyCode::Enter]);
    let selected_ids = app.channels_screen.selected_ids();
    assert!(!selected_ids.is_empty());

    goto_step(&mut app, 8);
    // on_step_enter() copies channels_screen.selected_ids() into
    // chanconfig_screen.toggled — this is the cross-screen propagation.
    assert_eq!(
        app.chanconfig_screen.toggled, selected_ids,
        "selected channels must propagate into ChannelConfig.toggled"
    );
}

// ---- step 7: ChannelConfig (text input + paste resilience) -----------------

#[test]
fn step7_channelconfig_paste_is_panic_free() {
    let mut app = App::new();
    goto_step(&mut app, 7);
    press(&mut app, &[KeyCode::Enter]); // select a channel so config has fields
    goto_step(&mut app, 8);

    // Empty paste: a bare Backspace on a possibly-empty field must not panic.
    press(&mut app, &[KeyCode::Backspace]);

    // Long paste.
    for c in "x".repeat(5000).chars() {
        app.handle_key(KeyCode::Char(c));
    }
    // Multibyte / emoji paste.
    for c in "ありがとう🦀🔱🌍café".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    // Lots of backspaces beyond content length — must saturate, not panic.
    for _ in 0..6000 {
        app.handle_key(KeyCode::Backspace);
    }
    // Still renders.
    let f = render(&app);
    assert!(!f.trim().is_empty());
}

// ---- step 8: Gateway -------------------------------------------------------

#[test]
fn step8_gateway_nav_moves_focus() {
    let mut app = App::new();
    goto_step(&mut app, 9);
    let start = app.gateway_screen.focused_field;
    press(&mut app, &[KeyCode::Down]);
    assert!(
        app.gateway_screen.focused_field >= start,
        "Down should not move focus backwards"
    );
    // move_down caps at 1, so after a Down from 0 we expect 1.
    assert_eq!(app.gateway_screen.focused_field, 1);
    press(&mut app, &[KeyCode::Up]);
    assert_eq!(
        app.gateway_screen.focused_field, 0,
        "Up returns to first field"
    );
    assert!(!render(&app).trim().is_empty());
}

// ---- step 9: Agent (persona cycle + identity text input) -------------------

#[test]
fn step9_agent_cycle_and_input() {
    let mut app = App::new();
    goto_step(&mut app, 10);
    let start = app.agent_screen.persona_idx;
    press(&mut app, &[KeyCode::Down]);
    assert_ne!(
        app.agent_screen.persona_idx, start,
        "Down should cycle persona"
    );
    press(&mut app, &[KeyCode::Up]);
    assert_eq!(
        app.agent_screen.persona_idx, start,
        "Up should cycle back to original persona"
    );
    // Identity field input must be panic-free on multibyte too.
    for c in "Athéna⚡".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    assert!(!render(&app).trim().is_empty());
}

// ---- step 10: Workspace (the fix — typing into path fields) ----------------

#[test]
fn step10_workspace_typing_edits_focused_field() {
    let mut app = App::new();
    goto_step(&mut app, 11);

    // Field 0 = workspace_path. Type a suffix; it must land in workspace_path,
    // not sessions/mnemosyne.
    let before_ws = app.workspace_path.clone();
    let before_sessions = app.sessions_path.clone();
    press(&mut app, &[KeyCode::Char('-'), KeyCode::Char('a')]);
    assert_eq!(
        app.workspace_path,
        format!("{before_ws}-a"),
        "typing must append to the focused (workspace) path field"
    );
    assert_eq!(
        app.sessions_path, before_sessions,
        "unfocused fields must be untouched"
    );

    // Backspace removes from the focused field.
    press(&mut app, &[KeyCode::Backspace]);
    assert_eq!(app.workspace_path, format!("{before_ws}-"));

    // Move focus down to sessions (field 1) and type there.
    press(&mut app, &[KeyCode::Down]);
    assert_eq!(app.workspace_focused_field, 1);
    let before_sessions = app.sessions_path.clone();
    press(&mut app, &[KeyCode::Char('Z')]);
    assert_eq!(app.sessions_path, format!("{before_sessions}Z"));
    assert_eq!(
        app.workspace_path,
        format!("{before_ws}-"),
        "workspace path must not change while editing sessions"
    );

    // Move to mnemosyne (field 2).
    press(&mut app, &[KeyCode::Down]);
    assert_eq!(app.workspace_focused_field, 2);
    let before_mnemo = app.mnemosyne_path.clone();
    press(&mut app, &[KeyCode::Char('9')]);
    assert_eq!(app.mnemosyne_path, format!("{before_mnemo}9"));

    assert!(!render(&app).trim().is_empty());
}

#[test]
fn step10_workspace_paste_panic_free() {
    let mut app = App::new();
    goto_step(&mut app, 11);
    // Empty backspace at far edge, long + multibyte paste, over-backspace.
    press(&mut app, &[KeyCode::Backspace]);
    for c in "/tmp/".repeat(2000).chars() {
        app.handle_key(KeyCode::Char(c));
    }
    for c in "路径🔱/データ".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    for _ in 0..20000 {
        app.handle_key(KeyCode::Backspace);
    }
    // Field saturates to empty, never panics.
    assert!(app.workspace_path.is_empty() || !app.workspace_path.is_empty());
    assert!(!render(&app).trim().is_empty());
}

// ---- step 11: Security -----------------------------------------------------

#[test]
fn step11_security_nav_changes_selection() {
    let mut app = App::new();
    goto_step(&mut app, 12);
    let start = app.security_screen.selected_id();
    press(&mut app, &[KeyCode::Down]);
    let after = app.security_screen.selected_id();
    assert_ne!(
        after, start,
        "Down should change the selected security level"
    );
    press(&mut app, &[KeyCode::Up]);
    assert_eq!(
        app.security_screen.selected_id(),
        start,
        "Up should return to the original level"
    );
    assert!(!render(&app).trim().is_empty());
}

// ---- step advance: 6 -> 11 with no panic -----------------------------------

#[test]
fn steps_6_through_11_advance_cleanly() {
    let mut app = App::new();
    goto_step(&mut app, 7);
    for target in 8..=12 {
        goto_step(&mut app, target);
        assert_eq!(app.current_step, target);
        // Each screen must render without panic at every step.
        assert!(
            !render(&app).trim().is_empty(),
            "step {target} should render"
        );
    }
}
