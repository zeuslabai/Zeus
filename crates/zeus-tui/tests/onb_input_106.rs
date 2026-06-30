//! Input-path regression guards for onboarding text-entry screens — zeus106.
//!
//! Substrate-walk (06/19) of `app.rs` `handle_key` confirmed every genuine
//! text-entry screen has SYMMETRIC `KeyCode::Char` → push + `KeyCode::Backspace`
//! → pop routing (Auth, ChannelConfig, Orchestration, Agent, Gateway, Workspace,
//! Voice, Images, Memory, Skills). The "most screens broken" report traced to
//! stale pre-render-v2 screenshots — same as the Voice + X/IRC refuted-stale
//! findings.
//!
//! BUT the two most-used text fields — **Auth** (the API-key entry) and
//! **Workspace** (the 3 path fields) — had ZERO end-to-end input coverage: no
//! test drove `Char` → state-mutation → `Backspace` → pop through the real
//! `handle_key` entry point. This file is that permanent guard, so if a future
//! cut breaks the input routing for these fields it fails loud.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files and
//! 112's cursor-render lane.

use crossterm::event::KeyCode;
use zeus_tui::App;

/// Onboarding step indices (mirror the `Step` enum discriminants).
const AUTH_STEP: usize = 3;
const WORKSPACE_STEP: usize = 10;

/// Type a string through the real key entry point, one `Char` event per byte.
fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyCode::Char(c));
    }
}

// ─────────────────────────── Auth (API key) ───────────────────────────

/// Char events on the Auth step append to the API-key buffer; Backspace pops.
/// Drives the real `handle_key` path, not the field directly.
#[test]
fn auth_char_input_appends_then_backspace_pops() {
    let mut app = App::new();
    app.current_step = AUTH_STEP;

    type_str(&mut app, "sk-ant-123");
    assert_eq!(app.auth_api_key(), "sk-ant-123", "chars must append to the key buffer");

    app.handle_key(KeyCode::Backspace);
    app.handle_key(KeyCode::Backspace);
    assert_eq!(app.auth_api_key(), "sk-ant-1", "backspace must pop the last char");
}

/// Backspace on an empty Auth buffer must NOT panic (pop on empty String).
#[test]
fn auth_backspace_on_empty_is_panic_free() {
    let mut app = App::new();
    app.current_step = AUTH_STEP;
    assert_eq!(app.auth_api_key(), "", "fresh key buffer is empty");
    app.handle_key(KeyCode::Backspace); // must not panic
    assert_eq!(app.auth_api_key(), "", "empty pop stays empty");
}

// ─────────────────────────── Workspace (paths) ───────────────────────────

/// Char events on the Workspace step type into the FOCUSED path field;
/// Backspace pops from that same field. Field 0 = workspace_path.
#[test]
fn workspace_field0_char_input_appends_then_backspace_pops() {
    let mut app = App::new();
    app.current_step = WORKSPACE_STEP;
    app.workspace_focused_field = 0;

    let before = app.workspace_path.clone();
    type_str(&mut app, "/tmp/x");
    assert_eq!(
        app.workspace_path,
        format!("{before}/tmp/x"),
        "chars must append to the focused (workspace) path field"
    );

    app.handle_key(KeyCode::Backspace);
    assert_eq!(
        app.workspace_path,
        format!("{before}/tmp/"),
        "backspace must pop from the focused path field"
    );
}

/// The focused-field index actually routes input — typing on field 1 mutates
/// sessions_path, leaving workspace_path untouched.
#[test]
fn workspace_focused_field_routes_input() {
    let mut app = App::new();
    app.current_step = WORKSPACE_STEP;

    let ws_before = app.workspace_path.clone();
    app.workspace_focused_field = 1;
    type_str(&mut app, "ABC");

    assert!(
        app.sessions_path.ends_with("ABC"),
        "input must land in the sessions field when focused (field 1)"
    );
    assert_eq!(
        app.workspace_path, ws_before,
        "the unfocused workspace field must be untouched"
    );
}
