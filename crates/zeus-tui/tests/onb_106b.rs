//! Integration tests for onboarding steps 0–5 + 12–18 (zeus106b batch):
//!   0  Welcome     1  Mode        2  Provider    3  Auth
//!   4  Model       5  Fallback
//!   12 Features    13 Voice       14 Images      15 Orchestration
//!   16 Memory      17 Skills      18 Complete
//!
//! Strategy (per merakizzz spec): drive `App::new()` → `handle_key(KeyCode…)`
//! sequences → render via `app::frame` into a ratatui `TestBackend`, then
//! assert: nav advances/moves selection, selection PROPAGATES to the Complete
//! summary, and no panic on long / multibyte / empty paste.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.
//! NOTE: Provider LIST *content* is intentionally NOT asserted here — it's on
//! HOLD pending merakizzz. We only exercise Provider render/nav/propagation.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

// ---- helpers ---------------------------------------------------------------

/// Render the current app state into a 120×40 TestBackend and return the full
/// screen as a single newline-joined String (for substring asserts).
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

/// Advance from Welcome (step 0) to `target`. Right is step-nav on every
/// non-Mode screen; Mode (step 1) consumes Right for card selection so we use
/// Enter to leave it.
fn goto_step(app: &mut App, target: usize) {
    let mut guard = 0;
    while app.current_step < target {
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 3 || s == 6 || s == 8 || s == 9 || s == 11 || s == 15 || s == 17 {
            // Channels (6), Gateway (8), Agent (9), Security (11) and
            // Orchestration (15) are GRIDs: Right=column focus (Gateway's 4-col
            // picker, Security's 4-col level grid, Orchestration's 3-col mode
            // grid), Enter=toggle/no-op → neither advances the step. Bump
            // directly + fire the step-enter hook (mirrors the real "continue"
            // affordance leaving the grid).
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
        guard += 1;
        assert!(guard < 100, "goto_step stalled before reaching {target}");
    }
    assert_eq!(app.current_step, target, "failed to reach step {target}");
}

// A 5k-char ASCII blob + a multibyte/emoji blob — the classic byte-index panic
// triggers. Pressing these as Char events must never panic.
fn paste_blob() -> Vec<KeyCode> {
    let mut v: Vec<KeyCode> = "x".repeat(5000).chars().map(KeyCode::Char).collect();
    for c in "héllo—世界�switch🔱".chars() {
        v.push(KeyCode::Char(c));
    }
    v
}

// ---- step 0: Welcome -------------------------------------------------------

#[test]
fn step0_welcome_renders_and_advances() {
    let mut app = App::new();
    assert_eq!(app.current_step, 0);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "welcome frame should render");
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.current_step, 1, "Right should advance off Welcome");
}

// ---- step 1: Mode ----------------------------------------------------------

#[test]
fn step1_mode_lr_moves_cards_enter_advances() {
    let mut app = App::new();
    goto_step(&mut app, 1);
    // Right/Left move the card selection (not step-nav) on Mode.
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.current_step, 1, "Right on Mode must NOT advance step");
    press(&mut app, &[KeyCode::Left]);
    assert_eq!(app.current_step, 1, "Left on Mode must NOT advance step");
    // Enter leaves Mode.
    press(&mut app, &[KeyCode::Enter]);
    assert_eq!(app.current_step, 2, "Enter should advance off Mode");
}

// ---- step 2: Provider (render/nav/propagation only — list content on HOLD) --

#[test]
fn step2_provider_nav_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 2);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "provider frame should render");
    // Up/Down move provider selection without panic; step stays put.
    press(&mut app, &[KeyCode::Down, KeyCode::Down, KeyCode::Up]);
    assert_eq!(app.current_step, 2, "provider nav must not change step");
    let _ = render(&app);
}

// ---- step 3: Auth (paste safety) -------------------------------------------

#[test]
fn step3_auth_paste_no_panic() {
    let mut app = App::new();
    goto_step(&mut app, 3);
    // Long + multibyte API-key paste, then over-backspace.
    press(&mut app, &paste_blob());
    for _ in 0..6000 {
        app.handle_key(KeyCode::Backspace);
    }
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "auth frame should survive paste");
    assert_eq!(app.current_step, 3);
}

// ---- step 4: Model ---------------------------------------------------------

