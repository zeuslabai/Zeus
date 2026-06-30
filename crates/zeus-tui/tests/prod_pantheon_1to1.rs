//! Render-fidelity guard for the Production TUI Pantheon tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`PantheonTab`, JSX 647–812).
//! Production keeps the prototype's three-pane mission-control structure while
//! rendering live `/v1/pantheon/missions` rows and honest empty states instead
//! of the old fabricated catalog/transcript/event fixtures.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use zeus_tui::api::PantheonMissionResponse;
use zeus_tui::prod::{PantheonLive, PantheonTab};

fn render_pantheon(widget: PantheonTab<'_>) -> (Buffer, String) {
    let backend = TestBackend::new(120, 34);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| widget.render(f.area(), f.buffer_mut()))
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    (buf.clone(), dump_buffer(&buf))
}

fn dump_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

#[test]
fn pantheon_tab_renders_live_mission_control_shell() {
    let live = vec![
        PantheonMissionResponse {
            id: "mission-alpha".into(),
            name: "Live mission alpha".into(),
            status: "active".into(),
            agent_count: 4,
        },
        PantheonMissionResponse {
            id: "mission-beta".into(),
            name: "Live mission beta".into(),
            status: "reviewing".into(),
            agent_count: 2,
        },
    ];

    let (buf, dump) = render_pantheon(PantheonTab::with_live(
        0,
        PantheonLive {
            missions: Some(&live),
        },
    ));
    std::fs::write(
        "/Users/mike/.zeus/workspace/pantheon_phase/prod_pantheon_1to1_dump.log",
        &dump,
    )
    .ok();

    for expected in [
        "MISSIONS",
        "2 live",
        "Active war rooms + scheduled work",
        "Live mission alpha",
        "Live mission beta",
        "4 agents · live",
        "WAR ROOM",
        "#live-mission-alpha",
        "No live war-room transcript yet",
        "PLAN CARD",
        "No pending plan awaiting approval",
        "LIVE EVENTS",
        "No live Pantheon events",
        "stream ready · /v1/pantheon/rooms/",
        "n new mission",
        "p pause",
        "c cancel",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }

    assert_eq!(buf.area.width, 120);
    assert_eq!(buf.area.height, 34);
}

#[test]
fn pantheon_tab_does_not_leak_fabricated_prototype_data() {
    let live = vec![PantheonMissionResponse {
        id: "real-1".into(),
        name: "Actual gateway mission".into(),
        status: "planning".into(),
        agent_count: 1,
    }];

    let (_buf, dump) = render_pantheon(PantheonTab::with_live(
        0,
        PantheonLive {
            missions: Some(&live),
        },
    ));

    assert!(
        dump.contains("Actual gateway mission"),
        "live row missing:\n{dump}"
    );
    for fake in [
        "v0.4.7 release prep",
        "Onboarding wizard impl",
        "Fleet shakedown audit",
        "DGX Spark integration",
        "Hephaestus",
        "Hermes",
        "Atlas",
        "Phase 2 — Tools browser + memory tab",
        "PR #2847",
        "7,801 passed",
    ] {
        assert!(
            !dump.contains(fake),
            "fabricated data leaked {fake:?}:\n{dump}"
        );
    }
}

#[test]
fn pantheon_tab_renders_honest_empty_state_without_live_rows() {
    let live: Vec<PantheonMissionResponse> = Vec::new();
    let (_buf, dump) = render_pantheon(PantheonTab::with_live(
        0,
        PantheonLive {
            missions: Some(&live),
        },
    ));

    assert!(dump.contains("0 live"), "empty live count missing:\n{dump}");
    assert!(
        dump.contains("no live missions"),
        "empty mission state missing:\n{dump}"
    );
    assert!(
        dump.contains("waiting on /v1/pantheon/missions"),
        "empty source hint missing:\n{dump}"
    );
    assert!(
        dump.contains("No Pantheon mission selected"),
        "center empty state missing:\n{dump}"
    );
}
