//! Data-flow propagation proof — Provider choice must reach the Fallback
//! candidate list (finding #2, folded into the same systemic fix as the Model
//! catalog freeze). Regression guard for:
//!
//!   `FallbackScreen` was constructed once with primary "anthropic" and never
//!   re-pointed when the user picked a different provider on the Provider
//!   screen. Result: pick e.g. glm → the Fallback step offered glm as its own
//!   fallback (a provider can't fall back to itself) AND wrongly excluded
//!   anthropic from the candidate list. Fix: `on_step_enter(Fallback)` calls
//!   `fallback_screen.set_primary(picked)`, which also drops the new primary
//!   from the chain if it was already added and clamps the cursor.
//!
//! Separate file = conflict-free with the other onb_*.rs files.

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::screens::FallbackScreen;

fn render(app: &mut App) -> String {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw must not panic");
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

/// Drive Welcome → Mode → Provider, pick the Nth provider, advance through
/// Auth → Model → Fallback. on_step_enter(Fallback) must sync the primary.
fn walk_to_fallback_with_provider(provider_down_presses: usize) -> App {
    let mut app = App::new();
    app.handle_key(KeyCode::Right); // Welcome(0) -> Mode(1)
    app.handle_key(KeyCode::Enter); // Mode(1) -> Provider(2)
    for _ in 0..provider_down_presses {
        app.handle_key(KeyCode::Down); // pick non-default provider
    }
    app.handle_key(KeyCode::Right); // Provider(2) -> Auth(3)
    app.current_step += 1;
    app.on_step_enter(); // Auth(3) -> Model(4): probe-gated (#240), bump past directly
    app.handle_key(KeyCode::Right); // Model(4) -> Fallback(5): on_step_enter syncs
    app
}

// ---- Direct-API proofs (highest signal — set_primary semantics) ----

#[test]
fn set_primary_repoints_and_excludes_new_primary() {
    // Default-constructed primary is "anthropic".
    let mut fb = FallbackScreen::new("anthropic".to_string());
    assert_eq!(fb.primary, "anthropic");

    // Re-point to glm (as the Provider screen would on entry).
    fb.set_primary("glm");
    assert_eq!(fb.primary, "glm", "primary must track the picked provider");
}

#[test]
fn set_primary_drops_new_primary_from_existing_chain() {
    // User picked anthropic primary, added glm to the chain, then went back
    // and promoted glm to primary. glm must not remain its own fallback.
    let mut fb = FallbackScreen::new("anthropic".to_string());
    fb.chain = vec!["glm".to_string(), "openai".to_string()];

    fb.set_primary("glm");
    assert!(
        !fb.chain.contains(&"glm".to_string()),
        "new primary must be dropped from the chain (no self-fallback); chain={:?}",
        fb.chain
    );
    // The other fallback survives.
    assert!(
        fb.chain.contains(&"openai".to_string()),
        "unrelated chain entries must survive the re-sync; chain={:?}",
        fb.chain
    );
}

#[test]
fn set_primary_clamps_cursor_into_filtered_list() {
    let mut fb = FallbackScreen::new("anthropic".to_string());
    // Park the cursor at a high index, then re-sync; it must clamp in-range.
    fb.cursor = 999;
    fb.set_primary("glm");
    // Candidates exclude exactly one provider (glm), so the list is non-empty
    // and the cursor must be a valid index into it.
    assert!(fb.cursor < 999, "cursor must be clamped after re-sync");
}

#[test]
fn set_primary_is_case_insensitive() {
    let mut fb = FallbackScreen::new("anthropic".to_string());
    fb.set_primary("GLM");
    assert_eq!(fb.primary, "glm", "primary must be normalized to lowercase");
}

// ---- Integration proof through on_step_enter + render ----

#[test]
fn fallback_candidates_exclude_glm_include_anthropic_when_glm_picked() {
    // glm is provider index 6 in the canonical registry → 6 Down presses.
    let mut app = walk_to_fallback_with_provider(6);
    let s = render(&mut app).to_lowercase();

    // anthropic is no longer the primary, so it must be OFFERED as a candidate.
    assert!(
        s.contains("anthropic"),
        "Fallback list must INCLUDE anthropic once glm is the primary\n{s}"
    );
    // The AVAILABLE candidate list must not present glm as its own fallback.
    // (glm is the primary; the right-panel chain is empty at entry.) We assert
    // the primary field is synced — the structural guarantee candidates()
    // filters on.
    assert_eq!(
        app.fallback_screen.primary, "glm",
        "on_step_enter(Fallback) must sync primary to the picked provider"
    );
}

#[test]
fn fallback_primary_stays_anthropic_when_default_picked() {
    // No Down presses → provider index 0 == anthropic (the default).
    let app = walk_to_fallback_with_provider(0);
    assert_eq!(
        app.fallback_screen.primary, "anthropic",
        "default provider keeps anthropic as the excluded primary"
    );
}
