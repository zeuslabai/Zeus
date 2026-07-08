//! 1:1 render tests for onboarding Screen 18/19 — Skills (SKIL) · JSX 1654–1737.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-skills`:
//!   • header "Install starter skills" + sub copy,
//!   • filter input box (`/` prefix + "filter..." placeholder; live text),
//!   • category tabs: All + the 5 (Productivity/Dev/Marketing/Security/Research)
//!     with per-category `(N)` counts on the non-All tabs,
//!   • right-aligned "N selected · M available" summary,
//!   • 2-col grid of skill cards (checkbox + name + ★ REC + category tag),
//!   • Space/Enter toggle install (count tracks),
//!   • ←/→ switch the category tab (grid-local, NOT step-nav) → Skills (step 17)
//!     requires the goto_step s==18 cascade (proven by the full suite not hanging),
//!   • typing feeds the live filter; Backspace pops it (char-safe),
//!   • empty-state when the filter matches nothing,
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const SKILLS_STEP: usize = 18;

/// Walk to the Skills step (17). Mirrors the goto_step cascade: Skills is a GRID
/// (←/→ switch category tabs), so the walker must bump past it directly rather
/// than firing Right (which would switch a tab, not advance the step).
fn goto_skills(app: &mut App) {
    let mut guard = 0;
    while app.current_step < SKILLS_STEP {
        if app.current_step == 4 {
            app.current_step += 1;
            app.on_step_enter();
            continue;
        }
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 7 || s == 9 || s == 10 || s == 12 || s == 16 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
        guard += 1;
        assert!(guard < 100, "goto_skills stalled before reaching Skills");
    }
    assert_eq!(app.current_step, SKILLS_STEP, "failed to reach Skills step");
}

fn render(app: &mut App) -> String {
    render_size(app, 140, 44)
}