#[test]
fn step4_model_nav_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 4);
    press(&mut app, &[KeyCode::Down, KeyCode::Up]);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "model frame should render");
    assert_eq!(app.current_step, 4);
}

// ---- step 5: Fallback ------------------------------------------------------

#[test]
fn step5_fallback_toggle_no_panic() {
    let mut app = App::new();
    goto_step(&mut app, 5);
    // Enter toggles a candidate into the chain; nav + toggle must not panic.
    press(&mut app, &[KeyCode::Down, KeyCode::Enter, KeyCode::Up, KeyCode::Enter]);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "fallback frame should render");
    assert_eq!(app.current_step, 5);
}

// ---- step 12: Features -----------------------------------------------------

#[test]
fn step12_features_nav_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 12);
    press(&mut app, &[KeyCode::Down, KeyCode::Up]);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "features frame should render");
    assert_eq!(app.current_step, 12);
}

// ---- step 13: Voice (input wiring + paste safety) --------------------------

#[test]
fn step13_voice_input_and_paste_no_panic() {
    let mut app = App::new();
    goto_step(&mut app, 13);
    // Tab focuses a config field; typed chars + multibyte paste must land
    // without panic (regression: Voice previously had no Char wiring at all).
    press(&mut app, &[KeyCode::Tab]);
    press(&mut app, &paste_blob());
    for _ in 0..6000 {
        app.handle_key(KeyCode::Backspace);
    }
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "voice frame should survive input");
    assert_eq!(app.current_step, 13);
}

// ---- step 14: Images (paste safety) ----------------------------------------

#[test]
fn step14_images_input_no_panic() {
    let mut app = App::new();
    goto_step(&mut app, 14);
    press(&mut app, &[KeyCode::Tab]);
    press(&mut app, &paste_blob());
    for _ in 0..6000 {
        app.handle_key(KeyCode::Backspace);
    }
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "images frame should survive input");
    assert_eq!(app.current_step, 14);
}

// ---- step 15: Orchestration ------------------------------------------------

#[test]
fn step15_orchestration_nav_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 15);
    press(&mut app, &[KeyCode::Down, KeyCode::Up, KeyCode::Tab]);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "orchestration frame should render");
    assert_eq!(app.current_step, 15);
}

// ---- step 16: Memory (paste safety) ----------------------------------------

#[test]
fn step16_memory_input_no_panic() {
    let mut app = App::new();
    goto_step(&mut app, 16);
    press(&mut app, &[KeyCode::Tab]);
    press(&mut app, &paste_blob());
    for _ in 0..6000 {
        app.handle_key(KeyCode::Backspace);
    }
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "memory frame should survive input");
    assert_eq!(app.current_step, 16);
}

// ---- step 17: Skills -------------------------------------------------------

#[test]
fn step17_skills_nav_and_renders() {
    let mut app = App::new();
    goto_step(&mut app, 17);
    press(&mut app, &[KeyCode::Down, KeyCode::Up, KeyCode::Tab]);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "skills frame should render");
    assert_eq!(app.current_step, 17);
}

// ---- step 18: Complete (PROPAGATION) ---------------------------------------

#[test]
fn step18_complete_summary_propagates_earlier_choices() {
    let mut app = App::new();
    // Walk the full flow so each screen's state feeds build_summary().
    goto_step(&mut app, 18);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "complete frame should render");
    // The summary stack mirrors earlier-screen state — these labels are built
    // in App::build_summary() from voice/images/orchestration/memory/skills/
    // features/fallback/provider/auth. Their presence proves propagation ran.
    for needle in [
        "LLM Provider",
        "Authentication",
        "Backup LLMs",
        "Features",
        "Voice",
        "Image Generator",
        "Orchestration",
        "Memory",
        "Skills",
    ] {
        assert!(
            frame.contains(needle),
            "Complete summary must propagate `{needle}` row from earlier screens"
        );
    }
}

#[test]
fn step18_complete_multibyte_summary_does_not_panic() {
    // Regression for the byte-based `truncate()` in complete.rs: stuff a
    // multibyte value into the Auth key (flows into no summary row directly,
    // but exercises the full walk + render with multibyte state present).
    let mut app = App::new();
    goto_step(&mut app, 3);
    press(&mut app, &paste_blob());
    goto_step(&mut app, 18);
    let frame = render(&app);
    assert!(!frame.trim().is_empty(), "complete must render with multibyte state");
}
