//! 1:1 render tests for the ChanConfig onboarding screen (08/19 · JSX 1007–1126).
//!
//! Asserts the prototype-faithful surfaces:
//!   • header "Configure {N} channel(s)" (singular/plural),
//!   • one config card per SELECTED channel with sdk italic + state badges
//!     (QR PAIRING amber · APPLESCRIPT cyan · ✓ TESTED green),
//!   • credential-less info lines for imessage (AppleScript) + signal (QR),
//!   • char-safe `***{last4}` secret masking (multibyte-safe, never byte-sliced),
//!   • ▸ SEND TEST button + ✓ DELIVERED / success line on test.
//!
//! Separate file = conflict-free with the other onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

fn render(app: &App) -> String {
    let backend = TestBackend::new(120, 44);
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

/// Put the app on the ChannelConfig step (7) with an explicit toggled set,
/// bypassing the grid-nav of the Channels screen for deterministic content.
fn chanconfig_with(app: &mut App, toggled: &[&str]) {
    app.current_step = 8;
    app.chanconfig_screen.toggled = toggled.iter().map(|s| s.to_string()).collect();
    app.chanconfig_screen.field_cursor = 0;
    // Refocus onto the first focusable field of the new set.
    app.chanconfig_screen.focus_prev();
    app.chanconfig_screen.focus_next();
}

// ── Header: pluralization ────────────────────────────────────────────────────

#[test]
fn header_pluralizes_channel_count() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["telegram", "discord"]);
    let s = render(&app);
    assert!(
        s.contains("Configure 2 channels"),
        "expected plural header:\n{s}"
    );

    let mut app1 = App::new();
    chanconfig_with(&mut app1, &["telegram"]);
    let s1 = render(&app1);
    assert!(
        s1.contains("Configure 1 channel"),
        "expected singular header:\n{s1}"
    );
    assert!(
        !s1.contains("Configure 1 channels"),
        "singular must not have trailing 's'"
    );
}

// ── State badges: QR PAIRING / APPLESCRIPT ───────────────────────────────────

#[test]
fn phone_paired_channels_show_state_badges_and_info_lines() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["imessage", "signal"]);
    let s = render(&app);

    // iMessage = AppleScript: cyan badge + ● macOS-bridge info line, no fields.
    assert!(
        s.contains("APPLESCRIPT"),
        "iMessage must show APPLESCRIPT badge:\n{s}"
    );
    assert!(
        s.contains("Uses native macOS bridge"),
        "iMessage must show the AppleScript info line:\n{s}"
    );

    // Signal = QR: amber badge + ⚠ QR info line, no fields.
    assert!(
        s.contains("QR PAIRING"),
        "Signal must show QR PAIRING badge:\n{s}"
    );
    assert!(
        s.contains("Requires phone-side QR scan"),
        "Signal must show the QR info line:\n{s}"
    );

    // Credential-less channels must NOT render a SEND TEST button.
    assert!(
        !s.contains("SEND TEST"),
        "imessage/signal have no test button:\n{s}"
    );
}

// ── sdk italic label renders ─────────────────────────────────────────────────

#[test]
fn card_header_shows_name_and_sdk() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["telegram"]);
    let s = render(&app);
    assert!(s.contains("Telegram"), "card must show channel name:\n{s}");
    assert!(
        s.contains("grammers MTProto"),
        "card must show sdk label:\n{s}"
    );
    // Telegram has fields → SEND TEST button present.
    assert!(
        s.contains("SEND TEST"),
        "credentialed channel must show SEND TEST:\n{s}"
    );
}

// ── Secret masking: ***{last4}, char-safe ────────────────────────────────────

#[test]
fn secret_field_masks_as_stars_last4() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["discord"]);
    // Focus the discord bot token (secret) and type a value.
    app.chanconfig_screen.focused_field = "discord.token".to_string();
    for c in "MTAxSECRET9876".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    let s = render(&app);
    assert!(s.contains("***9876"), "secret must render ***last4:\n{s}");
    assert!(!s.contains("MTAxSECRET"), "raw secret must not leak:\n{s}");
}

#[test]
fn secret_masking_is_multibyte_safe() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["discord"]);
    app.chanconfig_screen.focused_field = "discord.token".to_string();
    // Multibyte chars: a byte-slice of last-4 would panic / split a codepoint.
    for c in "key→café".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    // Must not panic; last 4 chars are "café" (4 chars, 5 bytes).
    let s = render(&app);
    assert!(
        s.contains("***café"),
        "multibyte last4 must render intact:\n{s}"
    );
}

