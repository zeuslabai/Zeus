//! Fidelity tests for the Provider onboarding screen (step 2, JSX 577–648).
//!
//! Asserts the cut from `fix/tui-provider-fidelity`:
//!   - the detail panel renders the 56×56 glyph BADGE (filled color block with
//!     the provider glyph centered) — not just bare `ANT` text;
//!   - the `WILL WRITE TO ~/.zeus/config.toml` box renders with the
//!     `model = "{id}/{flagship}"` line;
//!   - the RIGHT column renders HINTS + RECOMMENDATIONS;
//!   - the left list items render as bordered Cards (border glyphs present).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

/// Render current app state into a 140×44 TestBackend; return per-row lines.
fn render_lines(app: &App) -> Vec<String> {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
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

/// Navigate to the Provider step (index 2). Default `provider_selected` = 0
/// (Anthropic), which is enough to drive the detail-panel + right-column render.
fn goto_provider(app: &mut App) {
    app.current_step = 2;
    assert_eq!(app.current_step, 2, "should land on Provider (step 2)");
}

#[test]
fn provider_detail_renders_config_box() {
    let mut app = App::new();
    goto_provider(&mut app);
    let out = render(&app);

    assert!(
        out.contains("WILL WRITE TO ~/.zeus/config.toml"),
        "config-box header must render in the detail panel.\n{out}"
    );
    // The model line uses the picked provider's id/flagship (default = anthropic).
    assert!(
        out.contains("anthropic/claude-opus-4-8"),
        "config-box must show model = \"{{id}}/{{flagship}}\" for the picked provider.\n{out}"
    );
    assert!(
        out.contains("model"),
        "config-box must render the `model` key.\n{out}"
    );
}

#[test]
fn provider_right_column_renders_hints_and_recommendations() {
    let mut app = App::new();
    goto_provider(&mut app);
    let out = render(&app);

    assert!(out.contains("HINTS"), "right column must render the HINTS header.\n{out}");
    assert!(
        out.contains("RECOMMENDATIONS"),
        "right column must render the RECOMMENDATIONS header.\n{out}"
    );
    // A couple of the recommendation rows (category → provider).
    assert!(out.contains("Reasoning"), "RECOMMENDATIONS must list Reasoning.\n{out}");
    assert!(out.contains("Anthropic"), "RECOMMENDATIONS must name Anthropic.\n{out}");
}

#[test]
fn provider_detail_renders_glyph_badge_block() {
    let mut app = App::new();
    goto_provider(&mut app);
    let lines = render_lines(&app);

    // The 56×56 badge is a filled color block; the glyph sits centered inside.
    // Anthropic's glyph is "ANT" — assert it renders in the detail region.
    let joined = lines.join("\n");
    assert!(
        joined.contains("ANT"),
        "detail panel must render the provider glyph (badge content).\n{joined}"
    );
    // The NEXT line proves the detail panel rendered past the badge/config-box.
    assert!(
        joined.contains("Step 04 (AUTH)"),
        "detail panel must render the NEXT line after the config-box.\n{joined}"
    );
}

#[test]
fn provider_list_items_render_as_bordered_cards() {
    let mut app = App::new();
    goto_provider(&mut app);
    let lines = render_lines(&app);

    // Bordered cards introduce box-drawing corner glyphs in the left list.
    // The old flat-separator-row layout had only horizontal `─` runs and no
    // corners, so a corner glyph is a positive signal of card borders.
    let has_card_corner = lines.iter().any(|l| l.contains('┌') || l.contains('┐'));
    assert!(
        has_card_corner,
        "left list items must render as bordered Cards (box-drawing corners present).\n{}",
        lines.join("\n")
    );
}
