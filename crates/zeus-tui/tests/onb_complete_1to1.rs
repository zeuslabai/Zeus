//! 1:1 render tests for onboarding Screen 19/19 — Complete (CMPL) · JSX 1738–1810.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-complete`:
//!   • 2-column layout (LEFT: ZeusFace + header + summary + buttons,
//!     RIGHT 320w borderLeft: NEXT STEPS + SUMMARY SAVED),
//!   • ZeusFace static layout — ready state `(◉‿◉)` "ready to wake" (Idle),
//!     success `(◉‿◉)✓` "all systems go" (Tested) — animation = follow-up,
//!   • header "✓ Configuration complete" + sub with ~/.zeus/config.toml accent,
//!   • summary rows: dot + name + value + badge per status
//!     (configured→✓ READY green · skipped→⏭ SKIPPED muted · error→✕ ERROR red),
//!   • buttons: ▸ TEST ALL BACKENDS (Idle→Tested = ✓ ALL BACKENDS PASSED) +
//!     ▸ AWAKEN ZEUS (accent),
//!   • NEXT STEPS: $ zeus start / chat / pantheon / onboard --resume,
//!   • Nav: Complete (step 18, last) — Tab cycles buttons, Enter=AWAKEN→prod,
//!     ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::screens::complete::{CompleteScreen, RowStatus, SummaryRow, TestResult, TestState};

/// A single passing backend check — drives `run_test_all` to `Tested` in
/// render-state tests (the real per-backend checks are computed in `App`).
fn ok_results() -> Vec<TestResult> {
    vec![TestResult {
        name: "Provider API key".to_string(),
        passed: true,
        detail: "key format OK".to_string(),
    }]
}

use crossterm::event::KeyCode;

const COMPLETE_STEP: usize = 18;

/// Walk to the Complete step (18, the LAST step). Grid steps (1/6/8/9/11/15/17)
/// consume ←/→ for in-screen focus and Enter for toggle, so we bump them
/// directly — same cross-screen cascade idiom as goto_step in onb_106/106b.
fn goto_complete(app: &mut App) {
    let mut guard = 0;
    while app.current_step < COMPLETE_STEP {
        if app.current_step == 3 { app.current_step += 1; app.on_step_enter(); continue; }        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 6 || s == 8 || s == 9 || s == 11 || s == 15 || s == 17 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
        guard += 1;
        assert!(guard < 100, "goto_complete stalled before reaching Complete");
    }
    assert_eq!(app.current_step, COMPLETE_STEP, "failed to reach Complete step");
}

fn render(app: &mut App) -> String {
    render_app_at(app, 140, 44)
}

fn render_app_at(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    buf_to_string(terminal.backend().buffer().clone())
}

/// Render a CompleteScreen directly (lets us exercise Tested + Error states
/// the wizard wiring doesn't naturally produce in a fresh App walk).
fn render_screen(scr: &CompleteScreen) -> String {
    render_screen_at(scr, 140, 44)
}

fn render_screen_at(scr: &CompleteScreen, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    scr.render(area, &mut buf);
    buf_to_string(buf)
}

