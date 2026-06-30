//! Phase-1 tests for the Provider+Model unification (fix/tui-1to1-provider-model).
//!
//! Asserts the unification win — ONE shared provider registry consumed by
//! Provider (03) + Fallback (06):
//!   - the canonical registry is exactly our 12 providers;
//!   - membership: Grok/xAI IN, Groq/Mistral/Together/Fireworks/DeepSeek/Azure OUT;
//!   - flagships are current (anthropic claude-opus-4-8, glm-5.2, MiniMax-M3, …);
//!   - the Provider screen renders providers from the shared const;
//!   - Fallback candidates derive from the SAME const (no duplicate list).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::screens::providers::{self, PROVIDERS};

/// Render current app state into a 140×44 TestBackend; return joined text.
fn render(app: &App) -> String {
    let backend = TestBackend::new(140, 44);
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

#[test]
fn registry_is_exactly_our_twelve() {
    assert_eq!(
        PROVIDERS.len(),
        13,
        "canonical registry must be exactly our 13 providers"
    );
    let ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
    for want in [
        "anthropic",
        "openai",
        "google",
        "ollama",
        "gemini-cli",
        "kimi",
        "glm",
        "qwen",
        "minimax",
        "mimo",
        "openrouter",
        "xai",
        "sakana",
    ] {
        assert!(ids.contains(&want), "registry must include `{want}`");
    }
}

#[test]
fn dropped_providers_are_gone() {
    let ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
    for gone in ["groq", "mistral", "together", "fireworks", "deepseek", "azure"] {
        assert!(
            !ids.contains(&gone),
            "`{gone}` must be dropped from the canonical 12"
        );
    }
}

#[test]
fn flagships_are_current() {
    let f = |id: &str| providers::by_id(id).map(|p| p.flagship).unwrap_or("");
    assert_eq!(f("anthropic"), "claude-opus-4-8", "anthropic flagship stale");
    assert_eq!(f("glm"), "glm-5.2", "glm flagship stale");
    assert_eq!(f("minimax"), "MiniMax-M3", "minimax flagship stale");
    assert_eq!(f("kimi"), "kimi-k2.7-code", "kimi flagship stale");
    assert_eq!(f("openrouter"), "auto", "openrouter flagship stale");
}

#[test]
fn provider_screen_renders_from_shared_const() {
    // Provider = step 2.
    let mut app = App::new();
    app.current_step = 2;
    let screen = render(&app);

    // Our 12 names should be present somewhere on the Provider screen — and the
    // dropped ones should not. (At least the first-column names are visible.)
    assert!(screen.contains("Anthropic"), "Provider screen must list Anthropic");
    // Grok/xAI is in; Groq is out — the historically-conflated pair.
    assert!(
        !screen.contains("Groq"),
        "Provider screen must NOT show the dropped Groq"
    );
}

#[test]
fn fallback_candidates_derive_from_shared_const() {
    // Fallback = step 5. Its candidate list is now `providers::PROVIDERS`
    // filtered by primary, so the dropped providers can never reappear here.
    let mut app = App::new();
    app.current_step = 5;
    let screen = render(&app);

    assert!(
        !screen.contains("Groq") && !screen.contains("Mistral"),
        "Fallback candidates derive from the canonical 12 — no dropped providers"
    );
}
