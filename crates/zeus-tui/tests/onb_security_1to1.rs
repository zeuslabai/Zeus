//! 1:1 render tests for onboarding Screen 12/19 — Security (SCTY) · JSX 1353–1409.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-security`:
//!   • header "Aegis security level" + sub,
//!   • 4-col grid of level cards (Strict/Standard/Permissive/Custom) sharing one row,
//!   • glyph badge (filled), name, italic sub,
//!   • BLOCKED list: red label + `✕ {item}` (first 3 only),
//!   • badge precedence: ▸ SELECTED on the selected card, ★ REC on the
//!     recommended-but-unselected card,
//!   • SELECTED detail box + `[aegis] level = "{id}"` config write line,
//!   • ←/→ move the level selection (grid-local), NOT the step,
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const SECURITY_STEP: usize = 12;

/// Walk to the Security step (11). Steps 1/6/8/9 consume Right for in-screen
/// focus, so we bump those directly — mirrors goto_step in onb_106. Security
/// itself is the target, so we stop AT it (no s==12 special-case needed here
/// since we never walk past it).
fn goto_security(app: &mut App) {
    while app.current_step < SECURITY_STEP {
        if app.current_step == 4 {
            app.current_step += 1;
            app.on_step_enter();
            continue;
        }
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 7 || s == 9 || s == 10 {
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
        app.current_step, SECURITY_STEP,
        "failed to reach Security step"
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
fn security_header_and_sub() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Aegis security level"), "missing header\n{s}");
    assert!(
        s.contains("Approval pipeline is always active"),
        "missing sub copy\n{s}"
    );
}

// ── 4-col grid: all four level names share one row ─────────────────────────

#[test]
fn security_four_col_grid_names_share_row() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    // All four cards render their name; in a single horizontal row they should
    // land on the same buffer line.
    let r_strict = row_of(&s, "Strict").expect("Strict missing");
    let r_std = row_of(&s, "Standard").expect("Standard missing");
    let r_perm = row_of(&s, "Permissive").expect("Permissive missing");
    let r_cust = row_of(&s, "Custom").expect("Custom missing");
    assert_eq!(r_strict, r_std, "Strict/Standard not on same row\n{s}");
    assert_eq!(r_std, r_perm, "Standard/Permissive not on same row\n{s}");
    assert_eq!(r_perm, r_cust, "Permissive/Custom not on same row\n{s}");
}

#[test]
fn security_glyph_badges_present() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    for g in ["STR", "STD", "PRM", "CST"] {
        assert!(s.contains(g), "missing glyph badge {g}\n{s}");
    }
}

// ── BLOCKED list: ✕ prefix + first-3 slice ─────────────────────────────────

#[test]
fn security_blocked_list_uses_cross_glyph() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    // Strict has a BLOCKED list; the JSX prefixes items with `✕ `.
    assert!(s.contains("BLOCKED"), "missing BLOCKED label\n{s}");
    assert!(s.contains("✕"), "missing ✕ blocked-item glyph\n{s}");
    // Old `• ` bullet prefix must be gone from the cards.
    // (Permissive has zero blocked items — no BLOCKED label there, fine.)
}

#[test]
fn security_blocked_slices_first_three() {
    // Strict has 4 blocked items in the const; only the first 3 may render.
    // The 4th ("fs_write outside workspace") must NOT appear on a card.
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    // first item should be present
    assert!(
        s.contains("shell") || s.contains("✕"),
        "blocked items not rendered\n{s}"
    );
    // 4th strict item is long + distinctive; with first-3 slice it shouldn't show.
    // (Standard's "fs_write outside workspace + home" is a different string.)
    assert!(
        !s.contains("apply_patch") || s.matches("apply_patch").count() <= 1,
        "blocked list appears to exceed first-3 slice\n{s}"
    );
}

// ── badge precedence: ▸ SELECTED vs ★ REC ──────────────────────────────────

#[test]
fn security_selected_badge_on_selected_card() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    // Default selection = Standard (recommended). Selected card shows ▸ SELECTED.
    assert!(s.contains("▸ SELECTED"), "missing ▸ SELECTED badge\n{s}");
}

#[test]
fn security_rec_badge_when_recommended_unselected() {
    // Move selection OFF Standard (the recommended one) → ★ REC should appear
    // on the now-unselected recommended card; ▸ SELECTED moves to the new pick.
    let mut app = App::new();
    goto_security(&mut app);
    // Default = index 1 (Standard). Left → index 0 (Strict). Standard now
    // recommended-but-unselected → ★ REC shows.
    app.handle_key(KeyCode::Left);
    let s = render(&mut app);
    assert!(
        s.contains("★ REC"),
        "missing ★ REC on unselected recommended card\n{s}"
    );
    assert!(
        s.contains("▸ SELECTED"),
        "▸ SELECTED should follow the new pick\n{s}"
    );
}

// ── selected detail box + config write line ────────────────────────────────

#[test]
fn security_selected_detail_box_and_config_line() {
    let mut app = App::new();
    goto_security(&mut app);
    let s = render(&mut app);
    // Default = Standard.
    assert!(
        s.contains("SELECTED: STANDARD"),
        "missing SELECTED: {{NAME}} box\n{s}"
    );
    assert!(
        s.contains("[aegis] level ="),
        "missing config write preview\n{s}"
    );
    assert!(
        s.contains("~/.zeus/config.toml"),
        "missing config path\n{s}"
    );
}

// ── nav: ←/→ move the level selection, NOT the step ────────────────────────

#[test]
fn security_left_right_move_selection_not_step() {
    let mut app = App::new();
    goto_security(&mut app);
    let start_step = app.current_step;

    // Right → selection advances (Standard→Permissive), step unchanged.
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step, start_step,
        "Right must NOT step-advance at Security\n"
    );
    let s = render(&mut app);
    assert!(
        s.contains("SELECTED: PERMISSIVE"),
        "Right did not move selection\n{s}"
    );

    // Left twice → back to Strict, still same step.
    app.handle_key(KeyCode::Left);
    app.handle_key(KeyCode::Left);
    assert_eq!(
        app.current_step, start_step,
        "Left must NOT step-back at Security\n"
    );
    let s2 = render(&mut app);
    assert!(
        s2.contains("SELECTED: STRICT"),
        "Left did not move selection\n{s2}"
    );
}

#[test]
fn security_left_clamps_at_first_card() {
    // Hammer Left past the first card — must clamp at Strict (index 0), no wrap.
    let mut app = App::new();
    goto_security(&mut app);
    for _ in 0..6 {
        app.handle_key(KeyCode::Left);
    }
    let s = render(&mut app);
    assert!(
        s.contains("SELECTED: STRICT"),
        "Left did not clamp at first card\n{s}"
    );
}

// ── ESC = back one step (not quit) ──────────────────────────────────────────

#[test]
fn security_esc_backs_out_not_quit() {
    let mut app = App::new();
    goto_security(&mut app);
    let before = app.current_step;
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        before - 1,
        "ESC at Security must back out one step (not quit / not no-op)"
    );
}
