//! 1:1 render tests for onboarding Screen 11/19 — Workspace (WKSP) · JSX 1309–1352.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-workspace`:
//!   • header "Workspace paths" + sub,
//!   • PATHS section label,
//!   • 3 path fields (Workspace / Sessions / Mnemosyne DB) with correct
//!     defaults + JSX-exact hints,
//!   • DISK USAGE PROJECTION box (3-col: Workspace ~50MB / Sessions ~150MB /
//!     Mnemosyne ~800MB with sub-labels), inside a bordered box,
//!   • existing-workspace amber box (standalone render): "↻ EXISTING WORKSPACE
//!     FOUND" header + path-prefixed counts + "last modified" + USE EXISTING /
//!     START FRESH (BACKUP OLD) buttons,
//!   • Right step-advances at the Workspace step (NOT grid-local — fields use
//!     ↑/↓/Tab),
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::widgets::Widget;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::screens::WorkspaceScreen;

use crossterm::event::KeyCode;

const WORKSPACE_STEP: usize = 11;

/// Walk to the Workspace step. Grid steps consume Right for in-screen focus, so
/// we bump those directly — mirrors goto_step in onb_106. Workspace lives PAST
/// both grid steps (Channels=6, Gateway=8), so both must be special-cased or
/// the walk hangs (Right is grid-local there and never advances the step).
fn goto_workspace(app: &mut App) {
    while app.current_step < WORKSPACE_STEP {
        if app.current_step == 4 {
            app.current_step += 1;
            app.on_step_enter();
            continue;
        }
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 7 || s == 9 {
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
        app.current_step, WORKSPACE_STEP,
        "failed to reach Workspace step"
    );
}

/// Render the current app into a 120×44 TestBackend → newline-joined String.
fn render(app: &App) -> String {
    render_at(app, 120, 44)
}

/// Render the current app at a specific terminal size.
fn render_at(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    buf_to_string(terminal.backend().buffer().clone())
}

/// Render a standalone WorkspaceScreen into a TestBackend → String. Used to
/// exercise the existing-workspace amber box (app default has it off).
fn render_screen(screen: WorkspaceScreen) -> String {
    let backend = TestBackend::new(120, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let area = f.area();
            screen.render(area, f.buffer_mut());
        })
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

// ── always-present layout (via full-app walk) ──────────────────────────────

#[test]
fn workspace_header_and_paths_label() {
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render(&app);
    assert!(s.contains("Workspace paths"), "missing header\n{s}");
    assert!(
        s.contains("Where Zeus stores your agent's working memory"),
        "missing subtitle\n{s}"
    );
    assert!(s.contains("PATHS"), "missing PATHS section label\n{s}");
}

#[test]
fn workspace_100x30_uses_single_app_header() {
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render_at(&app, 100, 30);

    assert_eq!(
        s.matches("Workspace paths").count(),
        1,
        "Workspace title should come from the app StepHeader only\n{s}"
    );
    assert_eq!(
        s.matches("Where Zeus stores your agent's working memory")
            .count(),
        1,
        "Workspace subtitle should not be duplicated inside the body\n{s}"
    );
    assert!(
        s.contains("PATHS"),
        "missing PATHS section label at 100x30\n{s}"
    );
    assert!(
        s.contains("DISK USAGE PROJECTION"),
        "missing disk projection at 100x30\n{s}"
    );
}

#[test]
fn workspace_three_path_fields_and_defaults() {
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render(&app);
    // field labels
    assert!(s.contains("Workspace"), "missing Workspace field\n{s}");
    assert!(s.contains("Sessions"), "missing Sessions field\n{s}");
    assert!(
        s.contains("Mnemosyne DB"),
        "missing Mnemosyne DB field\n{s}"
    );
    // defaults
    assert!(
        s.contains("~/.zeus/workspace"),
        "missing workspace default\n{s}"
    );
    assert!(
        s.contains("~/.zeus/sessions"),
        "missing sessions default\n{s}"
    );
    assert!(
        s.contains("~/.zeus/mnemosyne.db"),
        "missing mnemosyne default\n{s}"
    );
}

#[test]
fn workspace_field_hints_match_jsx() {
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render(&app);
    assert!(
        s.contains("AGENTS.md, SOUL.md, journals, daily notes"),
        "missing workspace hint\n{s}"
    );
    assert!(
        s.contains("Per-conversation JSONL logs"),
        "missing/stale sessions hint (must be JSONL copy, not 'transcripts')\n{s}"
    );
    assert!(
        s.contains("SQLite + vector embeddings"),
        "missing mnemosyne hint\n{s}"
    );
    // the stale pre-fix sessions copy must be gone
    assert!(
        !s.contains("Session transcripts and state snapshots"),
        "stale sessions hint still present\n{s}"
    );
}

#[test]
fn workspace_disk_usage_projection() {
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render(&app);
    assert!(
        s.contains("DISK USAGE PROJECTION"),
        "missing disk box header\n{s}"
    );
    // 3-col values + sub-labels
    assert!(s.contains("~50 MB"), "missing workspace projection\n{s}");
    assert!(s.contains("~150 MB"), "missing sessions projection\n{s}");
    assert!(s.contains("~800 MB"), "missing mnemosyne projection\n{s}");
    assert!(s.contains("after 30 days"), "missing workspace sub\n{s}");
    assert!(
        s.contains("@ 5 MB/day for 30d"),
        "missing sessions sub\n{s}"
    );
    assert!(
        s.contains("after 1000 sessions"),
        "missing mnemosyne sub\n{s}"
    );
}

// ── existing-workspace amber box (standalone screen render) ─────────────────

#[test]
fn workspace_existing_amber_box() {
    let screen = WorkspaceScreen {
        existing_detected: true,
        memory_facts: 2847,
        session_count: 147,
        existing_mtime: "2 minutes ago".to_string(),
        ..WorkspaceScreen::new()
    };
    let s = render_screen(screen);
    assert!(
        s.contains("↻ EXISTING WORKSPACE FOUND"),
        "missing amber box header (with ↻ glyph)\n{s}"
    );
    // path-prefixed body + counts + last-modified
    assert!(
        s.contains("~/.zeus/workspace contains"),
        "missing path-prefixed body\n{s}"
    );
    assert!(s.contains("2847"), "missing memory-facts count\n{s}");
    assert!(s.contains("147"), "missing sessions count\n{s}");
    assert!(
        s.contains("last modified"),
        "missing last-modified copy\n{s}"
    );
    // buttons
    assert!(
        s.contains("USE EXISTING"),
        "missing USE EXISTING button\n{s}"
    );
    assert!(
        s.contains("START FRESH (BACKUP OLD)"),
        "missing START FRESH button\n{s}"
    );
}

#[test]
fn workspace_amber_box_hidden_by_default() {
    // App default has no existing workspace → amber box absent.
    let mut app = App::new();
    goto_workspace(&mut app);
    let s = render(&app);
    assert!(
        !s.contains("EXISTING WORKSPACE FOUND"),
        "amber box should be hidden when no existing workspace\n{s}"
    );
}

// ── nav: Right step-advances (fields, not grid), ESC backs out ─────────────

#[test]
fn workspace_right_advances_step_not_grid() {
    let mut app = App::new();
    goto_workspace(&mut app);
    assert_eq!(app.current_step, WORKSPACE_STEP);
    // Workspace is fields-based (↑/↓/Tab) → Right should step-advance.
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step,
        WORKSPACE_STEP + 1,
        "Right at Workspace must step-advance (no grid nav here)"
    );
}

#[test]
fn workspace_esc_backs_out_one_step() {
    let mut app = App::new();
    goto_workspace(&mut app);
    assert_eq!(app.current_step, WORKSPACE_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        WORKSPACE_STEP - 1,
        "ESC at Workspace must back out one step (not quit)"
    );
}
