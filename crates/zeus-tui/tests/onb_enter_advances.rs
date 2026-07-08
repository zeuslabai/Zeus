//! Proof tests for the P0 nav-UX wedge fix (merakizzz got stuck at Voice/Channels):
//! on grid/multiselect screens, **Enter ADVANCES the flow** (Space owns toggle
//! there). Before this fix, Enter was consumed by the per-screen toggle/test
//! (channels.toggle_focused, voice.test_voice) so the flow felt stuck — you had
//! to Tab to the footer NEXT first. Now Enter (with no footer focus) advances
//! directly on Channels/Features/Voice/Images.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use crossterm::event::KeyCode;
use zeus_tui::App;

const CHANNELS: usize = 7;
const VOICE: usize = 14;
const FEATURES: usize = 13;
const IMAGES: usize = 15;

fn goto(app: &mut App, step: usize) {
    app.current_step = step;
    app.on_step_enter();
    assert_eq!(app.current_step, step, "failed to seat at step {step}");
}

fn enter(app: &mut App) {
    app.handle_key(KeyCode::Enter);
}

// ─── Channels: Enter advances directly (no Tab-to-footer needed) ───

#[test]
fn channels_enter_advances_directly() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    // No footer focus seeded — Enter should advance the step, not toggle.
    assert_eq!(app.footer_focus, None, "fresh screen has no footer focus");
    enter(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Enter on Channels (grid) must advance the flow, not be consumed by toggle"
    );
}

// ─── Voice (the exact wedge merakizzz hit): Enter advances ───

#[test]
fn voice_enter_advances_directly() {
    let mut app = App::new();
    goto(&mut app, VOICE);
    assert_eq!(app.footer_focus, None);
    enter(&mut app);
    assert_eq!(
        app.current_step,
        VOICE + 1,
        "Enter on Voice must advance — this is the can't-continue wedge"
    );
}

// ─── Features: Enter advances ───

#[test]
fn features_enter_advances_directly() {
    let mut app = App::new();
    goto(&mut app, FEATURES);
    assert_eq!(app.footer_focus, None);
    enter(&mut app);
    assert_eq!(
        app.current_step,
        FEATURES + 1,
        "Enter on Features must advance"
    );
}

// ─── Images: Enter advances ───

#[test]
fn images_enter_advances_directly() {
    let mut app = App::new();
    goto(&mut app, IMAGES);
    assert_eq!(app.footer_focus, None);
    enter(&mut app);
    assert_eq!(app.current_step, IMAGES + 1, "Enter on Images must advance");
}

// ─── Space STILL toggles on Channels (we didn't break selection) ───

#[test]
fn channels_space_still_toggles_not_advances() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    app.handle_key(KeyCode::Char(' '));
    assert_eq!(
        app.current_step, CHANNELS,
        "Space on Channels toggles selection — must NOT advance the step"
    );
}
