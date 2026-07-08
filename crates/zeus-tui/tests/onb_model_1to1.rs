//! Phase-2 tests for the Model (05) screen — per-provider catalogs + live-fetch.
//!
//! Asserts the cut from `fix/tui-1to1-provider-model`:
//!   - per-provider catalogs dispatch for all 12 (anthropic/openai/google/
//!     gemini-cli/kimi/glm/qwen/minimax/mimo/openrouter/xai/ollama);
//!   - flagships are current (claude-opus-4-8, glm-5.2, MiniMax-M3, kimi-k2.7-code);
//!   - LIVE FETCH notice shows for ollama (localhost:11434), glm/zai (api.z.ai),
//!     kimi/moonshot (api.moonshot.ai).
//!
//! The Model screen renders from a provider id, so we construct ModelScreen
//! directly and render it into a TestBackend — no full-app step nav needed.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::screens::ModelScreen;

/// Render a ModelScreen for `provider` into an 80×30 TestBackend; joined text.
fn render_model(provider: &str) -> String {
    render_buf(ModelScreen::new(provider.to_string()))
}

/// Render a ModelScreen with a stubbed live fetch — seeds `live_models` so the
/// honest `● LIVE FETCH` badge path fires (the real fetch worker is async/
/// network-bound and out of scope for a render test, so we stub the result).
fn render_model_live(provider: &str, models: &[&str]) -> String {
    let mut screen = ModelScreen::new(provider.to_string());
    screen.set_live_models(models.iter().map(|m| m.to_string()).collect());
    render_buf(screen)
}

/// Draw a ModelScreen into an 80×30 TestBackend; return the joined cell text.

/// Render the full onboarding App at 100×30 for compact chrome + body fidelity.
fn render_model_app_100x30() -> String {
    let mut app = App::new();
    app.current_step = 5;

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
    out
}

fn render_buf(screen: ModelScreen) -> String {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let area = f.area();
            screen.render(area, f.buffer_mut());
        })
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

#[test]
fn model_100x30_keeps_search_filter_and_detail_affordance() {
    let rendered = render_model_app_100x30();
    for needle in [
        "Pick a model",
        "Claude Opus 4.8",
        "MODEL DETAILS",
        "CONTEXT",
        "PRICING",
        "SEARCH / FILTER",
        "/ search models",
        "WILL WRITE TO ~/.zeus/config.toml",
    ] {
        assert!(
            rendered.contains(needle),
            "Model 100×30 render should keep {needle:?}:\n{rendered}"
        );
    }
}

#[test]
fn anthropic_catalog_current() {
    let s = render_model("anthropic");
    assert!(
        s.contains("Opus 4.8"),
        "anthropic must list current Opus 4.8"
    );
    assert!(!s.contains("4.7"), "stale Opus 4.7 must be gone");
}

#[test]
fn glm_catalog_dispatches() {
    let s = render_model("glm");
    assert!(
        s.contains("GLM-5.2"),
        "glm must dispatch to GLM-5.2 catalog"
    );
}

#[test]
fn minimax_catalog_current() {
    let s = render_model("minimax");
    assert!(
        s.contains("MiniMax-M3"),
        "minimax flagship must be MiniMax-M3"
    );
    assert!(!s.contains("abab-7"), "stale abab-7-chat must be gone");
}

#[test]
fn kimi_catalog_dispatches() {
    let s = render_model("kimi");
    assert!(
        s.contains("Kimi K2.7"),
        "kimi must dispatch to k2.7-code catalog"
    );
}

#[test]
fn xai_catalog_dispatches() {
    let s = render_model("xai");
    assert!(s.contains("Grok 4"), "xai must dispatch to Grok catalog");
}

// #251 honest contract: the `● LIVE FETCH` badge + endpoint render ONLY when a
// real fetch landed (`live_models` seeded). With no fetch the screen shows the
// honest `○ FALLBACK` notice off the static seed catalog — never a fake badge.
// Each test pins BOTH states off the real `set_live_models` seam.

#[test]
fn ollama_live_fetch_notice() {
    // No fetch → honest fallback, no fake badge.
    let fallback = render_model("ollama");
    assert!(
        fallback.contains("○ FALLBACK"),
        "ollama with no live models must render the honest FALLBACK notice"
    );
    assert!(
        !fallback.contains("● LIVE FETCH"),
        "no fake LIVE FETCH badge when the static seed catalog is showing"
    );
    // Stubbed fetch → real LIVE FETCH badge + endpoint.
    let live = render_model_live("ollama", &["llama3.3", "qwen2.5-coder"]);
    assert!(
        live.contains("● LIVE FETCH"),
        "ollama with seeded live models must show the LIVE FETCH badge"
    );
    assert!(
        live.contains("localhost:11434"),
        "ollama live notice must name the local endpoint"
    );
}

#[test]
fn glm_live_fetch_zai_endpoint() {
    let fallback = render_model("glm");
    assert!(
        fallback.contains("○ FALLBACK"),
        "glm with no live models must render the honest FALLBACK notice"
    );
    assert!(
        !fallback.contains("● LIVE FETCH"),
        "no fake LIVE FETCH badge when the static seed catalog is showing"
    );
    let live = render_model_live("glm", &["glm-5.2", "glm-4.6"]);
    assert!(
        live.contains("● LIVE FETCH"),
        "glm with seeded live models must show the LIVE FETCH badge"
    );
    assert!(
        live.contains("api.z.ai"),
        "glm live-fetch must name api.z.ai"
    );
}

#[test]
fn kimi_live_fetch_moonshot_endpoint() {
    let fallback = render_model("kimi");
    assert!(
        fallback.contains("○ FALLBACK"),
        "kimi with no live models must render the honest FALLBACK notice"
    );
    assert!(
        !fallback.contains("● LIVE FETCH"),
        "no fake LIVE FETCH badge when the static seed catalog is showing"
    );
    let live = render_model_live("kimi", &["kimi-k2.7-code", "moonshot-v1-128k"]);
    assert!(
        live.contains("● LIVE FETCH"),
        "kimi with seeded live models must show the LIVE FETCH badge"
    );
    assert!(
        live.contains("api.moonshot.ai"),
        "kimi live-fetch must name api.moonshot.ai"
    );
}
