//! 1:1 fidelity tests for the Channels onboarding screen (step 6, JSX 951–1006).
//!
//! Asserts the cut from `fix/tui-1to1-channels`:
//!   - two GROUP sections render ("CLOUD APIS" / "PHONE-PAIRED") with their
//!     right-note hints ("API key auth" / "QR pairing required");
//!   - 2-col GRID — the two channels in a group's first row share one screen
//!     row (e.g. Telegram + Discord side-by-side);
//!   - SELECTED (N) panel count tracks toggles (Space and Enter both toggle);
//!   - empty-state dashed box appears with "No channels selected." /
//!     "Zeus will run console-only." when nothing is selected;
//!   - ←/→ move grid focus (not step-nav); ESC = back one step (does NOT quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

fn render_lines_at(app: &App, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut lines = Vec::with_capacity(buf.area.height as usize);
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
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut lines = Vec::with_capacity(buf.area.height as usize);
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

/// Navigate to the Channels step (index 6).
///
/// Step-nav across the intermediate screens is per-screen (Mode/Channels
/// consume ←/→ for card selection, several screens consume Enter for in-screen
/// actions), so we set the public `current_step` directly — this exercises the
/// real Channels render + key handlers without fighting each intermediate
/// screen's idiom.
fn goto_channels(app: &mut App) {
    app.current_step = 7;
    assert_eq!(app.current_step, 7, "should land on Channels");
}

/// Index of the screen row that contains `needle`, if any.
fn row_of(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|l| l.contains(needle))
}

#[test]
fn channels_two_group_sections_with_hints() {
    let mut app = App::new();
    goto_channels(&mut app);
    let lines = render_lines(&app);
    let text = lines.join("\n");

    // Both group headers present (uppercased).
    assert!(
        text.contains("CLOUD APIS"),
        "Cloud APIs group header must render (UPPER). Got:\n{text}"
    );
    assert!(
        text.contains("PHONE-PAIRED"),
        "Phone-paired group header must render (UPPER). Got:\n{text}"
    );

    // Group right-note hints.
    assert!(
        text.contains("API key auth"),
        "Cloud APIs group must show 'API key auth' note"
    );
    assert!(
        text.contains("QR pairing required"),
        "Phone-paired group must show 'QR pairing required' note"
    );

    // Ordering: Cloud APIs section sits above Phone-paired.
    let cloud = row_of(&lines, "CLOUD APIS").expect("cloud header row");
    let phone = row_of(&lines, "PHONE-PAIRED").expect("phone header row");
    assert!(
        cloud < phone,
        "Cloud APIs group must render above Phone-paired (cloud={cloud}, phone={phone})"
    );
}

#[test]
fn channels_two_col_grid_first_row_shares_a_line() {
    let mut app = App::new();
    goto_channels(&mut app);
    // Clear the default pre-selection (telegram+discord) so channel names only
    // appear in the GRID, not also in the SELECTED panel (which would make
    // first-occurrence row matching ambiguous).
    app.handle_key(KeyCode::Char(' ')); // toggle idx 0 (telegram) off
    app.handle_key(KeyCode::Right); // focus idx 1 (discord)
    app.handle_key(KeyCode::Char(' ')); // toggle idx 1 (discord) off
    let lines = render_lines(&app);

    // The first Cloud APIs row is Telegram (col 0) + Discord (col 1) → they
    // must appear on the SAME screen row (2-col grid, not a vertical list).
    let tg = row_of(&lines, "Telegram").expect("Telegram card must render");
    let dc = row_of(&lines, "Discord").expect("Discord card must render");
    assert_eq!(
        tg, dc,
        "Telegram and Discord are grid-row 0 → must share one screen row \
         (2-col grid). tg={tg}, dc={dc}"
    );

    // Slack + Email are grid-row 1 → also share a row, below row 0.
    let sl = row_of(&lines, "Slack").expect("Slack card");
    let em = row_of(&lines, "Email").expect("Email card");
    assert_eq!(
        sl, em,
        "Slack and Email are grid-row 1 → must share one screen row"
    );
    assert!(
        sl > tg,
        "Slack/Email row must be below Telegram/Discord row"
    );
}

#[test]
fn channels_selected_panel_count_tracks_toggles() {
    let mut app = App::new();
    goto_channels(&mut app);

    // Default pre-selection per JSX = discord + telegram → SELECTED (2).
    let text0 = render(&app);
    assert!(
        text0.contains("SELECTED (2)"),
        "default pre-selection is 2 (telegram+discord). Got:\n{text0}"
    );

    // Space toggles the focused (idx 0 = Telegram) OFF → SELECTED (1).
    app.handle_key(KeyCode::Char(' '));
    let text1 = render(&app);
    assert!(
        text1.contains("SELECTED (1)"),
        "after Space-toggle the count must drop to 1. Got:\n{text1}"
    );

    // Space toggles it back ON → SELECTED (2). Enter now ADVANCES the flow
    // (merakizzz nav-UX fix): on grid screens Space owns toggle, Enter = next.
    app.handle_key(KeyCode::Char(' '));
    let text2 = render(&app);
    assert!(
        text2.contains("SELECTED (2)"),
        "Space must toggle back to 2. Got:\n{text2}"
    );
}

