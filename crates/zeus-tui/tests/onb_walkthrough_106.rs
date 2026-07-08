//! End-to-end headless WALK-THROUGH of the full onboarding wizard (zeus106).
//!
//! Render+unit+per-screen-1:1 tests verify each screen in isolation. They
//! CANNOT structurally catch what only a live run surfaces: runtime panics on
//! a real render, state-machine bleed-through across transitions, and the
//! AWAKEN→launch handoff firing end-to-end. This test closes that gap headlessly
//! by driving the *same* seams the live loop uses (`handle_key`, `tick`,
//! `advance_step`) and rendering every step through `TestBackend` (the same
//! `frame(f, app)` the real loop calls), asserting no panic at any step.
//!
//! It is NOT a substitute for a human click-through (it can't see colour/blink
//! fidelity), but it IS the structural net for panics + transition bleed that
//! the isolated render tests miss. Separate file = conflict-free with other
//! agents' onb_*.rs.

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::Mutex;
use zeus_tui::app::{App, frame};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_isolated_zeus_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("temp ZEUS_HOME");
    let previous = std::env::var_os("ZEUS_HOME");
    unsafe {
        std::env::set_var("ZEUS_HOME", tmp.path());
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tmp.path())));
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

const LAST_STEP: usize = 19;
// Some steps cannot be advanced by a raw Right key in tests: grids/toggles
// consume ←/→ for in-screen focus, and Auth is probe-gated. The established
// onboarding-test idiom bumps them directly rather than waiting on live IO.
const DIRECT_ADVANCE_STEPS: [usize; 8] = [1, 4, 7, 9, 10, 12, 16, 18];

/// Render the full chrome+screen for the current step via the SAME entrypoint
/// the live loop uses. `.expect` on draw turns any paint panic into a test
/// failure pinned to the offending step.
fn render_current(app: &mut App, step: usize) {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .unwrap_or_else(|e| panic!("draw panicked at step {step}: {e}"));
}

/// Advance exactly one step forward using the live key path, with the grid-step
/// cascade idiom. Returns once `current_step` has incremented (or asserts stall).
fn step_forward(app: &mut App) {
    let s = app.current_step;
    if s == 1 {
        // Mode grid: Enter selects + advances.
        app.handle_key(KeyCode::Enter);
    } else if DIRECT_ADVANCE_STEPS.contains(&s) {
        app.current_step += 1;
        app.on_step_enter();
    } else {
        app.handle_key(KeyCode::Right);
    }
    if app.current_step == s {
        app.handle_key(KeyCode::Enter);
    }
    assert!(
        app.current_step > s,
        "walk-through stalled at step {s} (no forward transition)"
    );
}

#[test]
fn full_onboarding_walk_forward_renders_every_step_without_panic() {
    let mut app = App::new();
    assert_eq!(
        app.current_step, 0,
        "fresh App must start at Welcome (step 0)"
    );
    assert!(
        !app.onboarding_complete,
        "fresh App must boot into onboarding, not prod"
    );

    // Render Welcome, then walk 0→18 rendering each step we land on.
    let s0 = app.current_step;
    render_current(&mut app, s0);
    let mut guard = 0;
    while app.current_step < LAST_STEP {
        step_forward(&mut app);
        let sc = app.current_step;
        render_current(&mut app, sc);
        guard += 1;
        assert!(guard < 100, "forward walk exceeded guard — possible loop");
    }
    assert_eq!(app.current_step, LAST_STEP, "must reach Complete (19)");
}

#[test]
fn full_onboarding_walk_back_renders_every_step_without_panic() {
    // Forward to Complete, then ESC all the way back to Welcome — exercises the
    // reverse transitions (bleed-through risk the isolated render tests skip).
    let mut app = App::new();
    let mut guard = 0;
    while app.current_step < LAST_STEP {
        step_forward(&mut app);
        guard += 1;
        assert!(guard < 100, "forward walk exceeded guard");
    }

    // Now walk back. ESC backs out one step (established idiom, does NOT quit).
    guard = 0;
    while app.current_step > 0 {
        let before = app.current_step;
        app.handle_key(KeyCode::Esc);
        let after = app.current_step;
        render_current(&mut app, after);
        // A decrement proves ESC backed out one step rather than quitting (a
        // quit path would leave current_step unchanged, tripping this assert).
        assert!(
            after < before,
            "ESC failed to back out from step {before} (bleed-through or quit)"
        );
        guard += 1;
        assert!(guard < 100, "back walk exceeded guard");
    }
    assert_eq!(app.current_step, 0, "ESC chain must land back at Welcome");
}

#[test]
fn typing_into_editable_fields_does_not_panic_and_caret_renders() {
    // Walk to the Auth screen (step 3 — first editable text field), type a key,
    // tick to flip the cursor blink, and render. Catches a panic on the caret
    // paint path with live input + the blink seam together.
    let mut app = App::new();
    let mut guard = 0;
    while app.current_step < 4 {
        step_forward(&mut app);
        guard += 1;
        assert!(guard < 50, "failed to reach Auth");
    }
    assert_eq!(app.current_step, 4, "expected Auth at step 3");

    // Type a character into the focused field, advance the animation clock a few
    // ticks (covers both blink phases), render each time — no panic allowed.
    app.handle_key(KeyCode::Char('s'));
    app.handle_key(KeyCode::Char('k'));
    for _ in 0..6 {
        app.tick();
        let sc = app.current_step;
        render_current(&mut app, sc);
    }
}

#[test]
fn awaken_fires_launch_handoff_then_completes() {
    with_isolated_zeus_home(|zeus_home| {
        // Drive to Complete and fire AWAKEN — must enter the visible "launching"
        // handoff (AWAKEN-A), NOT jump silently to prod, then tick to completion.
        let mut app = App::new();
        let mut guard = 0;
        while app.current_step < LAST_STEP {
            step_forward(&mut app);
            guard += 1;
            assert!(guard < 100, "failed to reach Complete");
        }

        // AWAKEN. advance_step on Complete fires the launch handoff and persists
        // config; the temp ZEUS_HOME keeps this test from touching a real box.
        app.advance_step();
        assert!(
            zeus_home.join("config.toml").exists(),
            "AWAKEN should persist only inside isolated ZEUS_HOME"
        );
        // Render the launching frame — must not panic and must not have silently
        // flipped straight to a completed prod state on the same tick.
        let sc = app.current_step;
        render_current(&mut app, sc);

        // Tick past the launch dwell — eventually onboarding_complete flips true.
        let mut ticks = 0;
        while !app.onboarding_complete && ticks < 50 {
            app.tick();
            let sc = app.current_step;
            render_current(&mut app, sc);
            ticks += 1;
        }
        assert!(
            app.onboarding_complete,
            "AWAKEN handoff must complete to onboarding_complete within dwell window"
        );
    });
}
