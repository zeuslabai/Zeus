//! Fidelity regression for the Auth onboarding screen (step 04, JSX 649–790).
//!
//! The render audit found Auth already preserves the key-mode stack at 100×30:
//! field, validation hint, test-connection action, and config-write preview.
//! Guard that compact shape so later density/chrome changes don't hide it.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

fn render_100x30(app: &App) -> String {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw");
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
fn auth_100x30_keeps_key_validation_action_and_write_preview() {
    let mut app = App::new();
    app.current_step = 4;
    app.set_auth_api_key("sk-ant-test1234");

    let rendered = render_100x30(&app);
    for needle in [
        "Authenticate with Anthropic",
        "API Key",
        "***1234",
        "✓ Key format matches expected prefix",
        "▸ TEST CONNECTION",
        "WILL WRITE TO ~/.zeus/config.toml",
        "[credentials]",
        "ANTHROPIC_API_KEY = \"***1234\"",
    ] {
        assert!(
            rendered.contains(needle),
            "compact Auth render must preserve `{needle}`; got:\n{rendered}"
        );
    }
}