#[test]
fn short_secret_does_not_panic() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["discord"]);
    app.chanconfig_screen.focused_field = "discord.token".to_string();
    for c in "ab".chars() {
        app.handle_key(KeyCode::Char(c));
    }
    // 2-char secret: ***ab — must not panic on short input.
    let s = render(&app);
    assert!(s.contains("***ab"), "short secret masks safely:\n{s}");
}

// ── SEND TEST → DELIVERED + ✓ TESTED ─────────────────────────────────────────

#[test]
fn send_test_transitions_to_delivered_and_tested_badge() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["telegram"]);
    // Enter the required credentials first — trigger_test now does REAL
    // validation (every `required` field must be non-empty), so a test with
    // empty creds correctly Fails. Telegram requires api_id + api_hash.
    app.chanconfig_screen
        .config_values
        .insert("telegram.api_id".to_string(), "12345678".to_string());
    app.chanconfig_screen.config_values.insert(
        "telegram.api_hash".to_string(),
        "abcdef0123456789".to_string(),
    );
    app.chanconfig_screen
        .config_values
        .insert("telegram.phone".to_string(), "+15551234567".to_string());
    // Focus the telegram test button and trigger it (Space = trigger_test, #345).
    app.chanconfig_screen.focused_field = "test:telegram".to_string();
    app.handle_key(KeyCode::Char(' '));
    let s = render(&app);
    assert!(
        s.contains("DELIVERED"),
        "after test, button reads DELIVERED:\n{s}"
    );
    assert!(
        s.contains("✓ TESTED"),
        "after test, header shows ✓ TESTED badge:\n{s}"
    );
    assert!(
        s.contains("Test message delivered to Telegram"),
        "success line must name the channel:\n{s}"
    );
}


#[test]
fn channelconfig_enter_advances_space_activates_focused_control() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["telegram"]);
    app.chanconfig_screen
        .config_values
        .insert("telegram.api_id".to_string(), "12345678".to_string());
    app.chanconfig_screen.config_values.insert(
        "telegram.api_hash".to_string(),
        "abcdef0123456789".to_string(),
    );
    app.chanconfig_screen
        .config_values
        .insert("telegram.phone".to_string(), "+15551234567".to_string());
    app.chanconfig_screen.focused_field = "test:telegram".to_string();

    let before = app.current_step;
    app.handle_key(KeyCode::Enter);
    assert_eq!(
        app.current_step,
        before + 1,
        "Enter should navigate forward from ChannelConfig, not fire SEND TEST"
    );
    assert!(
        !app.chanconfig_screen.test_statuses.contains_key("telegram"),
        "Enter must not trigger the focused ChannelConfig control"
    );

    app.current_step = before;
    app.handle_key(KeyCode::Char(' '));
    assert_eq!(
        app.current_step, before,
        "Space should activate the focused ChannelConfig control without navigating"
    );
    assert!(
        render(&app).contains("DELIVERED"),
        "Space should still trigger SEND TEST on the focused test button"
    );
}

#[test]
fn channelconfig_space_cycles_allow_bots_enter_advances() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["discord"]);
    app.chanconfig_screen.focused_field = "allowbots:discord".to_string();

    let before = app.current_step;
    assert_eq!(app.chanconfig_screen.bot_policy("discord"), "mentions");
    app.handle_key(KeyCode::Char(' '));
    assert_eq!(
        app.chanconfig_screen.bot_policy("discord"),
        "on",
        "Space should cycle the allow_bots selector"
    );
    assert_eq!(app.current_step, before, "Space cycle should stay on the page");

    app.handle_key(KeyCode::Enter);
    assert_eq!(
        app.current_step,
        before + 1,
        "Enter should continue navigation even on an allow_bots selector"
    );
    assert_eq!(
        app.chanconfig_screen.bot_policy("discord"),
        "on",
        "Enter must not also cycle allow_bots"
    );
}

// ── Empty state ──────────────────────────────────────────────────────────────

#[test]
fn empty_toggled_shows_console_only_box() {
    let mut app = App::new();
    chanconfig_with(&mut app, &[]);
    let s = render(&app);
    assert!(
        s.contains("Configure 0 channels"),
        "header counts zero:\n{s}"
    );
    assert!(
        s.contains("No channels selected"),
        "empty state must show console-only message:\n{s}"
    );
}

// ── ESC = back one step (not quit) ───────────────────────────────────────────

#[test]
fn esc_steps_back_not_quit() {
    let mut app = App::new();
    chanconfig_with(&mut app, &["telegram"]);
    let before = app.current_step;
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        before - 1,
        "ESC backs out one step, not quit"
    );
}
