//! 1:1 render tests for onboarding Screen 13/19 — Features (FEAT) · JSX 1410–1466.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-features`:
//!   • header "Enable subsystems" + sub,
//!   • Talos macOS-gate warning banner ("⚠ MACOS GATE — TALOS IS MANDATORY" + body),
//!   • the 8 feature toggles (Talos/Nous/Mnemosyne/Hermes/Athena/Browser/Voice/Skill marketplace),
//!   • Talos FORCE-ON on macOS: reads ● ON regardless of toggle, FORCE-ON pill,
//!     amber warning line, non-toggleable,
//!   • Space/Enter toggle the focused subsystem (skip mandatory),
//!   • ↑/↓ move focus; ←/→ step-advance normally (Features is NOT a grid),
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const FEATURES_STEP: usize = 13;

/// Walk to the Features step (12). Steps 1/6/8/9/11 consume Right/Enter for
/// in-screen focus (grid-local), so we bump those directly — mirrors goto_step
/// in onb_106. Features itself is the target; we stop AT it.
fn goto_features(app: &mut App) {
    while app.current_step < FEATURES_STEP {
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
        app.current_step, FEATURES_STEP,
        "failed to reach Features step"
    );
}

fn render(app: &mut App) -> String {
    let backend = TestBackend::new(140, 60);
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

// ── header + sub ────────────────────────────────────────────────────────────

#[test]
fn features_header_and_sub() {
    let mut app = App::new();
    goto_features(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Enable subsystems"), "missing header\n{s}");
    assert!(
        s.contains("Toggle which Zeus crates are active"),
        "missing sub\n{s}"
    );
}

// ── Talos macOS-gate banner ──────────────────────────────────────────────────

#[test]
fn features_talos_banner_present() {
    let mut app = App::new();
    goto_features(&mut app);
    let s = render(&mut app);
    assert!(
        s.contains("⚠ MACOS GATE — TALOS IS MANDATORY"),
        "missing banner title (with ⚠ prefix)\n{s}"
    );
    assert!(
        s.contains("[talos] block must be present"),
        "missing banner body\n{s}"
    );
    assert!(
        s.contains("193 tools"),
        "missing banner 193-tools line\n{s}"
    );
}

// ── the 8 toggles ────────────────────────────────────────────────────────────

#[test]
fn features_all_eight_present() {
    let mut app = App::new();
    goto_features(&mut app);
    let s = render(&mut app);
    for name in [
        "Talos",
        "Nous",
        "Mnemosyne",
        "Hermes",
        "Athena",
        "Browser",
        "Voice",
        "Skill marketplace",
    ] {
        assert!(s.contains(name), "missing feature {name}\n{s}");
    }
}

// ── Talos FORCE-ON on macOS ──────────────────────────────────────────────────

#[test]
fn features_talos_force_on_macos() {
    let mut app = App::new();
    goto_features(&mut app);
    // Default platform is macOS → talos is mandatory.
    app.features_screen.platform = "macOS";
    let s = render(&mut app);
    // FORCE-ON pill shows the platform.
    assert!(
        s.contains("FORCE-ON ON MACOS"),
        "missing FORCE-ON pill\n{s}"
    );
    // Mandatory amber warning line renders.
    assert!(
        s.contains("macOS gate — without this"),
        "missing talos amber warning\n{s}"
    );
}

#[test]
fn features_talos_reads_on_even_when_toggle_false() {
    let mut app = App::new();
    goto_features(&mut app);
    app.features_screen.platform = "macOS";
    // Default toggled[0] (talos) is false — but mandatory must override → ● ON.
    assert!(
        !app.features_screen.toggled[0],
        "precondition: talos toggle is false"
    );
    let s = render(&mut app);
    // The talos row must show ● ON (not ○ OFF), proving is_mandatory || toggled.
    let talos_row = s
        .lines()
        .position(|l| l.contains("Talos"))
        .expect("Talos row");
    // ● ON appears in the banding of the talos card (its row + status row).
    let band: String = s
        .lines()
        .skip(talos_row)
        .take(4)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        band.contains("● ON"),
        "talos must read ● ON when mandatory\n{band}"
    );
}

#[test]
fn features_talos_non_toggleable_on_macos() {
    let mut app = App::new();
    goto_features(&mut app);
    app.features_screen.platform = "macOS";
    app.features_screen.focused = 0; // talos
    let before = app.features_screen.toggled[0];
    app.handle_key(KeyCode::Char(' ')); // try to toggle
    assert_eq!(
        app.features_screen.toggled[0], before,
        "talos must be non-toggleable on macOS"
    );
}

// ── Talos is a NORMAL toggle off-macOS (allowed diff) ────────────────────────

#[test]
fn features_talos_toggleable_off_macos() {
    let mut app = App::new();
    goto_features(&mut app);
    app.features_screen.platform = "linux";
    app.features_screen.focused = 0; // talos
    let before = app.features_screen.toggled[0];
    app.handle_key(KeyCode::Char(' '));
    assert_ne!(
        app.features_screen.toggled[0], before,
        "talos must toggle normally off-macOS"
    );
    // And no FORCE-ON pill off-macOS.
    let s = render(&mut app);
    assert!(
        !s.contains("FORCE-ON ON LINUX"),
        "FORCE-ON pill must not show off-macOS\n{s}"
    );
}

// ── Space / Enter toggle a non-mandatory subsystem ───────────────────────────

#[test]
fn features_space_toggles_focused() {
    let mut app = App::new();
    goto_features(&mut app);
    app.features_screen.focused = 1; // nous (non-mandatory)
    let before = app.features_screen.toggled[1];
    app.handle_key(KeyCode::Char(' '));
    assert_ne!(
        app.features_screen.toggled[1], before,
        "Space must toggle the focused subsystem"
    );
}

#[test]
fn features_enter_advances_not_toggles() {
    // merakizzz nav-UX fix: on grid screens Space toggles, Enter ADVANCES.
    // Enter must NOT toggle the focused subsystem — it moves to the next step.
    let mut app = App::new();
    goto_features(&mut app);
    let step = app.current_step;
    app.features_screen.focused = 5; // browser (non-mandatory, default off)
    let before = app.features_screen.toggled[5];
    app.handle_key(KeyCode::Enter);
    assert_eq!(
        app.features_screen.toggled[5], before,
        "Enter must NOT toggle — Space owns toggle on grid screens now"
    );
    assert_eq!(
        app.current_step,
        step + 1,
        "Enter on Features must advance the flow"
    );
}

// ── ↑/↓ move focus ───────────────────────────────────────────────────────────

#[test]
fn features_up_down_move_focus() {
    let mut app = App::new();
    goto_features(&mut app);
    app.features_screen.focused = 0;
    app.handle_key(KeyCode::Down);
    assert_eq!(app.features_screen.focused, 1, "Down should move focus +1");
    app.handle_key(KeyCode::Up);
    assert_eq!(app.features_screen.focused, 0, "Up should move focus -1");
}

// ── ←/→ step-advance (Features is NOT a grid) ────────────────────────────────

#[test]
fn features_right_advances_step_not_grid() {
    let mut app = App::new();
    goto_features(&mut app);
    let before_focus = app.features_screen.focused;
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step,
        FEATURES_STEP + 1,
        "Right must step-advance (Features is not a grid)"
    );
    assert_eq!(
        app.features_screen.focused, before_focus,
        "Right must NOT move in-screen focus"
    );
}

// ── ESC = back one step, not quit ────────────────────────────────────────────

#[test]
fn features_esc_backs_out_not_quit() {
    let mut app = App::new();
    goto_features(&mut app);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        FEATURES_STEP - 1,
        "ESC must back out one step (not quit)"
    );
}
