//! Render-fidelity guard for Production TUI Office.
//!
//! SoT: `docs/zeus-the-office-tui.jsx`.
//! P1: static 96×48 pixel office rendered as 24 half-block rows.
//! P2: live `/v1/network/agents` sprites, speech bubbles, and sidebar.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ratatui::Terminal;
use std::collections::HashMap;
use std::path::Path;

use zeus_tui::api::{AgentResponse, StatusResponse};
use zeus_tui::prod::top_bar::ConnState;
use zeus_tui::prod::{OfficeLive, OfficeTab, ProdTopBar};

fn render_office() -> (Buffer, String) {
    render_office_widget(OfficeTab::new())
}

fn render_office_widget(widget: OfficeTab<'_>) -> (Buffer, String) {
    let backend = TestBackend::new(132, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let area = f.area();
            widget.render(area, f.buffer_mut());
        })
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    (buf.clone(), dump_buffer(&buf))
}

fn render_prod_top_bar(width: u16) -> String {
    let backend = TestBackend::new(width, 1);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let area = f.area();
            ProdTopBar {
                hostname: "zeus-titan".to_string(),
                port: 8080,
                conn_state: ConnState::Connected,
                ctx_percent: 42,
            }
            .render(area, f.buffer_mut());
        })
        .expect("draw must not panic");
    dump_buffer(terminal.backend().buffer())
}

fn maybe_write_office_polish_dump(dump: &str) {
    if let Ok(path) = std::env::var("ZEUS_OFFICE_POLISH_RENDER_DUMP") {
        let path = Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dump dir");
        }
        std::fs::write(path, dump).expect("write office polish render dump");
    }
}

fn dump_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row);
    }
    lines.join("\n")
}

#[test]
fn prod_office_p1_render_dump() {
    let (_buf, dump) = render_office();
    println!("\n{dump}\n");
    assert!(dump.contains("ENGINEERING"), "zone label missing:\n{dump}");
    assert!(dump.contains("COMMS"), "zone label missing:\n{dump}");
    assert!(dump.contains("RESEARCH"), "zone label missing:\n{dump}");
    assert!(dump.contains("BREAK ROOM"), "zone label missing:\n{dump}");
    assert!(dump.contains("KITCHEN"), "zone label missing:\n{dump}");
    assert!(dump.contains("FLEET STATUS"), "sidebar missing:\n{dump}");
    assert!(
        dump.contains("/v1/network/agents"),
        "empty-live sidebar should point at the real network agents feed:\n{dump}"
    );
}

#[test]
fn prod_office_p1_static_scene_uses_96x48_half_block_grid() {
    let (buf, dump) = render_office();

    // The first 96 columns of the first 24 rows are the Office prototype's
    // 96×48 pixel grid, packed as top/bottom colors in `▀` cells. Zone labels
    // deliberately overwrite a few cells as text overlays; the material samples
    // below avoid those label positions.
    for y in 0..24 {
        let half_blocks = (0..96).filter(|x| buf[(*x, y)].symbol() == "▀").count();
        assert!(
            half_blocks >= 78,
            "row {y} should be mostly half-block scene pixels after zone-label overlays, got {half_blocks}:\n{dump}"
        );
    }

    // Ceiling light top/bottom pixels at prototype x=10, y=0/1.
    assert_eq!(buf[(10, 0)].symbol(), "▀");
    assert_eq!(buf[(10, 0)].fg, Color::Rgb(0xff, 0xd8, 0x80));
    assert_eq!(buf[(10, 0)].bg, Color::Rgb(0xe0, 0xd8, 0xc0));

    // Engineering CRT: prototype x=6, y=10/11 -> terminal row 5.
    assert_eq!(buf[(6, 5)].symbol(), "▀");
    assert_eq!(buf[(6, 5)].fg, Color::Rgb(0x1a, 0x30, 0x48));
    assert_eq!(buf[(6, 5)].bg, Color::Rgb(0x1a, 0x30, 0x48));

    // Comms monitor: prototype x=57, y=10/11 -> terminal row 5.
    assert_eq!(buf[(57, 5)].symbol(), "▀");
    assert_eq!(buf[(57, 5)].fg, Color::Rgb(0x0e, 0x18, 0x24));
    assert_eq!(buf[(57, 5)].bg, Color::Rgb(0x3b, 0x82, 0xf6));

    // Research bookshelf: prototype x=25, y=24/25 -> terminal row 12.
    assert_eq!(buf[(25, 12)].symbol(), "▀");
    assert_eq!(buf[(25, 12)].fg, Color::Rgb(0x4a, 0x38, 0x28));
    assert_eq!(buf[(25, 12)].bg, Color::Rgb(0xa0, 0x60, 0x20));

    // Break-room carpet edge: prototype x=56, y=30/31 -> terminal row 15.
    assert_eq!(buf[(56, 15)].symbol(), "▀");
    assert_eq!(buf[(56, 15)].fg, Color::Rgb(0x56, 0x28, 0x30));

    // Kitchen coffee machine: prototype x=4, y=32/33 -> terminal row 16.
    assert_eq!(buf[(4, 16)].symbol(), "▀");
    assert_eq!(buf[(4, 16)].fg, Color::Rgb(0x4a, 0x4a, 0x4a));
    assert_eq!(buf[(4, 16)].bg, Color::Rgb(0x3a, 0x3a, 0x3a));
}

