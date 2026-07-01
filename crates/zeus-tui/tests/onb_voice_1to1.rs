//! Voice (14/19) 1:1 render tests — zeus106 `fix/tui-1to1-voice`.
//!
//! Asserts the Voice onboarding screen matches the JSX prototype after the
//! 1:1 cut:
//!   • provider list = our REAL zeus-tts set: ElevenLabs · OpenAI TTS ·
//!     Edge TTS · Custom Endpoint · Skip  (Cartesia swapped → Edge).
//!   • RIGHT credentials panel: API Key (secret) + Voice ID; custom → Base URL.
//!   • secret masking is char-safe `***{last4}` (never byte-slice → no panic
//!     on multibyte input).
//!   • Skip → yellow "⚠ NO VOICE CONFIGURED" warning box.
//!   • nav: ↑/↓ select provider, Right step-advances (Voice is NOT a ←/→ grid).
//!   • ESC = back one step (not quit).
//!
//! Self-contained helpers → conflict-free with the other onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

// ---- helpers ---------------------------------------------------------------

/// Render the current app state into a 120×44 TestBackend → newline-joined
/// String for substring asserts.
fn render(app: &App) -> String {
    let backend = TestBackend::new(120, 44);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| frame(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for k in keys {
        app.handle_key(*k);
    }
}

/// Walk to the Voice step (13). Steps 6/8/9/11 are ←/→ grids → bump directly;
/// everything else (incl. Voice's own ↑/↓ list at 13) step-advances on Right.
fn goto_voice(app: &mut App) {
    let mut guard = 0;
    while app.current_step < 13 {
        if app.current_step == 3 { app.current_step += 1; app.on_step_enter(); continue; }        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 6 || s == 8 || s == 9 || s == 11 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
        guard += 1;
        assert!(guard < 100, "goto_voice stalled");
    }
    assert_eq!(app.current_step, 13, "failed to reach Voice (13)");
}

// ---- provider list ---------------------------------------------------------

#[test]
fn voice_provider_list_is_real_zeus_tts_set() {
    let mut app = App::new();
    goto_voice(&mut app);
    let f = render(&app);
    // Real set present.
    assert!(f.contains("ElevenLabs"), "ElevenLabs (real) must be present");
    assert!(f.contains("OpenAI TTS"), "OpenAI TTS must be present");
    assert!(f.contains("Edge TTS"), "Edge TTS must replace Cartesia");
    assert!(f.contains("Custom Endpoint"), "Custom Endpoint must be present");
    assert!(f.contains("Skip"), "Skip must be present");
    // Glyphs.
    assert!(f.contains("11L"), "ElevenLabs glyph 11L");
    assert!(f.contains("OAI"), "OpenAI glyph OAI");
    assert!(f.contains("EDG"), "Edge glyph EDG");
    // Cartesia is gone (the swap).
    assert!(!f.contains("Cartesia"), "Cartesia must be removed");
    assert!(!f.contains("CTS"), "Cartesia glyph CTS must be gone");
}

// ---- credentials panel ------------------------------------------------------

#[test]
fn voice_credentials_panel_renders_for_api_provider() {
    let mut app = App::new();
    goto_voice(&mut app);
    // Default selection = ElevenLabs (index 0, ≠ Skip) → credentials show.
    let f = render(&app);
    assert!(f.contains("CREDENTIALS"), "CREDENTIALS label for non-Skip provider");
    assert!(f.contains("API Key"), "API Key field");
    assert!(f.contains("Voice ID"), "Voice ID field");
    assert!(f.contains("TEST VOICE"), "▸ TEST VOICE button");
    // Standard (non-custom) provider must NOT show Base URL.
    assert!(!f.contains("Base URL"), "Base URL only for custom endpoint");
}

#[test]
fn voice_custom_endpoint_shows_base_url() {
    let mut app = App::new();
    goto_voice(&mut app);
    // ElevenLabs(0) → OpenAI(1) → Edge(2) → Custom(3): three Downs.
    press(&mut app, &[KeyCode::Down, KeyCode::Down, KeyCode::Down]);
    let f = render(&app);
    assert!(f.contains("Custom Endpoint"), "custom selected");
    assert!(f.contains("Base URL"), "custom → Base URL field appears");
    assert!(f.contains("API Key"), "custom still has API Key");
    assert!(f.contains("Voice ID"), "custom still has Voice ID");
}

// ---- secret masking (char-safe ***{last4}) ---------------------------------

#[test]
fn voice_api_key_masks_char_safe_last4() {
    let mut app = App::new();
    goto_voice(&mut app);
    // Focus is on API Key (field 0, secret). Type a known secret.
    press(
        &mut app,
        &"sk-SECRET1234".chars().map(KeyCode::Char).collect::<Vec<_>>(),
    );
    let f = render(&app);
    // ***{last4} shape → "***1234"; raw secret must NOT leak.
    assert!(f.contains("***1234"), "API Key masks to ***last4");
    assert!(!f.contains("sk-SECRET"), "raw secret must never render");
}

#[test]
fn voice_multibyte_secret_no_panic() {
    // The byte-slice trap: a multibyte secret must mask without splitting a
    // UTF-8 code point. chars().rev().take(4) is panic-proof.
    let mut app = App::new();
    goto_voice(&mut app);
    press(
        &mut app,
        &"café→世界🔱".chars().map(KeyCode::Char).collect::<Vec<_>>(),
    );
    let f = render(&app);
    assert!(!f.trim().is_empty(), "multibyte secret must not panic render");
    // last-4 chars = "界🔱" tail (we don't assert exact glyphs, just no leak +
    // the *** prefix present).
    assert!(f.contains("***"), "masked prefix present for multibyte secret");
}

#[test]
fn voice_short_secret_no_panic() {
    // Secret shorter than 4 chars must not underflow/byte-panic.
    let mut app = App::new();
    goto_voice(&mut app);
    press(&mut app, &[KeyCode::Char('a')]);
    let f = render(&app);
    assert!(f.contains("***a"), "1-char secret → ***a (no underflow)");
}

// ---- Skip warning box -------------------------------------------------------

#[test]
fn voice_skip_shows_no_voice_warning() {
    let mut app = App::new();
    goto_voice(&mut app);
    // Walk to Skip (last entry). 5 providers → 4 Downs from index 0.
    press(
        &mut app,
        &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down],
    );
    let f = render(&app);
    assert!(f.contains("NO VOICE CONFIGURED"), "Skip → ⚠ NO VOICE CONFIGURED box");
    // The credentials panel must NOT show for Skip.
    assert!(!f.contains("TEST VOICE"), "no TEST VOICE button on Skip");
}

// ---- nav + ESC --------------------------------------------------------------

#[test]
fn voice_down_selects_not_step() {
    let mut app = App::new();
    goto_voice(&mut app);
    let before = app.current_step;
    press(&mut app, &[KeyCode::Down]);
    assert_eq!(app.current_step, before, "↓ selects provider, does NOT step-nav");
}

#[test]
fn voice_right_step_advances_not_grid() {
    // Voice is a vertical ↑/↓ list — Right is NOT consumed as grid-local, so it
    // must advance the step (the substrate check before touching goto_step).
    let mut app = App::new();
    goto_voice(&mut app);
    let before = app.current_step;
    app.handle_key(KeyCode::Right);
    assert_eq!(app.current_step, before + 1, "Right advances step (no ←/→ grid)");
}

#[test]
fn voice_esc_goes_back_not_quit() {
    let mut app = App::new();
    goto_voice(&mut app);
    let before = app.current_step;
    app.handle_key(KeyCode::Esc);
    assert_eq!(app.current_step, before - 1, "ESC = back one step");
    // Still rendering = did not quit.
    let f = render(&app);
    assert!(!f.trim().is_empty(), "ESC must not quit the app");
}
