//! 1:1 render tests for onboarding Screen 09/19 — Gateway (GTWY) · JSX 1191–1262.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-gateway`:
//!   • header "Configure gateway" + sub,
//!   • BIND section: Host field + "Use 0.0.0.0 to expose on LAN" hint,
//!     Port field + port-in-use error line,
//!   • FEATURES: 3 pill toggles (Agent Processing Loop ON · WebUI Co-host ON ·
//!     MCP Server OFF) with labels+descs, 1/2/3 toggle them,
//!   • INSTALL AS SERVICE: 4-col card grid (launchd/systemd/rc.d/Manual) +
//!     "WILL INSTALL {path}" box reflecting the selected service,
//!   • ←/→ moves the service-grid selection (NOT the step),
//!   • ESC backs out one step (does not quit),
//!   • host/port text input is panic-free on multibyte.
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const GATEWAY_STEP: usize = 9;

/// Walk to the Gateway step. Grid steps (Channels=6, Gateway=8) consume Right
/// for in-screen focus, so we bump those directly — mirrors goto_step in onb_106.
fn goto_gateway(app: &mut App) {
    while app.current_step < GATEWAY_STEP {
        if app.current_step == 4 {
            app.current_step += 1;
            app.on_step_enter();
            continue;
        }
        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 7 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
    }
    assert_eq!(
        app.current_step, GATEWAY_STEP,
        "failed to reach Gateway step"
    );
}

/// Render the current app into a 120×44 TestBackend → newline-joined String.
fn render(app: &App) -> String {
    render_at(app, 120, 44)
}

