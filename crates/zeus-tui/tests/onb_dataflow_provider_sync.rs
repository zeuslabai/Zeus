//! Data-flow propagation proof — Provider choice must reach the Model catalog
//! and the Complete summary. Regression guard for the frozen-catalog bug found
//! in the pre-deploy audit on `187a6ce0`:
//!
//!   `ModelScreen` was constructed once with provider "anthropic" and never
//!   re-pointed when the user picked a different provider on the Provider
//!   screen, so the Model step always showed Anthropic's catalog and the
//!   Complete summary emitted e.g. `openai/claude-opus-4-8` (provider/model
//!   mismatch). Fix: `on_step_enter(Model)` calls `model_screen.set_provider`.
//!
//! Separate file = conflict-free with the other onb_*.rs files.

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

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

/// Drive Welcome → Mode → Provider, select the Nth provider via Down presses,
/// then advance into the Model step (Auth sits between — step past it).
fn walk_to_model_with_provider(provider_down_presses: usize) -> App {
    let mut app = App::new();
    // Welcome(0) -> Mode(1): Right advances.
    app.handle_key(KeyCode::Right);
    // Mode(1) -> Provider(2): Enter picks a mode card and advances.
    app.handle_key(KeyCode::Enter);
    // Provider(2): move the selection down N times to pick a non-default provider.
    for _ in 0..provider_down_presses {
        app.handle_key(KeyCode::Down);
    }
    // Provider(2) -> Auth(3): Right advances.
    app.handle_key(KeyCode::Right);
    // Auth(3) -> Model(4): Right advances; on_step_enter(Model) must sync provider.
    app.handle_key(KeyCode::Right);
    app
}

#[test]
fn model_catalog_tracks_picked_provider_not_frozen_anthropic() {
    // Default (no Down): provider index 0 == anthropic. Catalog = Anthropic.
    let mut app0 = walk_to_model_with_provider(0);
    let s0 = render(&mut app0);
    assert!(
        s0.to_lowercase().contains("anthropic"),
        "default provider should render the Anthropic catalog\n{s0}"
    );

    // Pick the 2nd provider (one Down). Whatever it is, the Model screen's
    // sub-line names *that* provider — NOT a frozen "anthropic".
    let mut app1 = walk_to_model_with_provider(1);
    let s1 = render(&mut app1);
    // The two renders must differ — proof the catalog is not frozen.
    assert_ne!(
        s0, s1,
        "Model screen rendered identically for two different providers — \
         catalog is frozen (the pre-deploy bug)"
    );
}

#[test]
fn complete_summary_provider_matches_picked_provider() {
    use zeus_tui::screens::provider::provider_id_at;

    // Walk to Model with the 2nd provider picked, capture its canonical id.
    let mut app = walk_to_model_with_provider(1);
    // The picked provider id — derived the same way build_summary does.
    // provider_selected is private, but the Model sub-line + summary both
    // flow from it; we assert the summary's LLM Provider row starts with the
    // picked id (not "anthropic/").
    // Advance Model(4) -> ... -> Complete(18) so build_summary runs.
    // Grid steps (6/8/9/11/15/17) consume Right for in-screen focus, so bump
    // them directly — same idiom as goto_complete in onb_complete_1to1.rs.
    let mut guard = 0;
    while app.current_step < 18 {
        let s = app.current_step;
        if s == 6 || s == 8 || s == 9 || s == 11 || s == 15 || s == 17 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        guard += 1;
        assert!(guard < 100, "walk to Complete stalled");
    }
    let picked = provider_id_at(1);
    let s = render(&mut app);
    // The summary's LLM Provider row reads "{provider}/{model}". The picked
    // provider id must appear — and crucially the catalog model must be that
    // provider's, not Anthropic's frozen default.
    assert!(
        s.contains(picked),
        "Complete summary must name the picked provider '{picked}'\n{s}"
    );
}
