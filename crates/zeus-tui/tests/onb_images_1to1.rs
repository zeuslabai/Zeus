//! 1:1 render tests for the Images onboarding screen (15/19, step index 14).
//!
//! Verifies the screen matches the corrected talos backend set (per Zeus100's
//! dispatch, which overrides the JSX-mirrored spec list):
//!   OpenAI GPT Image · Automatic1111 · ComfyUI · Fooocus · OpenAI-compat · Skip
//! The JSX's Google NanoBanana / BFL Flux are NOT distinct talos backends and
//! must be GONE. Also checks the a1111 Z-Image Turbo Steps hint (verbatim),
//! char-safe ***{last4} secret masking, Base URL for local backends, and that
//! Right step-advances (Images is a vertical list, NOT a ←/→ grid).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

// ---- helpers ---------------------------------------------------------------

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

fn press(app: &mut App, keys: &[KeyCode]) {
    for k in keys {
        app.handle_key(*k);
    }
}

/// Walk Welcome(0) → `target`. Mirrors onb_106b's goto_step: Mode(1) consumes
/// Right (Enter leaves it); the grid steps (Channels 6, Gateway 8, Agent 9,
/// Security 11) consume ←/→ for in-screen focus so we bump+on_step_enter past
/// them; every other screen step-advances on Right. Images=14 sits past all the
/// grids, so the helper must clear them or the walk hangs (the cross-screen rule).
fn goto_images(app: &mut App) {
    let mut guard = 0;
    while app.current_step < 14 {
        let s = app.current_step;
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
        assert!(guard < 100, "goto_images stalled at step {s}");
    }
    assert_eq!(app.current_step, 14, "failed to reach Images (step 14)");
}

// ---- tests -----------------------------------------------------------------

#[test]
fn images_provider_set_is_real_talos_backends() {
    let mut app = App::new();
    goto_images(&mut app);
    // Default selection = OpenAI (index 0) — header shows it; the list shows all.
    let frame = render(&app);
    // The 5 real backends + Skip must all render in the LEFT provider list.
    assert!(frame.contains("OpenAI GPT Image"), "OpenAI GPT Image missing");
    assert!(frame.contains("Automatic1111"), "Automatic1111 missing");
    assert!(frame.contains("ComfyUI"), "ComfyUI missing");
    assert!(frame.contains("Fooocus"), "Fooocus missing");
    assert!(frame.contains("OpenAI compat URL"), "OpenAI compat URL missing");
    assert!(frame.contains("Skip"), "Skip missing");
}

#[test]
fn images_nanobanana_and_bfl_are_gone() {
    let mut app = App::new();
    goto_images(&mut app);
    let frame = render(&app);
    // The stale JSX-mirrored providers must NOT appear anywhere.
    assert!(!frame.contains("NanoBanana"), "NanoBanana must be swapped out");
    assert!(!frame.contains("BFL"), "BFL Flux must be swapped out");
    assert!(!frame.contains("flux"), "flux placeholder must be gone");
    assert!(!frame.contains("GCP"), "Google glyph must be gone");
    // The new glyphs ARE present.
    assert!(frame.contains("CMF"), "ComfyUI glyph CMF missing");
    assert!(frame.contains("FOO"), "Fooocus glyph FOO missing");
}

