//! Render-fidelity guard for the Production TUI Approvals tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`ApprovalsTab`, JSX 1189–1245).
//! The tab should render Aegis pending approval cards from live `/v1/approvals`
//! data: header count/key hints, risk-coded cards, args/reason rows, and focused
//! approve/deny/expand affordances — without falling back to stale mock rows.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use zeus_tui::api::ApprovalResponse;
use zeus_tui::prod::ApprovalsTab;

fn render_approvals(widget: ApprovalsTab<'_>) -> (Buffer, String) {
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

fn live_approvals() -> Vec<ApprovalResponse> {
    vec![
        ApprovalResponse {
            id: "ap_47".into(),
            tool_name: "shell".into(),
            args: serde_json::json!("rm -rf node_modules && npm install"),
            agent_id: Some("Hephaestus".into()),
            created_at: "32s ago".into(),
            status: serde_json::json!("pending"),
        },
        ApprovalResponse {
            id: "ap_46".into(),
            tool_name: "web_fetch".into(),
            args: serde_json::json!("https://api.unallowlisted.com/data"),
            agent_id: Some("Hermes".into()),
            created_at: "1m ago".into(),
            status: serde_json::json!({"status": "pending"}),
        },
        ApprovalResponse {
            id: "ap_old".into(),
            tool_name: "discord_send".into(),
            args: serde_json::json!({"channel": "#done"}),
            agent_id: Some("Calliope".into()),
            created_at: "9m ago".into(),
            status: serde_json::json!("approved"),
        },
    ]
}

#[test]
fn approvals_tab_matches_prototype_structure_with_live_queue() {
    let live = live_approvals();
    let widget = ApprovalsTab {
        focused: 0,
        expanded: true,
        live: Some(&live),
    };
    let (_buf, dump) = render_approvals(widget);

    std::fs::create_dir_all("/Users/mike/.zeus/workspace/approvals_phase")
        .expect("create dump dir");
    std::fs::write(
        "/Users/mike/.zeus/workspace/approvals_phase/prod_approvals_1to1_dump.log",
        &dump,
    )
    .expect("write render dump");

    for expected in [
        "2 pending approvals",
        "approve",
        "deny",
        "expand",
        "Hephaestus",
        "shell",
        "HIGH RISK",
        "args",
        "rm -rf node_modules && npm install",
        "WHY BLOCKED",
        "flagged by Aegis",
        "a APPROVE",
        "d DENY",
        "v VIEW FULL",
        "Hermes",
        "web_fetch",
        "MED RISK",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }

    assert!(
        !dump.contains("Calliope"),
        "resolved approvals must not render:\n{dump}"
    );
}

#[test]
fn approvals_tab_renders_honest_empty_state_without_live_rows() {
    let live: Vec<ApprovalResponse> = Vec::new();
    let (_buf, dump) = render_approvals(ApprovalsTab::with_live(Some(&live)));

    assert!(
        dump.contains("0 pending approvals"),
        "empty count missing:\n{dump}"
    );
    assert!(
        dump.contains("no pending approvals"),
        "empty state missing:\n{dump}"
    );
    assert!(
        dump.contains("waiting on /v1/approvals"),
        "live source hint missing:\n{dump}"
    );
}