#[test]
fn channels_empty_state_dashed_box() {
    let mut app = App::new();
    goto_channels(&mut app);

    // Clear the default selection: focus idx 0 + 1 and toggle both off.
    app.handle_key(KeyCode::Char(' ')); // toggle idx 0 (telegram) off
    app.handle_key(KeyCode::Right); // focus idx 1 (discord)
    app.handle_key(KeyCode::Char(' ')); // toggle idx 1 (discord) off

    let text = render(&app);
    assert!(
        text.contains("SELECTED (0)"),
        "all toggled off → SELECTED (0). Got:\n{text}"
    );
    assert!(
        text.contains("No channels selected."),
        "empty-state copy line 1 must render"
    );
    assert!(
        text.contains("Zeus will run console-only."),
        "empty-state copy line 2 must render"
    );
    // Dashed border glyphs present (the box rule chars).
    assert!(
        text.contains('╌') || text.contains('╎'),
        "empty-state must render a dashed border box. Got:\n{text}"
    );
}

#[test]
fn channels_right_left_move_grid_focus_not_step() {
    let mut app = App::new();
    goto_channels(&mut app);

    // → must NOT advance the step (it moves grid focus instead).
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.current_step, 7,
        "→ on Channels must move grid focus, NOT advance the step"
    );

    // ← must NOT go back a step either.
    app.handle_key(KeyCode::Left);
    assert_eq!(
        app.current_step, 7,
        "← on Channels must move grid focus, NOT step back"
    );
}

#[test]
fn channels_esc_backs_out_not_quit() {
    let mut app = App::new();
    goto_channels(&mut app);

    // ESC = back one step (to Fallback, step 5) — must NOT quit the app.
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step, 6,
        "ESC on Channels must back out to Fallback, not quit"
    );
}

/// IRC + X/Twitter (added fix/tui-channels-irc-x): both must render in the
/// Channels list under the CLOUD APIS group — merakizzz flagged them as
/// supported channels missing from the onboarding list. Adapters verified to
/// exist: zeus-channels `IrcAdapter` (channel_type "irc") + `x` (channel_type
/// "x_twitter").
#[test]
fn channels_includes_irc_and_x_under_cloud_apis() {
    let mut app = App::new();
    goto_channels(&mut app);
    let lines = render_lines(&app);
    let full = lines.join("\n");

    // Both present in the rendered list.
    assert!(full.contains("IRC"), "IRC must render in the Channels list");
    assert!(
        full.contains("X / Twitter"),
        "X / Twitter must render in the Channels list"
    );

    // Correct group: both sit below the CLOUD APIS header and above the
    // PHONE-PAIRED header (the render iterates GROUPS and filters by group).
    let cloud_row = row_of(&lines, "CLOUD APIS").expect("CLOUD APIS header renders");
    let phone_row = row_of(&lines, "PHONE-PAIRED").expect("PHONE-PAIRED header renders");
    let irc_row = row_of(&lines, "IRC").expect("IRC row renders");
    let x_row = row_of(&lines, "X / Twitter").expect("X / Twitter row renders");

    assert!(
        irc_row > cloud_row && irc_row < phone_row,
        "IRC must render within the CLOUD APIS group (row {irc_row}, cloud {cloud_row}, phone {phone_row})"
    );
    assert!(
        x_row > cloud_row && x_row < phone_row,
        "X / Twitter must render within the CLOUD APIS group (row {x_row}, cloud {cloud_row}, phone {phone_row})"
    );
}

#[test]
fn channels_100x30_render_dump() {
    let mut app = App::new();
    goto_channels(&mut app);
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, &app))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    eprintln!("\n--- CHANNELS 100x30 ---\n{out}\n--- END CHANNELS 100x30 ---");
}

#[test]
fn channels_100x30_uses_single_header_and_clean_footer() {
    let mut app = App::new();
    goto_channels(&mut app);
    let lines = render_lines_at(&app, 100, 30);
    let s = lines.join(
        "
",
    );

    assert_eq!(
        s.matches("Pick messaging channels").count(),
        1,
        "Channels should rely on the app StepHeader only at 100×30:
{s}"
    );
    assert!(
        s.contains("PHONE-PAIRED"),
        "compact render should still expose the second channel group:
{s}"
    );
    assert!(
        s.contains("QR pairing required"),
        "compact render should keep the phone-paired group hint intact:
{s}"
    );
    assert!(
        s.contains("↑↓←→ navigate  •  space toggle  •  ↵ continue  •  esc back"),
        "compact render should keep the Channels-specific footer hint:
{s}"
    );
    assert!(
        !s.contains("backquired"),
        "footer must not collide with the QR hint at 100×30:
{s}"
    );
}
