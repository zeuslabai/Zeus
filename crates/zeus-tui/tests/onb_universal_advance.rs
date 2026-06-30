//! Proof tests for the universal advance control (Ctrl+N) — the deploy
//! blocker from merakizzz's 20260618 testing: Channels can't advance after
//! selecting Discord, and Voice strands the user (Enter toggles on the
//! multiselect/test screens; nothing advances the step).
//!
//! Root cause: on Channels / Voice / Features / Fallback / Gateway / Skills,
//! the `Enter` handler is consumed by a per-screen toggle/test action, and
//! `Right` is consumed as grid-nav — so NO key advanced the step. Ctrl+N is a
//! collision-free universal advance: it works on EVERY onboarding screen via
//! `advance_step()`, including the wedge screens, without disturbing any
//! existing per-screen key.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use crossterm::event::{KeyCode, KeyModifiers};
use std::sync::Mutex;
use zeus_tui::App;

const AUTH: usize = 3;
const CHANNELS: usize = 6;
const FEATURES: usize = 12;
const VOICE: usize = 13;
const COMPLETE: usize = 18;

// `ctrl_n_on_complete_awakens` drives the real Complete/AWAKEN path, which
// persists through `Config::load()`/`save_unchecked()` and therefore respects
// the process-global `ZEUS_HOME`. Keep that test off the owner's real
// ~/.zeus and off any parallel env mutation so the assertion is deterministic
// under full-suite load.
static ZEUS_HOME_LOCK: Mutex<()> = Mutex::new(());

fn with_isolated_zeus_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let _guard = ZEUS_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os("ZEUS_HOME");
    let tmp = tempfile::tempdir().expect("create isolated ZEUS_HOME");

    // Seed a valid config so the Complete persist path loads a real config and
    // cannot depend on whatever ~/.zeus/config.toml looks like on the runner.
    std::fs::write(
        tmp.path().join("config.toml"),
        "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = false\n",
    )
    .expect("seed isolated config.toml");

    // SAFETY: this helper serializes all ZEUS_HOME mutation in this test binary
    // and restores the previous value before returning.
    unsafe {
        std::env::set_var("ZEUS_HOME", tmp.path());
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tmp.path())));

    // SAFETY: same serialized env section as above; restore even on panic.
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("ZEUS_HOME", previous);
        } else {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn ctrl_n(app: &mut App) {
    app.handle_key_mods(KeyCode::Char('n'), KeyModifiers::CONTROL);
}

/// Jump directly to a step via the on-enter sync path (mirrors the test
/// idiom other onb_*.rs files use for grid screens that don't advance on Right).
fn goto(app: &mut App, step: usize) {
    app.current_step = step;
    app.on_step_enter();
    assert_eq!(app.current_step, step, "failed to seat at step {step}");
}

// ─── The wedge screens: Ctrl+N advances where Enter/Right are consumed ───

#[test]
fn ctrl_n_advances_channels_the_reported_wedge() {
    let mut app = App::new();
    goto(&mut app, CHANNELS);

    // merakizzz nav-UX fix: Enter now ADVANCES on grid screens (Space toggles).
    // Right is still grid-nav — must not advance.
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step, CHANNELS,
        "Right is grid-nav on Channels — must not advance"
    );

    // Ctrl+N is the universal advance shortcut — also unblocks onboarding.
    ctrl_n(&mut app);
    assert_eq!(
        app.current_step,
        CHANNELS + 1,
        "Ctrl+N must advance past Channels (the reported deploy blocker)"
    );
}

#[test]
fn ctrl_n_advances_voice_the_reported_strand() {
    let mut app = App::new();
    goto(&mut app, VOICE);

    // merakizzz nav-UX fix: Enter now advances past Voice (the reported strand).
    ctrl_n(&mut app);
    assert_eq!(
        app.current_step,
        VOICE + 1,
        "Ctrl+N must advance past Voice (the reported strand)"
    );
}

#[test]
fn ctrl_n_advances_features_multiselect() {
    let mut app = App::new();
    goto(&mut app, FEATURES);

    // merakizzz nav-UX fix: Enter advances on Features now (Space toggles).
    ctrl_n(&mut app);
    assert_eq!(
        app.current_step,
        FEATURES + 1,
        "Ctrl+N advances past Features"
    );
}

// ─── Universality: Ctrl+N advances every interior step ───

#[test]
fn ctrl_n_advances_every_interior_step() {
    // Walk the whole flow using ONLY Ctrl+N. If any step swallows it, this
    // wedges — proving the control is truly universal.
    let mut app = App::new();
    let mut guard = 0;
    while app.current_step < COMPLETE {
        let before = app.current_step;
        // #240: the Auth step (3) now gates advance on a valid key format.
        // Seed a valid Anthropic key (default provider → `sk-ant-` prefix)
        // before driving Ctrl+N, so the universality walk isn't blocked by the
        // intentional validation gate.
        if before == AUTH {
            app.set_auth_api_key("sk-ant-walk-test-key");
        }
        ctrl_n(&mut app);
        assert_eq!(
            app.current_step,
            before + 1,
            "Ctrl+N failed to advance step {before} — not universal"
        );
        guard += 1;
        assert!(guard <= 30, "runaway loop");
    }
    assert_eq!(
        app.current_step, COMPLETE,
        "Ctrl+N walked the full flow to Complete"
    );
}

// ─── Complete: Ctrl+N = AWAKEN (mirror the Enter AWAKEN arm) ───

#[test]
fn ctrl_n_on_complete_awakens() {
    with_isolated_zeus_home(|zeus_home| {
        let mut app = App::new();
        goto(&mut app, COMPLETE);
        assert!(!app.onboarding_complete, "precondition: not yet awoken");

        ctrl_n(&mut app);
        assert!(
            app.onboarding_complete,
            "Ctrl+N on Complete must AWAKEN (transition to production UI)"
        );

        let persisted = std::fs::read_to_string(zeus_home.join("config.toml"))
            .expect("Complete Ctrl+N should persist isolated config.toml");
        assert!(
            persisted.contains("onboarding_complete = true"),
            "Complete Ctrl+N should persist the onboarding completion flip; got:\n{persisted}"
        );
    });
}

// ─── Non-regression: Ctrl+N must not disturb existing per-screen keys ───

#[test]
fn ctrl_n_does_not_break_plain_n_on_text_screens() {
    // A plain 'n' (no modifier) on a text-input screen must still type, not
    // advance. Ctrl+N is the ONLY advance trigger; bare 'n' is untouched.
    let mut app = App::new();
    goto(&mut app, CHANNELS);
    let before = app.current_step;
    // Bare 'n' through the modifier-aware entry with NO modifiers.
    app.handle_key_mods(KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(
        app.current_step, before,
        "bare 'n' (no Ctrl) must NOT advance — only Ctrl+N does"
    );
}

#[test]
fn ctrl_n_inert_after_onboarding_complete() {
    // Once in production UI, Ctrl+N must fall through to prod handling and
    // not re-trigger onboarding advance.
    let mut app = App::new();
    app.onboarding_complete = true;
    let before = app.current_step;
    ctrl_n(&mut app);
    assert_eq!(
        app.current_step, before,
        "Ctrl+N must be inert for onboarding-advance once complete"
    );
}