fn render_size(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
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
fn skills_header_and_sub() {
    let mut app = App::new();
    goto_skills(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Install starter skills"), "missing header\n{s}");
    assert!(
        s.contains("SKILL.md plugins from the registry"),
        "missing sub copy\n{s}"
    );
}

// ── filter input ──────────────────────────────────────────────────────────────

#[test]
fn skills_filter_placeholder_renders() {
    let mut app = App::new();
    goto_skills(&mut app);
    let s = render(&mut app);
    // `/` prefix + "filter..." placeholder when the filter is empty.
    assert!(s.contains("filter..."), "missing filter placeholder\n{s}");
}

/// Deterministic skill fixture so render-fidelity tests don't depend on the
/// host's live `~/.zeus/skills` (which #247 now loads in production). Mirrors
/// the original proto entries the 1:1 assertions were written against.
const TEST_FIXTURE: &[(&str, &str, &str, &str, bool)] = &[
    (
        "calendar-pro",
        "Calendar Pro",
        "Auto-schedule + conflict detection",
        "Productivity",
        true,
    ),
    (
        "email-triage",
        "Email Triage",
        "Inbox prioritization",
        "Productivity",
        true,
    ),
    (
        "todo-sync",
        "Todo Sync",
        "Cross-platform task sync",
        "Productivity",
        false,
    ),
    (
        "git-flow",
        "Git Flow",
        "Branch + PR automation",
        "Dev",
        true,
    ),
    (
        "ci-watch",
        "CI Watch",
        "Pipeline monitoring + fixes",
        "Dev",
        true,
    ),
    (
        "openclaw-compat",
        "OpenClaw Compat",
        "Adds claw_* tools",
        "Dev",
        false,
    ),
    (
        "test-gen",
        "Test Gen",
        "Auto-generate test cases",
        "Dev",
        false,
    ),
];

fn seed_fixture(app: &mut App) {
    app.skills_screen.set_test_skills(TEST_FIXTURE);
}

#[test]
fn skills_filter_typing_shows_text_and_narrows_grid() {
    let mut app = App::new();
    goto_skills(&mut app);
    seed_fixture(&mut app);
    // Type "git" → should match "Git Flow" only, hide non-matches like
    // "Calendar Pro".
    for c in "git".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    let s = render(&mut app);
    assert!(s.contains("git"), "filter text not shown\n{s}");
    assert!(
        s.contains("Git Flow"),
        "matching skill should be visible\n{s}"
    );
    assert!(
        !s.contains("Calendar Pro"),
        "non-matching skill should be filtered out\n{s}"
    );
}

#[test]
fn skills_filter_backspace_is_char_safe() {
    let mut app = App::new();
    goto_skills(&mut app);
    // Multibyte filter input — Backspace must pop a codepoint, never panic.
    for c in "café".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    app.handle_key(KeyCode::Backspace); // drop 'é'
    let s = render(&mut app);
    assert!(
        s.contains("caf"),
        "filter should read 'caf' after backspace\n{s}"
    );
    // No panic on render = char-safe.
    assert!(!s.trim().is_empty());
}

#[test]
fn skills_filter_no_match_shows_empty_state() {
    let mut app = App::new();
    goto_skills(&mut app);
    for c in "zzzznomatch".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    let s = render(&mut app);
    assert!(
        s.contains("No skills match your filter"),
        "missing empty-state\n{s}"
    );
}

// ── category tabs + counts ────────────────────────────────────────────────────

#[test]
fn skills_all_five_category_tabs_present() {
    let mut app = App::new();
    goto_skills(&mut app);
    let s = render(&mut app);
    for tab in [
        "All",
        "Productivity",
        "Dev",
        "Marketing",
        "Security",
        "Research",
    ] {
        assert!(s.contains(tab), "missing category tab {tab}\n{s}");
    }
}

#[test]
fn skills_tabs_show_per_category_counts() {
    let mut app = App::new();
    goto_skills(&mut app);
    seed_fixture(&mut app);
    let s = render(&mut app);
    // Fixture: Productivity has 3 skills, Dev has 4 → tabs render `(3)`/`(4)`.
    assert!(
        s.contains("Productivity (3)"),
        "missing Productivity count\n{s}"
    );
    assert!(s.contains("Dev (4)"), "missing Dev count\n{s}");
    // "All" carries NO count (matches JSX).
    assert!(!s.contains("All ("), "All tab must not show a count\n{s}");
}

#[test]
fn skills_summary_count_tracks_installs() {
    let mut app = App::new();
    goto_skills(&mut app);
    seed_fixture(&mut app); // 7-skill deterministic fixture, all installed by default
    let s0 = render(&mut app);
    // #263: opt-OUT default — every fixture skill starts selected.
    assert!(
        s0.contains("7 selected"),
        "should start all 7 selected\n{s0}"
    );
    assert!(s0.contains("available"), "missing available summary\n{s0}");
    // Toggle the focused skill via Space OFF → 6 selected.
    app.handle_key(KeyCode::Char(' '));
    let s1 = render(&mut app);
    assert!(s1.contains("6 selected"), "uninstall not counted\n{s1}");
}

// ── 2-col grid + cards ────────────────────────────────────────────────────────

#[test]
fn skills_grid_shows_cards_with_rec_and_tag() {
    let mut app = App::new();
    goto_skills(&mut app);
    seed_fixture(&mut app);
    let s = render(&mut app);
    // A recommended skill in the All view shows its name + the ★ REC badge.
    assert!(s.contains("Calendar Pro"), "missing skill name\n{s}");
    assert!(s.contains("★ REC"), "missing recommended badge\n{s}");
    // Category tag (uppercase) renders on cards.
    assert!(s.contains("PRODUCTIVITY"), "missing category tag\n{s}");
}

#[test]
fn skills_100x30_cards_keep_prototype_blurbs() {
    let mut app = App::new();
    goto_skills(&mut app);
    seed_fixture(&mut app);
    let s = render_size(&mut app, 100, 30);

    assert!(
        s.contains("Calendar Pro"),
        "missing first skill card
{s}"
    );
    assert!(
        s.contains("Auto-schedule + conflict detection"),
        "100x30 card grid should retain the first skill blurb
{s}"
    );
    assert!(
        s.contains("Email Triage"),
        "missing second skill card
{s}"
    );
    assert!(
        s.contains("Inbox prioritization"),
        "100x30 card grid should retain the second skill blurb
{s}"
    );
}

// ── toggle install ────────────────────────────────────────────────────────────

#[test]
fn skills_enter_toggles_install() {
    let mut app = App::new();
    goto_skills(&mut app);
    // #263: all skills installed (selected) by default — opt-OUT contract.
    let total = app.skills_screen.installed.len();
    assert!(total > 0, "default must seed every skill as installed");
    app.handle_key(KeyCode::Enter); // toggle focused skill OFF (it starts on)
    assert_eq!(
        app.skills_screen.installed.len(),
        total - 1,
        "Enter should uninstall the focused (default-on) skill"
    );
    app.handle_key(KeyCode::Enter); // toggle back ON
    assert_eq!(
        app.skills_screen.installed.len(),
        total,
        "Enter should re-install"
    );
}

#[test]
fn skills_space_toggles_install() {
    let mut app = App::new();
    goto_skills(&mut app);
    // #263: focused skill starts installed → Space toggles it OFF.
    let total = app.skills_screen.installed.len();
    assert!(total > 0, "default must seed every skill as installed");
    app.handle_key(KeyCode::Char(' '));
    assert_eq!(
        app.skills_screen.installed.len(),
        total - 1,
        "Space should uninstall the focused (default-on) skill"
    );
}

// ── nav: ←/→ switch category (grid-local, NOT step-nav) ─────────────────────────

#[test]
fn skills_right_switches_category_not_step() {
    let mut app = App::new();
    goto_skills(&mut app);
    assert_eq!(app.skills_screen.active_category, 0, "starts on All");
    app.handle_key(KeyCode::Right);
    assert_eq!(app.current_step, SKILLS_STEP, "Right must NOT step-advance");
    assert_eq!(
        app.skills_screen.active_category, 1,
        "Right should move to the next category tab"
    );
}

#[test]
fn skills_left_switches_category_not_step_back() {
    let mut app = App::new();
    goto_skills(&mut app);
    app.handle_key(KeyCode::Right); // → category 1
    app.handle_key(KeyCode::Left); // ← back to category 0 (All)
    assert_eq!(app.current_step, SKILLS_STEP, "Left must NOT step-back");
    assert_eq!(
        app.skills_screen.active_category, 0,
        "Left should move to the previous category tab"
    );
}

#[test]
fn skills_down_selects_not_step() {
    let mut app = App::new();
    goto_skills(&mut app);
    app.handle_key(KeyCode::Down);
    assert_eq!(app.current_step, SKILLS_STEP, "Down must NOT step-advance");
}

// ── ESC = back one step (not quit) ──────────────────────────────────────────────

#[test]
fn skills_esc_backs_out_one_step() {
    let mut app = App::new();
    goto_skills(&mut app);
    let before = app.current_step;
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        before - 1,
        "ESC should back out one step, not quit"
    );
}
