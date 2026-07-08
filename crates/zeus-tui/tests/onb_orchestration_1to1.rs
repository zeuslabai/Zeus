//! 1:1 render tests for onboarding Screen 16/19 — Orchestration (ORCH) · JSX 1565–1608.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-orchestration`:
//!   • header "Orchestration mode" + sub,
//!   • 3-col grid of mode cards (All-on/Heartbeat-only/Disabled) sharing one row,
//!   • glyph badge (filled), name, italic sub, desc,
//!   • badge precedence: ▸ SELECTED on the selected card, ★ REC on the
//!     recommended-but-unselected card,
//!   • HEARTBEAT TIMING fields (Interval/Quiet Start/Quiet End) shown when
//!     selected ≠ Disabled, hidden (replaced by info line) when Disabled,
//!   • ←/→ move the mode selection (grid-local), NOT the step,
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const ORCH_STEP: usize = 16;

/// Walk to the Orchestration step (15). Steps 1/6/8/9/11 consume Right for
/// in-screen focus, so we bump those directly — mirrors goto_step in onb_106.
/// Orchestration itself is the target, so we stop AT it.
fn goto_orchestration(app: &mut App) {
    while app.current_step < ORCH_STEP {
        if app.current_step == 4 {
            app.current_step += 1;
            app.on_step_enter();
            continue;
        }
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 7 || s == 9 || s == 10 || s == 12 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
    }
    assert_eq!(
        app.current_step, ORCH_STEP,
        "failed to reach Orchestration step"
    );
}

fn render(app: &mut App) -> String {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    buf_to_string(terminal.backend().buffer().clone())
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

/// Find the row index (0-based) of the first line containing `needle`.
fn row_of(s: &str, needle: &str) -> Option<usize> {
    s.lines().position(|l| l.contains(needle))
}

// ── header + sub ────────────────────────────────────────────────────────────

#[test]
fn orch_header_and_sub() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Orchestration mode"), "missing header\n{s}");
    assert!(
        s.contains("How Zeus runs background work"),
        "missing sub copy\n{s}"
    );
}

// ── 3-col grid: all three mode names share one row ──────────────────────────

#[test]
fn orch_three_col_grid_names_share_row() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    let s = render(&mut app);
    let r_all = row_of(&s, "All-on").expect("All-on present");
    let r_hb = row_of(&s, "Heartbeat-only").expect("Heartbeat-only present");
    let r_off = row_of(&s, "Disabled").expect("Disabled present");
    assert_eq!(
        r_all, r_hb,
        "All-on and Heartbeat-only not on same row\n{s}"
    );
    assert_eq!(
        r_hb, r_off,
        "Heartbeat-only and Disabled not on same row\n{s}"
    );
}

// ── glyph badges present ─────────────────────────────────────────────────────

#[test]
fn orch_glyph_badges_present() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    let s = render(&mut app);
    assert!(s.contains("ALL"), "missing ALL glyph\n{s}");
    assert!(s.contains("HB"), "missing HB glyph\n{s}");
    assert!(s.contains("OFF"), "missing OFF glyph\n{s}");
}

// ── badge precedence: ▸ SELECTED on selection, ★ REC on unselected-recommended ─

#[test]
fn orch_selected_badge_on_default_recommended() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    // Default selection = All-on (index 0, recommended) → ▸ SELECTED, no ★ REC.
    let s = render(&mut app);
    assert!(
        s.contains("▸ SELECTED"),
        "missing ▸ SELECTED on default\n{s}"
    );
    assert!(
        !s.contains("★ REC"),
        "★ REC should be hidden while the recommended card IS selected\n{s}"
    );
}

#[test]
fn orch_rec_badge_appears_when_recommended_unselected() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    // Move off All-on → the recommended (All-on) card now shows ★ REC, and
    // ▸ SELECTED follows the new pick.
    app.handle_key(KeyCode::Right);
    let s = render(&mut app);
    assert!(
        s.contains("★ REC"),
        "★ REC missing after moving off recommended\n{s}"
    );
    assert!(
        s.contains("▸ SELECTED"),
        "▸ SELECTED missing on new pick\n{s}"
    );
}

// ── HEARTBEAT TIMING fields shown for All-on / Heartbeat-only ────────────────

#[test]
fn orch_timing_fields_shown_when_not_disabled() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    // Default = All-on → timing fields visible.
    let s = render(&mut app);
    assert!(
        s.contains("HEARTBEAT TIMING"),
        "missing timing section\n{s}"
    );
    assert!(s.contains("Interval"), "missing Interval field\n{s}");
    assert!(s.contains("Quiet Start"), "missing Quiet Start field\n{s}");
    assert!(s.contains("Quiet End"), "missing Quiet End field\n{s}");
    // Default value hints.
    assert!(s.contains("300"), "missing 300 default\n{s}");
}

// ── HEARTBEAT TIMING hidden when Disabled selected ──────────────────────────

#[test]
fn orch_timing_fields_hidden_when_disabled() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    // Move to Disabled (index 2): Right twice.
    app.handle_key(KeyCode::Right);
    app.handle_key(KeyCode::Right);
    let s = render(&mut app);
    assert!(
        !s.contains("HEARTBEAT TIMING"),
        "timing section must be hidden when Disabled\n{s}"
    );
    assert!(
        s.contains("No configuration needed for disabled mode"),
        "missing disabled-mode info line\n{s}"
    );
}

// ── ←/→ move selection (grid-local), NOT the step ───────────────────────────

#[test]
fn orch_left_right_move_selection_not_step() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    assert_eq!(app.current_step, ORCH_STEP);
    // Right should move the mode selection, not advance the step.
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step, ORCH_STEP,
        "Right must not step-advance on the Orchestration grid"
    );
    let s = render(&mut app);
    // After one Right, Heartbeat-only (index 1) is selected.
    let r_sel = row_of(&s, "▸ SELECTED").expect("▸ SELECTED present");
    let r_hb = row_of(&s, "Heartbeat-only").expect("Heartbeat-only present");
    // Both should be in the same card column band → roughly same row region.
    assert!(
        r_sel <= r_hb,
        "▸ SELECTED should sit above the HB name\n{s}"
    );
    // Left moves back; step still unchanged.
    app.handle_key(KeyCode::Left);
    assert_eq!(
        app.current_step, ORCH_STEP,
        "Left must not step-back on the grid"
    );
}

#[test]
fn orch_left_clamps_no_wrap() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    // At index 0, Left should clamp (no wrap to Disabled) and not step-back.
    app.handle_key(KeyCode::Left);
    assert_eq!(
        app.current_step, ORCH_STEP,
        "Left at index 0 must clamp, not step-back"
    );
    let s = render(&mut app);
    // Still on All-on (recommended + selected → ▸ SELECTED, no ★ REC).
    assert!(
        s.contains("▸ SELECTED"),
        "should still be selected on All-on\n{s}"
    );
    assert!(
        !s.contains("★ REC"),
        "no ★ REC while All-on is selected\n{s}"
    );
}

// ── ESC backs out one step (does not quit) ──────────────────────────────────

#[test]
fn orch_esc_backs_out_not_quit() {
    let mut app = App::new();
    goto_orchestration(&mut app);
    assert_eq!(app.current_step, ORCH_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        ORCH_STEP - 1,
        "ESC should back out one step, not quit"
    );
}