fn render_at(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
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
// ── header + the 3 section labels ──────────────────────────────────────────

#[test]
fn gateway_renders_header_and_three_sections() {
    let mut app = App::new();
    goto_gateway(&mut app);
    let screen = render(&app);
    // Header + sub (driven by App::current_step header/sub).
    assert!(screen.contains("Configure gateway"), "header missing");
    assert!(
        screen.contains("hosts the API"),
        "sub-copy missing (expected 'The gateway hosts the API, WebUI, ...')"
    );
    // The three section labels, 1:1 with JSX.
    assert!(screen.contains("BIND"), "BIND section label missing");
    assert!(
        screen.contains("FEATURES"),
        "FEATURES section label missing"
    );
    assert!(
        screen.contains("INSTALL AS SERVICE"),
        "INSTALL AS SERVICE section label missing"
    );
}

// ── BIND: host/port defaults + LAN hint ────────────────────────────────────

#[test]
fn gateway_bind_fields_and_lan_hint() {
    let mut app = App::new();
    goto_gateway(&mut app);
    let screen = render(&app);
    assert!(screen.contains("Host"), "Host field label missing");
    assert!(screen.contains("127.0.0.1"), "Host default missing");
    assert!(screen.contains("Port"), "Port field label missing");
    assert!(screen.contains("8080"), "Port default missing");
    // The JSX host hint must render under the Host field.
    assert!(
        screen.contains("Use 0.0.0.0 to expose on LAN"),
        "Host LAN hint missing"
    );
}

// ── BIND: port-in-use error line (static example per spec, real probe = follow-up)

#[test]
fn gateway_port_in_use_error_line() {
    let mut app = App::new();
    goto_gateway(&mut app);
    // No error by default.
    assert!(
        !render(&app).contains("in use by PID"),
        "port-in-use error should be hidden by default"
    );
    // Flip the (currently static) probe flag → the error line renders.
    app.gateway_screen.port_in_use = true;
    let screen = render(&app);
    assert!(
        screen.contains("in use by PID"),
        "port-in-use error line should render when port_in_use is set"
    );
    assert!(
        screen.contains("Pick a different port"),
        "port-in-use remediation copy missing"
    );
}

// ── FEATURES: 3 toggles with labels+descs, and 1/2/3 flip them ─────────────

#[test]
fn gateway_features_render_with_defaults() {
    let mut app = App::new();
    goto_gateway(&mut app);
    let screen = render(&app);
    // All three feature labels.
    assert!(
        screen.contains("Agent Processing Loop"),
        "Agent Processing Loop label missing"
    );
    assert!(
        screen.contains("WebUI Co-host"),
        "WebUI Co-host label missing"
    );
    assert!(screen.contains("MCP Server"), "MCP Server label missing");
    // A couple of the descriptions (1:1 copy).
    assert!(
        screen.contains("Background heartbeat"),
        "Agent Processing Loop desc missing"
    );
    assert!(
        screen.contains("Model Context Protocol"),
        "MCP Server desc missing"
    );
    // Defaults: agent_processing ON, webui ON, mcp OFF.
    assert_eq!(
        app.gateway_screen.features,
        [true, true, false],
        "feature defaults must be [ON, ON, OFF]"
    );
}

#[test]
fn gateway_feature_toggle_keys_flip_pills() {
    let mut app = App::new();
    goto_gateway(&mut app);
    // '3' toggles MCP (default OFF) → ON.
    app.handle_key(KeyCode::Char('3'));
    assert!(
        app.gateway_screen.features[2],
        "'3' should toggle MCP Server ON"
    );
    // '1' toggles Agent Processing Loop (default ON) → OFF.
    app.handle_key(KeyCode::Char('1'));
    assert!(
        !app.gateway_screen.features[0],
        "'1' should toggle Agent Processing Loop OFF"
    );
    // Still on the Gateway step — toggles don't step-nav.
    assert_eq!(
        app.current_step, GATEWAY_STEP,
        "toggles must not change step"
    );
}

// ── INSTALL AS SERVICE: 4-col grid + WILL INSTALL box ──────────────────────

#[test]
fn gateway_service_grid_and_will_install_box() {
    let mut app = App::new();
    goto_gateway(&mut app);
    let screen = render(&app);
    // All 4 service glyphs render (4-col grid).
    for glyph in ["MAC", "LIN", "BSD"] {
        assert!(screen.contains(glyph), "service glyph {glyph} missing");
    }
    assert!(screen.contains("launchd"), "launchd service name missing");
    assert!(screen.contains("systemd"), "systemd service name missing");
    assert!(screen.contains("rc.d"), "rc.d service name missing");
    assert!(screen.contains("Manual"), "Manual service name missing");
    // Default selection = launchd (idx 0) → WILL INSTALL box shows its path.
    assert!(screen.contains("WILL INSTALL"), "WILL INSTALL box missing");
    assert!(
        screen.contains("ai.zeuslab.gateway.plist"),
        "launchd install path missing from WILL INSTALL box"
    );
}

// ── ←/→ moves the service-grid selection, NOT the step ─────────────────────

#[test]
fn gateway_arrow_moves_service_not_step() {
    let mut app = App::new();
    goto_gateway(&mut app);
    assert_eq!(
        app.gateway_screen.service_mode, 0,
        "default service = launchd"
    );
    // → moves to systemd (idx 1), step unchanged.
    app.handle_key(KeyCode::Right);
    assert_eq!(
        app.gateway_screen.service_mode, 1,
        "→ should select systemd"
    );
    assert_eq!(app.current_step, GATEWAY_STEP, "→ must not step-nav");
    // → again to rc.d (idx 2). Its install path replaces the WILL INSTALL box.
    app.handle_key(KeyCode::Right);
    assert_eq!(app.gateway_screen.service_mode, 2, "→ should select rc.d");
    let screen = render(&app);
    assert!(
        screen.contains("zeus_gateway"),
        "WILL INSTALL box should reflect rc.d's path after →"
    );
    // ← moves back to systemd, step still unchanged.
    app.handle_key(KeyCode::Left);
    assert_eq!(
        app.gateway_screen.service_mode, 1,
        "← should move back to systemd"
    );
    assert_eq!(app.current_step, GATEWAY_STEP, "← must not step-back");
}

#[test]
fn gateway_manual_service_has_no_install_box() {
    let mut app = App::new();
    goto_gateway(&mut app);
    // Walk to Manual (idx 3) — it has path=None → no WILL INSTALL box.
    app.handle_key(KeyCode::Right);
    app.handle_key(KeyCode::Right);
    app.handle_key(KeyCode::Right);
    assert_eq!(app.gateway_screen.service_mode, 3, "should reach Manual");
    let screen = render(&app);
    assert!(
        !screen.contains("WILL INSTALL"),
        "Manual (no path) must not render a WILL INSTALL box"
    );
}

// ── ESC backs out one step (does not quit) ─────────────────────────────────

#[test]
fn gateway_esc_backs_out_one_step() {
    let mut app = App::new();
    goto_gateway(&mut app);
    assert_eq!(app.current_step, GATEWAY_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        GATEWAY_STEP - 1,
        "ESC should back out one step, not quit"
    );
}

// ── host/port text input is panic-free (incl. multibyte) ───────────────────

#[test]
fn gateway_field_input_multibyte_safe() {
    let mut app = App::new();
    goto_gateway(&mut app);
    // Focus stays on host (field 0) by default; type a multibyte char.
    app.handle_key(KeyCode::Char('é'));
    assert!(
        app.gateway_screen.host.ends_with('é'),
        "multibyte char should append to host field"
    );
    // Backspace removes it without panicking on the codepoint boundary.
    app.handle_key(KeyCode::Backspace);
    assert!(
        !app.gateway_screen.host.ends_with('é'),
        "backspace should remove the multibyte char cleanly"
    );
    // Render must not panic with the edited field.
    let _ = render(&app);
}

#[test]
fn gateway_100x30_renders_bind_probe_affordance() {
    let mut app = App::new();
    goto_gateway(&mut app);
    app.gateway_screen.host = "0.0.0.0".to_string();
    app.gateway_screen.port = "9099".to_string();

    let screen = render_at(&app, 100, 30);
    assert!(screen.contains("BIND"), "BIND section missing at 100x30");
    assert!(
        screen.contains("Host *"),
        "required Host marker missing at 100x30"
    );
    assert!(
        screen.contains("Port *"),
        "required Port marker missing at 100x30"
    );
    assert!(
        screen.contains("● PROBE"),
        "available-port probe missing at 100x30"
    );
    assert!(
        screen.contains("0.0.0.0:9099 available"),
        "probe target/copy missing at 100x30:\n{screen}"
    );
}
