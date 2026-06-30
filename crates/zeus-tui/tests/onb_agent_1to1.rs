//! 1:1 render tests for onboarding Screen 10/19 — Agent (AGNT) · JSX 1263–1308.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-agent`:
//!   • 2-col persona GRID (6 cards, 3 rows) with ←/→/↑/↓ navigation,
//!   • grid edges clamp (no wrap into the wrong row/col),
//!   • live SOUL.md preview reflects the SELECTED persona (role/tone/principles),
//!   • IDENTITY name auto-suggest + role/tone fallbacks render,
//!   • ESC backs out one step (does not quit),
//!   • multibyte name input is panic-free.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const AGENT_STEP: usize = 9;

/// Walk to the Agent step. Grid steps (Channels=6, Agent=9) consume Right for
/// in-screen focus, so we bump those directly — mirrors goto_step in onb_106.
fn goto_agent(app: &mut App) {
    while app.current_step < AGENT_STEP {
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 6 || s == 8 {
            // Channels (6) and Gateway (8) consume Right for in-screen grid
            // focus → bump directly past them (Gateway became grid-local once
            // its 4-col service picker was wired). Without the s==8 case this
            // walk hangs on the way to Agent (9).
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
    }
    assert_eq!(app.current_step, AGENT_STEP, "failed to reach Agent step");
    // Pin the picker to the offline default persona set (the JSX prototype's
    // fixed 6 cards). #247 wired the picker to load the real on-disk persona
    // library; these are 1:1 *layout* tests that assert the prototype grid, so
    // they must exercise the offline fallback, not whatever lives on disk in CI.
    app.agent_screen.use_default_personas_for_test();
}

/// Render the current app into a 120×40 TestBackend → newline-joined String.
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

// ── persona grid: all 6 render, 2-col layout ───────────────────────────────

#[test]
fn agent_renders_all_six_personas_and_headers() {
    let mut app = App::new();
    goto_agent(&mut app);
    let screen = render(&app);
    // Header + sub.
    assert!(screen.contains("Agent persona"), "persona header missing");
    assert!(screen.contains("SOUL.md"), "SOUL.md sub-copy missing");
    // All 6 persona glyphs (the 2-col grid).
    for glyph in ["COO", "ENG", "CRT", "OPS", "ANL", "CST"] {
        assert!(screen.contains(glyph), "persona glyph {glyph} missing from grid");
    }
    // IDENTITY section + SOUL preview box + footer target.
    assert!(screen.contains("IDENTITY"), "IDENTITY section missing");
    assert!(screen.contains("SOUL.MD PREVIEW"), "SOUL preview header missing");
    // Footer renders the write-path. At a 46-col preview the full path clips,
    // so assert on the visible head ("writes to ~/.zeus…") — proves the footer
    // is present without over-asserting on column-clipped text.
    assert!(
        screen.contains("writes to ~/.zeus"),
        "SOUL.md write-path footer missing"
    );
}

// ── grid nav: ←/→/↑/↓ move the selection (not the step) ────────────────────

#[test]
fn agent_grid_right_moves_column_not_step() {
    let mut app = App::new();
    goto_agent(&mut app);
    assert_eq!(app.agent_screen.persona_idx, 0);
    app.handle_key(KeyCode::Right); // col 0 → col 1 (idx 1)
    assert_eq!(app.agent_screen.persona_idx, 1, "Right should move column");
    assert_eq!(app.current_step, AGENT_STEP, "Right must NOT advance the step");
}

#[test]
fn agent_grid_down_moves_row_by_two() {
    let mut app = App::new();
    goto_agent(&mut app);
    app.handle_key(KeyCode::Down); // row 0 → row 1 (idx 0 → 2)
    assert_eq!(app.agent_screen.persona_idx, 2, "Down should move one grid row (+2)");
    app.handle_key(KeyCode::Down); // row 1 → row 2 (idx 2 → 4)
    assert_eq!(app.agent_screen.persona_idx, 4);
    app.handle_key(KeyCode::Up); // back up a row
    assert_eq!(app.agent_screen.persona_idx, 2, "Up should move back a grid row (−2)");
}

#[test]
fn agent_grid_edges_clamp_no_wrap() {
    let mut app = App::new();
    goto_agent(&mut app);
    // Left at col 0 stays put (no wrap to previous row's col 1).
    app.handle_key(KeyCode::Left);
    assert_eq!(app.agent_screen.persona_idx, 0, "Left at col0 must clamp");
    // Up at row 0 stays put.
    app.handle_key(KeyCode::Up);
    assert_eq!(app.agent_screen.persona_idx, 0, "Up at row0 must clamp");
    // Walk to the last persona (idx 5 = Custom, row2/col1) and try to overshoot.
    app.handle_key(KeyCode::Down); // 0→2
    app.handle_key(KeyCode::Down); // 2→4
    app.handle_key(KeyCode::Right); // 4→5
    assert_eq!(app.agent_screen.persona_idx, 5);
    app.handle_key(KeyCode::Down); // row2 is last → clamp
    assert_eq!(app.agent_screen.persona_idx, 5, "Down at last row must clamp");
    app.handle_key(KeyCode::Right); // col1 is last → clamp
    assert_eq!(app.agent_screen.persona_idx, 5, "Right at col1 must clamp");
}

// ── live SOUL preview reflects the selected persona ────────────────────────

#[test]
fn agent_soul_preview_reflects_selection() {
    let mut app = App::new();
    goto_agent(&mut app);
    // Default = Coordinator (idx 0): role "Coordinator", tone "...decisive".
    let coord = render(&app);
    assert!(coord.contains("## Role"), "SOUL Role section missing");
    assert!(coord.contains("Coordinator"), "default persona role not in preview");
    assert!(
        coord.contains("decisive"),
        "Coordinator tone should seed the SOUL preview"
    );
    assert!(
        coord.contains("Make decisions quickly"),
        "Coordinator guiding principle missing from preview"
    );

    // Move to Engineer (idx 1, col1) → preview must switch role+tone+principles.
    app.handle_key(KeyCode::Right);
    assert_eq!(app.agent_screen.persona_idx, 1);
    let eng = render(&app);
    assert!(eng.contains("Engineer"), "Engineer role not in preview after select");
    assert!(
        eng.contains("Read existing code before writing new"),
        "Engineer guiding principle missing — preview not live"
    );
    assert!(
        !eng.contains("Make decisions quickly"),
        "stale Coordinator principle leaked into Engineer preview"
    );
}

// ── name auto-suggest + multibyte-safe input ───────────────────────────────

#[test]
fn agent_name_autosuggest_and_multibyte_input() {
    let mut app = App::new();
    goto_agent(&mut app);
    // Auto-suggested name (zeus{host} or Zeus100) appears in the SOUL `# {name}`.
    let pre = render(&app);
    assert!(
        pre.contains("# zeus") || pre.contains("# Zeus100"),
        "auto-suggested agent name missing from SOUL header"
    );
    // Typing into the focused name field is panic-free on multibyte.
    for c in "Athéna⚡".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    let post = render(&app);
    assert!(post.contains("Athéna"), "typed multibyte name should render");
}

// ── ESC backs out one step (does not quit) ─────────────────────────────────

#[test]
fn agent_esc_backs_out_one_step() {
    let mut app = App::new();
    goto_agent(&mut app);
    assert_eq!(app.current_step, AGENT_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        AGENT_STEP - 1,
        "ESC must back out one step, not quit"
    );
}