#[test]
fn images_a1111_steps_hint_verbatim() {
    let mut app = App::new();
    goto_images(&mut app);
    // Select Automatic1111: from OpenAI(0) → ComfyUI → Fooocus → compat → a1111.
    // Down 4 times lands on a1111 (index 4).
    press(&mut app, &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down]);
    let frame = render(&app);
    assert!(
        frame.contains("Automatic1111"),
        "a1111 should be selected/visible"
    );
    // Steps field + the EXACT Z-Image Turbo warning (the real talos gotcha).
    // The hint WRAPS across two rows in the narrow right panel (a single
    // set_line truncated "black PNG)" before the fix). Because this is a 2-col
    // layout, the left provider list interleaves between the wrapped rows — so
    // assert the head fragment (with ⚠ + "must be 1") AND the tail fragment
    // ("black PNG)") that previously got clipped, each on its own rendered row.
    assert!(frame.contains("Steps"), "Steps field missing for a1111");
    assert!(
        frame.contains("⚠ Z-Image Turbo: must be 1 (multi-step →"),
        "a1111 Z-Image Turbo hint head must render"
    );
    assert!(
        frame.contains("black PNG)"),
        "a1111 Z-Image Turbo hint tail (black PNG) must not be clipped"
    );
    // a1111 = local backend → Base URL present, default dgx-spark host.
    assert!(frame.contains("Base URL"), "a1111 needs a Base URL field");
    assert!(
        frame.contains("dgx-spark:7860"),
        "a1111 default base url placeholder missing"
    );
}

#[test]
fn images_local_backends_have_base_url() {
    let mut app = App::new();
    goto_images(&mut app);
    // ComfyUI = index 1 (one Down from OpenAI).
    press(&mut app, &[KeyCode::Down]);
    let frame = render(&app);
    assert!(frame.contains("ComfyUI"), "ComfyUI should be selected");
    assert!(
        frame.contains("Base URL"),
        "ComfyUI is a local backend → needs Base URL"
    );
    assert!(
        frame.contains("localhost:8188"),
        "ComfyUI default base url placeholder missing"
    );
}

#[test]
fn images_secret_mask_is_last4_char_safe() {
    let mut app = App::new();
    goto_images(&mut app);
    // OpenAI selected: fields = [API Key (secret), Model]. Focus is field 0.
    // Type a multibyte key — masking must NOT byte-slice (panic) and must show
    // ***{last4}.
    press(&mut app, &"sk-café".chars().map(KeyCode::Char).collect::<Vec<_>>());
    let frame = render(&app);
    // ***{last4} of "sk-café" = "***café".
    assert!(frame.contains("***café"), "secret must render as ***{{last4}}");
    // The full secret must NOT leak.
    assert!(
        !frame.contains("sk-café"),
        "raw secret must never appear on screen"
    );
}

#[test]
fn images_short_secret_no_underflow() {
    let mut app = App::new();
    goto_images(&mut app);
    // A 2-char key (fewer than 4) must not panic / underflow → "***xy".
    press(&mut app, &[KeyCode::Char('x'), KeyCode::Char('y')]);
    let frame = render(&app);
    assert!(frame.contains("***xy"), "short secret should mask to ***xy");
}

#[test]
fn images_skip_shows_no_config() {
    let mut app = App::new();
    goto_images(&mut app);
    // Up from OpenAI(0) wraps to Skip (last index) — no config panel fields.
    press(&mut app, &[KeyCode::Up]);
    let frame = render(&app);
    assert!(frame.contains("Skip"), "Skip option must render");
    assert!(
        frame.contains("No image gen configured."),
        "Skip → empty config message"
    );
    // No credential fields when skipping.
    assert!(
        !frame.contains("API Key *"),
        "Skip must not show required API Key field"
    );
}

#[test]
fn images_down_selects_not_step_and_right_advances() {
    let mut app = App::new();
    goto_images(&mut app);
    assert_eq!(app.current_step, 14);
    // Down moves selection within the vertical list — does NOT change the step.
    press(&mut app, &[KeyCode::Down]);
    assert_eq!(app.current_step, 14, "Down must select, not step-nav");
    // Right step-advances (Images is NOT a ←/→ grid).
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.current_step, 15, "Right must advance Images → step 15");
}

#[test]
fn images_esc_backs_not_quit() {
    let mut app = App::new();
    goto_images(&mut app);
    assert_eq!(app.current_step, 14);
    app.handle_key(KeyCode::Esc);
    // ESC backs out one step (does not quit).
    assert_eq!(app.current_step, 13, "Esc must back to step 13, not quit");
}