#[test]
fn prod_office_p2_live_agents_sidebar_and_sprites() {
    fn agent(name: &str, status: &str, task: &str, model: &str) -> AgentResponse {
        let mut metadata = HashMap::new();
        metadata.insert("model".to_string(), model.to_string());
        AgentResponse {
            id: name.to_ascii_lowercase().replace(' ', "-"),
            name: name.to_string(),
            status: status.to_string(),
            metadata,
            current_task: Some(task.to_string()),
            ..Default::default()
        }
    }

    let agents = vec![
        agent(
            "Zeus Prime",
            "executing",
            "cargo build --release",
            "claude-sonnet-4",
        ),
        agent("Hermes", "syncing", "Discord: #ops-alerts", "gpt-4o"),
        agent(
            "Athena",
            "researching",
            "LLM accuracy benchmark",
            "llama-3.3-70b",
        ),
        agent("Prometheus", "idle", "Coffee break", "claude-sonnet-4"),
    ];
    let status = StatusResponse {
        status: "ok".to_string(),
        model: "claude-sonnet-4".to_string(),
        provider: "anthropic".to_string(),
        gateway_url: "http://127.0.0.1:8080".to_string(),
        agent_name: "zeus-titan".to_string(),
        ..Default::default()
    };
    let live = OfficeLive {
        agents: Some(&agents),
        status: Some(&status),
    };

    let (buf, dump) = render_office_widget(OfficeTab::with_live(Some(0), live));
    println!("\n{dump}\n");

    assert!(dump.contains("FLEET STATUS"));
    assert!(dump.contains("EVENT LOG"));
    assert!(dump.contains("ZONES"));
    assert!(dump.contains("Zeus Prime EXEC"));
    assert!(dump.contains("Hermes SYNC"));
    assert!(dump.contains("Athena RESEARCH"));
    assert!(dump.contains("Prometheus IDLE"));
    assert!(dump.contains("cargo build"));
    assert!(dump.contains("Discord"));
    assert!(dump.contains("engineering"));
    assert!(dump.contains("breakroom"));
    assert!(!dump.contains("P1 STATIC SCENE"));
    assert!(!dump.contains("agents/sidebar in P2"));

    // Sidebar Fleet Status dot for the executing Zeus agent uses prototype green.
    assert_eq!(buf[(106, 3)].symbol(), "●");
    assert_eq!(buf[(106, 3)].fg, Color::Rgb(0x22, 0xc5, 0x5e));

    // Sprite shirt pixels are composed into the 96×48 half-block scene in the
    // Engineering zone from live agent state, not from a mock const roster.
    assert_eq!(buf[(20, 7)].symbol(), "▀");
    assert_eq!(buf[(20, 7)].fg, Color::Rgb(0x22, 0xc5, 0x5e));

    // Speech bubble sits over the scene and uses the live task text.
    assert!(dump.contains("Zeus · cargo build"));
}

