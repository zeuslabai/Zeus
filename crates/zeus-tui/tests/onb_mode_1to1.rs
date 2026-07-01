//! 1:1 fidelity tests for the Mode onboarding screen (step 1, JSX 509–576).
//!
//! Asserts the known-bug fixes from `fix/tui-1to1-mode`:
//!   - the three mode cards render as a 3-COLUMN HORIZONTAL GRID (all three
//!     on the same rows, side by side — NOT a vertical list);
//!   - ←/→ move the card selection (and do NOT change the step);
//!   - the `▸ SELECTED` badge tracks the selected card;
//!   - the NOTE box renders;
//!   - ESC = back one step (does NOT quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

/// Render current app state into a 120×40 TestBackend; return per-row lines.
fn render_lines(app: &App) -> Vec<String> {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row);
    }
    lines
}

fn render(app: &App) -> String {
    render_lines(app).join("\n")
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for k in keys {
        app.handle_key(*k);
    }
}

/// Drive Welcome (step 0) → Mode (step 1) via one step-nav Right.
fn goto_mode(app: &mut App) {
    let mut guard = 0;
    while app.current_step < 1 && guard < 10 {
        if app.current_step == 3 { app.current_step += 1; app.on_step_enter(); continue; }        app.handle_key(KeyCode::Right);
        guard += 1;
    }
    assert_eq!(app.current_step, 1, "should land on Mode (step 1)");
}

// ---- 3-column horizontal GRID ---------------------------------------------

#[test]
fn mode_cards_render_as_3col_horizontal_grid() {
    let mut app = App::new();
    goto_mode(&mut app);
    let lines = render_lines(&app);

    // All three card names must appear, and on the SAME row (side by side) —
    // that is the defining property of a horizontal grid vs a vertical list.
    let row_with_all_three = lines.iter().find(|row| {
        row.contains("QuickStart") && row.contains("Full Setup") && row.contains("Custom")
    });
    assert!(
        row_with_all_three.is_some(),
        "all three mode names must share one row (3-col GRID, not a vertical list).\n--- screen ---\n{}",
        lines.join("\n")
    );

    // And their column order must be left→right QuickStart < Full Setup < Custom.
    let row = row_with_all_three.unwrap();
    let qs = row.find("QuickStart").unwrap();
    let fu = row.find("Full Setup").unwrap();
    let cu = row.find("Custom").unwrap();
    assert!(qs < fu && fu < cu, "cards must be ordered QuickStart → Full Setup → Custom left-to-right");
}

// ---- ←/→ selection moves (not step-nav) -----------------------------------

#[test]
fn mode_right_left_move_selection_not_step() {
    let mut app = App::new();
    goto_mode(&mut app);

    // Selection starts at QuickStart (0); SELECTED badge sits on its column.
    let row = selected_badge_row(&render_lines(&app)).expect("SELECTED badge must render");
    let qs_col = row.find("QuickStart").map(|_| col_of(&row, "\u{25b8} SELECTED"));

    // Right → selection moves to Full Setup; step unchanged.
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.current_step, 1, "Right on Mode must NOT advance the step");
    let after = selected_badge_row(&render_lines(&app)).expect("SELECTED badge after Right");
    let fu_badge_col = col_of(&after, "\u{25b8} SELECTED");
    assert!(
        qs_col.unwrap_or(0) < fu_badge_col,
        "→ must move the SELECTED badge rightward (QuickStart → Full Setup)"
    );

    // Right again → Custom (rightmost). Left → back to Full Setup.
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.current_step, 1);
    let custom_col = col_of(
        &selected_badge_row(&render_lines(&app)).unwrap(),
        "\u{25b8} SELECTED",
    );
    assert!(custom_col > fu_badge_col, "→ again moves to Custom (rightmost card)");

    press(&mut app, &[KeyCode::Left]);
    assert_eq!(app.current_step, 1, "Left on Mode must NOT change the step");
    let back_col = col_of(
        &selected_badge_row(&render_lines(&app)).unwrap(),
        "\u{25b8} SELECTED",
    );
    assert!(back_col < custom_col, "← moves the selection back leftward");
}

// ---- NOTE box --------------------------------------------------------------

#[test]
fn mode_renders_note_box() {
    let mut app = App::new();
    goto_mode(&mut app);
    let screen = render(&app);
    assert!(screen.contains("NOTE"), "NOTE box label must render");
    assert!(
        screen.contains("zeus onboard --resume"),
        "NOTE box must mention `zeus onboard --resume`"
    );
}

// ---- ESC = back one step (does NOT quit) ----------------------------------

#[test]
fn mode_esc_goes_back_one_step_not_quit() {
    let mut app = App::new();
    goto_mode(&mut app);
    assert_eq!(app.current_step, 1);

    // ESC steps back (Mode → Welcome). A quit would leave the step unchanged
    // and exit instead; the decrement is the observable proof it did NOT quit.
    app.handle_key(KeyCode::Esc);
    assert_eq!(app.current_step, 0, "ESC on Mode must step back to Welcome (0), not quit");

    // ESC on the first step is a no-op (clamped) — no underflow, no quit.
    app.handle_key(KeyCode::Esc);
    assert_eq!(app.current_step, 0, "ESC on step 0 clamps (no underflow)");
}

// ---- helpers ---------------------------------------------------------------

/// Find the rendered row containing the `▸ SELECTED` badge.
fn selected_badge_row(lines: &[String]) -> Option<String> {
    lines.iter().find(|r| r.contains("\u{25b8} SELECTED")).cloned()
}

/// Column index of `needle` within `row` (panics if absent — caller asserts).
fn col_of(row: &str, needle: &str) -> usize {
    row.find(needle).expect("needle present in row")
}
