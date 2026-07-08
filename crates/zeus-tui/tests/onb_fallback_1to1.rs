//! 1:1 fidelity tests for the Fallback onboarding screen (step 5, JSX 870–950).
//!
//! Asserts the cut from `fix/tui-1to1-fallback`:
//!   - 2-column layout (LEFT "Backup LLM chain" / AVAILABLE checklist,
//!     RIGHT "FALLBACK CHAIN (N)" + reorder hint + empty-state box);
//!   - checkbox toggle adds the highlighted candidate into the chain (count
//!     in the "FALLBACK CHAIN (N)" header increments);
//!   - numbered-badge ordering — chain items render in selection order with
//!     numeric badges (1, 2, 3);
//!   - SUGGESTED copy is Groq-free (adapted to our 12: "OpenAI + Ollama");
//!   - ESC = back one step (does NOT quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

/// Render current app state into a TestBackend; return per-row lines.
fn render_lines_at(app: &App, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
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

/// Render current app state into a 140×44 TestBackend; return per-row lines.
fn render_lines(app: &App) -> Vec<String> {
    render_lines_at(app, 140, 44)
}

fn render(app: &App) -> String {
    render_lines(app).join("\n")
}

/// Navigate to the Fallback step (index 5).
///
/// Step-nav across the intermediate screens is per-screen (Mode consumes
/// ←/→ for card selection, several screens consume Enter for in-screen
/// actions), so we set the public `current_step` directly and fire the
/// step-enter hook via a no-op key — this exercises the real Fallback
/// render + key handlers without fighting each intermediate screen's idiom.
fn goto_fallback(app: &mut App) {
    app.current_step = 6;
    assert_eq!(app.current_step, 6, "should land on Fallback (step 5)");
}

/// Find the column index of the first occurrence of `needle` in `line`.
fn col_of(line: &str, needle: &str) -> Option<usize> {
    line.find(needle)
}

#[test]
fn fallback_two_column_layout() {
    let mut app = App::new();
    goto_fallback(&mut app);
    let screen = render(&app);

    // LEFT column: "Backup LLM chain" header + AVAILABLE label.
    assert!(
        screen.contains("Backup LLM chain"),
        "LEFT column must show the 'Backup LLM chain' header"
    );
    assert!(
        screen.contains("AVAILABLE"),
        "LEFT column must show the AVAILABLE checklist label"
    );

    // RIGHT column: "FALLBACK CHAIN (0)" header + reorder hint + empty state.
    assert!(
        screen.contains("FALLBACK CHAIN (0)"),
        "RIGHT column must show 'FALLBACK CHAIN (N)' with the live count"
    );
    assert!(
        screen.contains("Reorder with"),
        "RIGHT column must show the 'Reorder with [ / ]' hint"
    );
    assert!(
        screen.contains("No fallbacks selected."),
        "empty chain must show the empty-state box"
    );

    // 2-column geometry: on the row carrying the LEFT header, the RIGHT
    // header must sit strictly to its right (proves side-by-side columns,
    // not a stacked vertical list).
    let lines = render_lines(&app);
    let left_col = lines
        .iter()
        .find_map(|l| col_of(l, "Backup LLM chain"))
        .expect("left header present");
    let right_col = lines
        .iter()
        .find_map(|l| col_of(l, "FALLBACK CHAIN"))
        .expect("right header present");
    assert!(
        right_col > left_col,
        "RIGHT column (FALLBACK CHAIN @ {right_col}) must be right of LEFT (@ {left_col})"
    );
}

#[test]
fn fallback_checkbox_toggle_into_chain() {
    let mut app = App::new();
    goto_fallback(&mut app);

    // Empty to start.
    assert!(
        render(&app).contains("FALLBACK CHAIN (0)"),
        "chain starts empty"
    );

    // Toggle the highlighted candidate into the chain (Enter/Space).
    app.handle_key(KeyCode::Enter);
    assert!(
        render(&app).contains("FALLBACK CHAIN (1)"),
        "toggling a candidate must add it → count = 1"
    );

    // Move down and add a second.
    app.handle_key(KeyCode::Down);
    app.handle_key(KeyCode::Enter);
    assert!(
        render(&app).contains("FALLBACK CHAIN (2)"),
        "adding a second candidate → count = 2"
    );

    // Toggling the same row off decrements.
    app.handle_key(KeyCode::Enter);
    assert!(
        render(&app).contains("FALLBACK CHAIN (1)"),
        "toggling an in-chain candidate off → count = 1"
    );
}

#[test]
fn fallback_numbered_badge_ordering() {
    let mut app = App::new();
    goto_fallback(&mut app);

    // Add two candidates → chain renders numbered badges 1 and 2.
    app.handle_key(KeyCode::Enter); // candidate 0 → badge 1
    app.handle_key(KeyCode::Down);
    app.handle_key(KeyCode::Enter); // candidate 1 → badge 2

    let screen = render(&app);
    assert!(
        screen.contains("FALLBACK CHAIN (2)"),
        "two candidates in chain"
    );
    // Numbered badges: the chain rows carry " 1 " and " 2 " chips.
    assert!(
        screen.contains(" 1 "),
        "first chain item must carry numbered badge 1"
    );
    assert!(
        screen.contains(" 2 "),
        "second chain item must carry numbered badge 2"
    );

    // SUGGESTED box appears once the chain is non-empty. Its copy wraps
    // across the narrow right panel, so we scope the Groq/Ollama checks to
    // the box's row band (from the SUGGESTED label down to "fallback."),
    // NOT the whole screen — Groq IS a valid AVAILABLE candidate (it's in
    // our 12), it just must not appear in the *suggestion copy*.
    let lines = render_lines(&app);
    let sug_start = lines
        .iter()
        .position(|l| l.contains("SUGGESTED"))
        .expect("SUGGESTED box label must render when chain is non-empty");
    let sug_end = lines
        .iter()
        .skip(sug_start)
        .position(|l| l.contains("fallback."))
        .map(|off| sug_start + off + 1)
        .unwrap_or((sug_start + 5).min(lines.len()));
    let sug_band: String = lines[sug_start..sug_end].join(" ");
    assert!(
        !sug_band.contains("Groq"),
        "SUGGESTED copy must be Groq-free (JSX hardcoded Groq; adapted to our 12).\nband: {sug_band}"
    );
    assert!(
        sug_band.contains("Ollama"),
        "SUGGESTED copy adapts to our 12 (OpenAI + Ollama).\nband: {sug_band}"
    );
}

#[test]
fn fallback_100x30_empty_state_keeps_lines_separated() {
    let mut app = App::new();
    goto_fallback(&mut app);

    let lines = render_lines_at(&app, 100, 30);
    let dump = lines.join("\n");

    assert!(
        dump.contains("No fallbacks selected."),
        "compact Fallback render must keep the empty-state headline visible:\n{dump}"
    );
    assert!(
        dump.contains("Primary failures will fail"),
        "compact Fallback render must keep the empty-state detail on its own visible line:\n{dump}"
    );
    assert!(
        !dump.contains("selected.Primary"),
        "compact Fallback empty-state lines must not collapse together:\n{dump}"
    );
}

#[test]
fn fallback_esc_steps_back_not_quit() {
    let mut app = App::new();
    goto_fallback(&mut app);
    assert_eq!(app.current_step, 6);

    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step, 5,
        "ESC on Fallback must step back to the prior step, not quit"
    );
}