#[test]
fn prod_office_polish_keeps_header_logo_and_collapses_doubled_agent_name() {
    let mut evidence = String::new();
    for width in [100_u16, 112, 132] {
        let dump = render_prod_top_bar(width);
        evidence.push_str(&format!("--- prod top bar width {width} ---\n{dump}\n"));
        let first = dump.lines().next().unwrap_or_default();
        assert!(
            first.starts_with("ZEUS "),
            "prod header must keep ZEUS flush-left at width {width}: {first:?}"
        );
        assert!(
            !first.starts_with("EUS"),
            "prod header clipped leading Z at width {width}: {first:?}"
        );
    }

    let mut metadata = HashMap::new();
    metadata.insert("model".to_string(), "fugu-ultra".to_string());
    let agents = vec![AgentResponse {
        id: "zeuszeus-titan".to_string(),
        name: "zeuszeus-titan".to_string(),
        status: "executing".to_string(),
        metadata,
        current_task: Some("office polish".to_string()),
        ..Default::default()
    }];
    let live = OfficeLive {
        agents: Some(&agents),
        status: None,
    };
    let (_, office_dump) = render_office_widget(OfficeTab::with_live(Some(0), live));
    evidence.push_str("--- office doubled-name regression ---\n");
    evidence.push_str(&office_dump);
    maybe_write_office_polish_dump(&evidence);

    assert!(
        office_dump.contains("zeus-titan"),
        "Office Fleet Status/Event Log should render normalized agent name:\n{office_dump}"
    );
    assert!(
        !office_dump.contains("zeuszeus-titan"),
        "Office must not render doubled zeus prefix in Fleet Status/Event Log:\n{office_dump}"
    );
}

#[test]
fn prod_office_p3_motion_overlays_and_status_bar() {
    fn agent(name: &str, status: &str, task: &str, model: &str) -> AgentResponse {
        let mut metadata = HashMap::new();
        metadata.insert("model".to_string(), model.to_string());
        AgentResponse {
            id: name.to_ascii_lowercase().replace(' ', "-"),
            name: name.to_string(),
            status: status.to_string(),
            metadata,
            current_task: Some(task.to_string()),
            ..Default::default()
        }
    }

    let agents = vec![
        agent("Zeus Prime", "executing", "cargo build --release", "claude-sonnet-4"),
        agent("Hermes", "syncing", "Discord: #ops-alerts", "gpt-4o"),
        agent("Athena", "researching", "LLM accuracy benchmark", "llama-3.3-70b"),
        agent("Prometheus", "idle", "Coffee break", "claude-sonnet-4"),
    ];
    let status = StatusResponse {
        status: "ok".to_string(),
        model: "claude-sonnet-4".to_string(),
        provider: "anthropic".to_string(),
        gateway_url: "http://127.0.0.1:8080".to_string(),
        agent_name: "zeus-titan".to_string(),
        ..Default::default()
    };
    let live = OfficeLive {
        agents: Some(&agents),
        status: Some(&status),
    };

    let (buf, dump) = render_office_widget(
        OfficeTab::with_live(Some(1), live)
            .with_tick(19)
            .with_memo(true)
            .with_help(true),
    );

    assert!(dump.contains("YESTERDAY'S MEMO"));
    assert!(dump.contains("Deployed NovaTradeEngine"));
    assert!(dump.contains("CONTROLS"));
    assert!(dump.contains("Esc"));
    assert!(dump.contains("the-office"));
    assert!(dump.contains("4 agents"));
    assert!(dump.contains("3 active"));
    assert!(dump.contains("tick 19"));
    assert!(dump.contains("M Memo"));
    assert!(dump.contains("Tab Focus"));
    assert!(dump.contains("? Help"));

    // Tick-driven motion should move the first live Engineering sprite away
    // from its P2 static anchor while preserving the half-block scene render.
    assert_eq!(buf[(20, 7)].symbol(), "▀");
    assert_ne!(buf[(20, 7)].fg, Color::Rgb(0x22, 0xc5, 0x5e));
}
