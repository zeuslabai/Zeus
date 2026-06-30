//! Cut 2 — X/Twitter + IRC sync-timing dropout trace (zeus106).
//!
//! merakizzz reported X/Twitter and IRC "don't show in channel config" after
//! being selected. This drives the REAL key sequence — focus + toggle irc
//! (idx 4) and x_twitter (idx 5) on the Channels screen, advance into
//! ChannelConfig, and asserts both survive the cross-screen sync into
//! `chanconfig.toggled` AND render as config cards.
//!
//! Separate file = conflict-free with other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

fn render_lines(app: &App) -> Vec<String> {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw must not panic");
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

/// Drive the real key path: land on Channels (step 6), focus idx 4 (irc) via
/// Right×4 from idx 0, toggle, then Right to idx 5 (x_twitter), toggle.
#[test]
fn x_and_irc_survive_sync_into_chanconfig() {
    let mut app = App::new();
    app.current_step = 6;
    assert_eq!(app.current_step, 6, "land on Channels");

    // Channels default-selected = [discord(1), telegram(0)]. Navigate the grid
    // to irc (flat idx 4) and x_twitter (flat idx 5) and toggle them ON.
    // focused starts at 0. Right×4 → idx 4 (irc).
    for _ in 0..4 {
        app.handle_key(KeyCode::Right);
    }
    assert_eq!(app.channels_screen.focused, 4, "focus on irc (idx 4)");
    app.handle_key(KeyCode::Char(' ')); // toggle irc ON
    app.handle_key(KeyCode::Right); // → idx 5 (x_twitter)
    assert_eq!(app.channels_screen.focused, 5, "focus on x_twitter (idx 5)");
    app.handle_key(KeyCode::Char(' ')); // toggle x_twitter ON

    // Both must be in the channels-screen selection.
    let ids = app.channels_screen.selected_ids();
    assert!(ids.contains(&"irc".to_string()), "irc selected: {ids:?}");
    assert!(
        ids.contains(&"x_twitter".to_string()),
        "x_twitter selected: {ids:?}"
    );

    // Advance into ChannelConfig (step 7) — the real on_step_enter sync fires.
    app.advance_step();
    assert_eq!(app.current_step, 7, "land on ChannelConfig");

    // The dropout claim: irc/x_twitter vanish here. Assert they DON'T.
    let toggled = &app.chanconfig_screen.toggled;
    assert!(
        toggled.contains(&"irc".to_string()),
        "irc must survive sync into chanconfig.toggled: {toggled:?}"
    );
    assert!(
        toggled.contains(&"x_twitter".to_string()),
        "x_twitter must survive sync into chanconfig.toggled: {toggled:?}"
    );

    // And they must actually RENDER as config cards (not just live in state).
    let lines = render_lines(&app);
    let blob = lines.join("\n");
    assert!(
        blob.contains("IRC") || blob.to_lowercase().contains("irc"),
        "IRC config card must render on ChannelConfig"
    );
    assert!(
        blob.contains("X / Twitter") || blob.contains("Twitter"),
        "X/Twitter config card must render on ChannelConfig"
    );
}

/// Back-nav regression: enter ChannelConfig, step BACK to Channels, toggle
/// x_twitter, advance again — the re-sync must re-capture it (stale-state guard).
#[test]
fn back_nav_resync_recaptures_x_twitter() {
    let mut app = App::new();
    app.current_step = 6;

    // Advance to ChannelConfig with defaults, then back to Channels.
    app.advance_step();
    assert_eq!(app.current_step, 7);
    app.step_back();
    assert_eq!(app.current_step, 6, "back on Channels");

    // Now toggle x_twitter ON (idx 5) and re-advance.
    for _ in 0..5 {
        app.handle_key(KeyCode::Right);
    }
    assert_eq!(app.channels_screen.focused, 5);
    app.handle_key(KeyCode::Char(' '));
    app.advance_step();
    assert_eq!(app.current_step, 7);

    assert!(
        app.chanconfig_screen
            .toggled
            .contains(&"x_twitter".to_string()),
        "x_twitter toggled after back-nav must re-sync: {:?}",
        app.chanconfig_screen.toggled
    );
}