fn buf_to_string(buf: ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn sample_summary() -> Vec<SummaryRow> {
    vec![
        SummaryRow {
            name: "LLM Provider".into(),
            value: "anthropic/claude-opus-4-8".into(),
            status: RowStatus::Configured,
        },
        SummaryRow {
            name: "Voice (TTS)".into(),
            value: "none".into(),
            status: RowStatus::Skipped,
        },
        SummaryRow {
            name: "Gateway".into(),
            value: "bind failed".into(),
            status: RowStatus::Error,
        },
    ]
}

// ── ZeusFace (the headline gap that was MISSING) ──────────────────────────────

#[test]
fn complete_zeusface_ready_state() {
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render(&mut app);
    // Idle/ready: the ZeusFace glyph + the "ready to wake" label must render
    // BEFORE the header (JSX 1751 leads the left column with the face).
    assert!(s.contains("(◉‿◉)"), "ZeusFace ready glyph missing\n{s}");
    assert!(s.contains("ready to wake"), "ZeusFace ready label missing\n{s}");
}

#[test]
fn complete_zeusface_success_state() {
    let mut scr = CompleteScreen::new();
    scr.summary = sample_summary();
    scr.run_test_all(ok_results()); // Idle -> Tested
    let s = render_screen(&scr);
    // Tested/success: glyph gains the ✓ and the label flips to "all systems go".
    assert!(s.contains("(◉‿◉)✓"), "ZeusFace success glyph missing\n{s}");
    assert!(s.contains("all systems go"), "ZeusFace success label missing\n{s}");
}

// ── header + sub ──────────────────────────────────────────────────────────────

#[test]
fn complete_header_and_sub() {
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render(&mut app);
    assert!(
        s.contains("✓ Configuration complete"),
        "header missing\n{s}"
    );
    assert!(
        s.contains("persist to") && s.contains("~/.zeus/config.toml"),
        "sub copy / config path missing\n{s}"
    );
}

// ── summary rows: all 3 statuses + badges ─────────────────────────────────────

#[test]
fn complete_summary_badges_all_three_statuses() {
    let mut scr = CompleteScreen::new();
    scr.summary = sample_summary();
    let s = render_screen(&scr);
    // configured -> ✓ READY ; skipped -> ⏭ SKIPPED ; error -> ✕ ERROR.
    assert!(s.contains("✓ READY"), "configured badge missing\n{s}");
    assert!(s.contains("⏭ SKIPPED"), "skipped badge missing\n{s}");
    assert!(s.contains("✕ ERROR"), "error badge missing\n{s}");
    // names + a value render too.
    assert!(s.contains("LLM Provider"), "configured row name missing\n{s}");
    assert!(s.contains("Voice (TTS)"), "skipped row name missing\n{s}");
    assert!(s.contains("Gateway"), "error row name missing\n{s}");
}

#[test]
fn complete_100x30_keeps_prototype_summary_stack() {
    let mut scr = CompleteScreen::new();
    scr.summary = sample_summary();
    let s = render_screen_at(&scr, 100, 30);

    for expected in ["LLM Provider", "Voice (TTS)", "Gateway"] {
        assert!(s.contains(expected), "summary row {expected:?} missing at 100x30\n{s}");
    }
    for expected in ["✓ READY", "⏭ SKIPPED", "✕ ERROR"] {
        assert!(s.contains(expected), "summary badge {expected:?} missing at 100x30\n{s}");
    }
    assert!(s.contains("TEST ALL BACKENDS"), "backend-test affordance missing at 100x30\n{s}");
    assert!(s.contains("AWAKEN ZEUS"), "awaken affordance missing at 100x30\n{s}");
}

#[test]
fn complete_app_100x30_renders_dense_summary_stack() {
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render_app_at(&mut app, 100, 30);

    for expected in ["LLM Provider", "Gateway", "Skills"] {
        assert!(s.contains(expected), "wizard summary row {expected:?} missing at 100x30\n{s}");
    }
    assert!(s.contains("✓ READY"), "configured summary badge missing at 100x30\n{s}");
    assert!(s.contains("TEST ALL BACKENDS"), "backend-test affordance missing at 100x30\n{s}");
    assert!(s.contains("AWAKEN ZEUS"), "awaken affordance missing at 100x30\n{s}");
}

#[test]
fn complete_summary_from_wizard_state() {
    // The App walk builds the summary from real wizard state on step entry —
    // at minimum the LLM Provider row (always Configured) must show.
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render(&mut app);
    assert!(s.contains("LLM Provider"), "wizard summary not built\n{s}");
    assert!(s.contains("✓ READY"), "configured badge from wizard missing\n{s}");
}

// ── buttons ──────────────────────────────────────────────────────────────────

#[test]
fn complete_both_buttons_render() {
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render(&mut app);
    // Focus marker (▶) is now applied by focus state, not baked into the label:
    // the focused button (default = TEST, button 0) carries the leading ▶, the
    // unfocused button renders its bare label with no marker.
    assert!(
        s.contains("▶ TEST ALL BACKENDS"),
        "focused TEST ALL BACKENDS button (with ▶ marker) missing\n{s}"
    );
    assert!(
        s.contains("AWAKEN ZEUS"),
        "AWAKEN ZEUS button missing\n{s}"
    );
}

#[test]
fn complete_test_button_idle_to_passed() {
    let mut scr = CompleteScreen::new();
    scr.summary = sample_summary();
    // Idle label.
    let s0 = render_screen(&scr);
    // TEST is the default-focused button → carries the ▶ focus marker.
    assert!(s0.contains("▶ TEST ALL BACKENDS"), "idle test label missing\n{s0}");
    assert!(
        !s0.contains("ALL BACKENDS PASSED"),
        "passed label leaked before test\n{s0}"
    );
    // Run test -> Tested label.
    scr.run_test_all(ok_results());
    assert_eq!(scr.test_state, TestState::Tested);
    let s1 = render_screen(&scr);
    assert!(
        s1.contains("✓ ALL BACKENDS PASSED"),
        "passed label missing after test\n{s1}"
    );
}

// ── NEXT STEPS (right panel) ──────────────────────────────────────────────────

#[test]
fn complete_next_steps_commands() {
    let mut app = App::new();
    goto_complete(&mut app);
    let s = render(&mut app);
    for cmd in ["zeus start", "zeus chat", "zeus pantheon", "zeus onboard --resume"] {
        assert!(s.contains(cmd), "NEXT STEPS missing `{cmd}`\n{s}");
    }
}

// ── nav: Tab cycles buttons, Enter=AWAKEN→prod, ESC=back ──────────────────────

#[test]
fn complete_tab_cycles_buttons() {
    let mut app = App::new();
    goto_complete(&mut app);
    assert_eq!(app.complete_screen.focused_button, 0, "default focus = TEST");
    app.handle_key(KeyCode::Tab);
    assert_eq!(app.complete_screen.focused_button, 1, "Tab -> AWAKEN");
    app.handle_key(KeyCode::Tab);
    assert_eq!(app.complete_screen.focused_button, 0, "Tab wraps -> TEST");
}

#[test]
fn complete_enter_on_awaken_enters_production() {
    let mut app = App::new();
    goto_complete(&mut app);
    app.handle_key(KeyCode::Tab); // focus AWAKEN
    assert_eq!(app.complete_screen.focused_button, 1);
    assert!(!app.onboarding_complete, "not done before AWAKEN");

    // AWAKEN now starts a VISIBLE handoff (the "⚡ Launching Zeus…" splash)
    // instead of an instant silent swap — pressing it must clearly DO something.
    app.handle_key(KeyCode::Enter);
    assert!(
        app.is_launching(),
        "Enter on AWAKEN must start the visible launching handoff"
    );
    assert!(
        !app.onboarding_complete,
        "prod UI must not take over until the handoff dwell elapses"
    );

    // The tick-driven dwell completes the transition into the production UI.
    for _ in 0..8 {
        app.tick();
    }
    assert!(
        app.onboarding_complete,
        "after the handoff dwell, AWAKEN enters the production UI"
    );
    assert!(!app.is_launching(), "splash clears once prod takes over");
}

#[test]
fn complete_enter_on_test_runs_test_not_awaken() {
    let mut app = App::new();
    goto_complete(&mut app);
    assert_eq!(app.complete_screen.focused_button, 0, "focus = TEST");
    app.handle_key(KeyCode::Enter);
    // Real backend checks now run (not the old rubber-stamp). On a freshly
    // constructed App with no API key entered, the provider check correctly
    // FAILS — so the state leaves Idle but lands on Failed, not Tested. The
    // contract this test guards is "Enter on TEST runs the checks (state
    // advances off Idle) and does NOT awaken" — not a specific pass/fail.
    assert_ne!(
        app.complete_screen.test_state,
        TestState::Idle,
        "Enter on TEST must run the checks (state advances off Idle)"
    );
    assert!(
        !app.onboarding_complete,
        "Enter on TEST must NOT awaken (only AWAKEN does)"
    );
}

#[test]
fn complete_esc_backs_out_not_quit() {
    let mut app = App::new();
    goto_complete(&mut app);
    assert_eq!(app.current_step, COMPLETE_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        COMPLETE_STEP - 1,
        "ESC backs out one step (does not quit / stay)"
    );
    assert!(!app.onboarding_complete, "ESC must not awaken");
}
