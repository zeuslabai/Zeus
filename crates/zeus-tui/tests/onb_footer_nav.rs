//! Proof tests for the focusable footer NEXT/BACK buttons (merakizzz's refined
//! advance UX, A2 implementation): every screen has a Tab-reachable BACK button
//! (bottom-left → step_back) and NEXT button (bottom-right → advance_step), in
//! the focus order, with Enter/Space activating the focused one. Ctrl+N + ESC
//! stay as keyboard shortcuts.
//!
//! A2 model (App-owned, zero screen-method signature changes): App owns a Tab
//! cursor reset on every screen-enter. Tab walks the screen's own fields
//! (footer_field_count) then BACK → NEXT → wrap. Grid/multiselect screens with
//! no Tab-field (Channels/Voice-grid/Features/Mode) → Tab lands on the footer
//! immediately. The App Tab counter — not any screen's internal focused_field —
//! drives the footer highlight, so reachability is consistent.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use crossterm::event::{KeyCode, KeyModifiers};
use zeus_tui::App;

const MODE: usize = 1;
const CHANNELS: usize = 7;
const AGENT: usize = 10;
const VOICE: usize = 14;

fn tab(app: &mut App) {
    app.handle_key(KeyCode::Tab);
}
fn enter(app: &mut App) {
    app.handle_key(KeyCode::Enter);
}
fn space(app: &mut App) {
    app.handle_key(KeyCode::Char(' '));
}
fn ctrl_n(app: &mut App) {
    app.handle_key_mods(KeyCode::Char('n'), KeyModifiers::CONTROL);
}
fn goto(app: &mut App, step: usize) {
    app.current_step = step;
    app.on_step_enter();
    assert_eq!(app.current_step, step, "failed to seat at step {step}");
}

// ─── Channels (the wedge): Tab reaches footer, Tab→NEXT→Enter advances ───

#[test]
fn channels_tab_reaches_footer_then_next_enter_advances() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    // Channels is a grid (0 Tab-fields) → first Tab = BACK, second = NEXT.
    tab(&mut app);
    assert_eq!(
        app.footer_focus,
        Some(zeus_tui::FooterFocus::Back),
        "grid screen's first Tab should land on BACK"
    );
    tab(&mut app);
    assert_eq!(
        app.footer_focus,
        Some(zeus_tui::FooterFocus::Next),
        "second Tab should land on NEXT"
    );
    // Enter on focused NEXT advances the step (the reported wedge fix).
    enter(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Tab→NEXT→Enter must advance past the Channels wedge"
    );
}

// ─── Tab→BACK→Enter steps back ───

#[test]
fn channels_tab_back_enter_steps_back() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    tab(&mut app); // BACK focused
    assert_eq!(app.footer_focus, Some(zeus_tui::FooterFocus::Back));
    enter(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS - 1,
        "Tab→BACK→Enter must step back one"
    );
}

// ─── Space activates the focused footer button too ───

#[test]
fn space_activates_focused_next() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    tab(&mut app); // BACK
    tab(&mut app); // NEXT
    space(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Space on focused NEXT must advance"
    );
}

// ─── Voice (the other wedge): grid Tab reaches footer ───

#[test]
fn voice_tab_back_next_then_advance() {
    let mut app = App::new();
    goto(&mut app, VOICE);
    // Voice has config fields only when a non-None provider is picked; from a
    // fresh seat we walk Tab until the footer appears, then NEXT→Enter.
    let mut guard = 0;
    while app.footer_focus != Some(zeus_tui::FooterFocus::Next) {
        tab(&mut app);
        guard += 1;
        assert!(guard < 12, "Tab never reached NEXT on Voice");
    }
    enter(&mut app);
    assert_eq!(
        app.current_step,
        VOICE + 1,
        "Tab→NEXT→Enter must advance past the Voice strand"
    );
}

// ─── Field screen: Tab walks fields THEN footer (Agent = 3 fields) ───

#[test]
fn agent_tab_walks_fields_before_footer() {
    let mut app = App::new();
    goto(&mut app, AGENT);
    // Agent has 3 Tab-fields → Tabs 1..=3 stay on the screen (footer None),
    // Tab 4 = BACK, Tab 5 = NEXT, Tab 6 wraps back to the screen.
    for i in 1..=3 {
        tab(&mut app);
        assert_eq!(
            app.footer_focus, None,
            "Tab #{i} should still be walking Agent fields (footer not yet reached)"
        );
    }
    tab(&mut app);
    assert_eq!(
        app.footer_focus,
        Some(zeus_tui::FooterFocus::Back),
        "Tab #4 = BACK"
    );
    tab(&mut app);
    assert_eq!(
        app.footer_focus,
        Some(zeus_tui::FooterFocus::Next),
        "Tab #5 = NEXT"
    );
    tab(&mut app);
    assert_eq!(
        app.footer_focus, None,
        "Tab #6 wraps back to the screen fields"
    );
}

// ─── footer_focus resets on screen-enter (Zeus100's edge note) ───

#[test]
fn footer_focus_resets_on_step_enter() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    tab(&mut app); // BACK focused
    assert!(app.footer_focus.is_some());
    // Advancing (any path) re-enters a screen → cursor + focus reset.
    ctrl_n(&mut app);
    assert_eq!(
        app.footer_focus, None,
        "footer_focus must reset on screen-enter so Tab-reachability is consistent"
    );
    assert_eq!(app.tab_cursor, 0, "tab_cursor resets on step-enter");
}

// ─── Mode (horizontal-card grid) Tab reaches footer directly ───

#[test]
fn mode_grid_tab_reaches_footer() {
    let mut app = App::new();
    goto(&mut app, MODE);
    tab(&mut app);
    assert_eq!(
        app.footer_focus,
        Some(zeus_tui::FooterFocus::Back),
        "Mode (no Tab-field) → first Tab lands on the footer"
    );
}

// ─── Ctrl+N + ESC shortcuts still live alongside the buttons ───

#[test]
fn ctrl_n_and_esc_shortcuts_still_work() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    ctrl_n(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Ctrl+N shortcut still advances"
    );
    app.handle_key(KeyCode::Esc);
    assert_eq!(app.current_step, CHANNELS, "ESC shortcut still steps back");
}

// ─── merakizzz nav-UX fix (supersedes old per-screen-toggle behavior): on
//     grid/multiselect screens, Enter ADVANCES (Space owns toggle). With the
//     footer unfocused, Enter on Channels now moves to the next step. ───

#[test]
fn enter_unfocused_footer_advances_on_grid() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    // No Tab → footer unfocused → Enter advances the flow (Space = toggle).
    enter(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Enter on a grid screen advances (merakizzz nav-UX: Space toggles, Enter = next)"
    );
}
