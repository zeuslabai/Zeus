use ratatui::backend::TestBackend;
use ratatui::Terminal;
use zeus_tui::api::{ExtensionResponse, McpServerResponse, ProjectResponse, WorkflowResponse};
use zeus_tui::prod::advanced_sub::{self, AdvancedLive};

fn render_dump(tab_id: &'static str, live: AdvancedLive<'_>) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            advanced_sub::render(tab_id, area, f.buffer_mut(), &live);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
}

#[test]
fn mcp_subview_renders_live_servers_without_fixture_fallback() {
    let servers = vec![
        McpServerResponse {
            name: "filesystem".into(),
            command: "npx".into(),
            transport: "stdio".into(),
        },
        McpServerResponse {
            name: "browser".into(),
            command: "node".into(),
            transport: "sse".into(),
        },
    ];

    let dump = render_dump(
        "mcp",
        AdvancedLive {
            mcp: Some(&servers),
            ..AdvancedLive::default()
        },
    );

    assert!(dump.contains("2/2 connected"));
    assert!(dump.contains("0 tools exposed"));
    assert!(dump.contains("ADD SERVER"));
    assert!(dump.contains("filesystem"));
    assert!(dump.contains("browser"));
    assert!(dump.contains("STDIO"));
    assert!(dump.contains("SSE"));
    assert!(dump.contains("[INSPECT]"));
    assert!(!dump.contains("fetching from /v1/mcp/servers"));
}

#[test]
fn projects_subview_renders_live_project_rows_and_honest_empty_state() {
    let projects = vec![ProjectResponse {
        name: "prod-tui".into(),
        status: "active".into(),
        agents: vec![
            serde_json::json!("zeus-titan"),
            serde_json::json!("zeus100"),
        ],
        lead: "zeus-titan".into(),
        progress: 72,
    }];

    let dump = render_dump(
        "projects",
        AdvancedLive {
            projects: Some(&projects),
            ..AdvancedLive::default()
        },
    );

    assert!(dump.contains("1 active"));
    assert!(dump.contains("1 total"));
    assert!(dump.contains("NEW PROJECT"));
    assert!(dump.contains("prod-tui"));
    assert!(dump.contains("ACTIVE"));
    assert!(dump.contains("lead zeus-titan"));
    assert!(dump.contains("2 agents"));
    assert!(dump.contains("72%"));
    assert!(!dump.contains("Hermes Refactor"));

    let empty = render_dump(
        "projects",
        AdvancedLive {
            projects: Some(&[]),
            ..AdvancedLive::default()
        },
    );
    assert!(empty.contains("No projects"));
    assert!(empty.contains("/v1/projects"));
    assert!(!empty.contains("Hermes Refactor"));
}

#[test]
fn canvas_subview_renders_live_workflow_progress_without_mock_nodes() {
    let workflows = vec![WorkflowResponse {
        workflow_id: "wf-chat-feedback".into(),
        status: "running".into(),
        message: "render-gate batch2".into(),
        progress_percentage: 66.0,
        total_nodes: 6,
        completed_nodes: 4,
        failed_nodes: 1,
        created_at: "2026-06-28T00:00:00Z".into(),
    }];

    let dump = render_dump(
        "canvas",
        AdvancedLive {
            workflows: Some(&workflows),
            ..AdvancedLive::default()
        },
    );

    assert!(dump.contains("research-flow"));
    assert!(dump.contains("1 nodes"));
    assert!(dump.contains("NEW FLOW"));
    assert!(dump.contains("render-gate batch2"));
    assert!(dump.contains("4/6 nodes"));
    assert!(dump.contains("RUNNING"));
    assert!(!dump.contains("Prototype workflow"));
}

#[test]
fn extensions_subview_renders_live_extensions_and_statuses_without_overlay_drift() {
    let extensions = vec![
        ExtensionResponse {
            name: "github".into(),
            version: "1.2.3".into(),
            status: serde_json::json!("Running"),
            extension_type: "deno".into(),
        },
        ExtensionResponse {
            name: "broken".into(),
            version: "0.1.0".into(),
            status: serde_json::json!({"Error": "boom"}),
            extension_type: "mcp".into(),
        },
    ];

    let dump = render_dump(
        "extensions",
        AdvancedLive {
            extensions: Some(&extensions),
            ..AdvancedLive::default()
        },
    );
    std::fs::create_dir_all("/Users/mike/.zeus/workspace/advanced_sub_batch2").unwrap();
    std::fs::write(
        "/Users/mike/.zeus/workspace/advanced_sub_batch2/prod_advanced_sub_batch2_dump.log",
        &dump,
    )
    .unwrap();

    assert!(dump.contains("NAME"));
    assert!(dump.contains("VERSION"));
    assert!(dump.contains("STATUS"));
    assert!(dump.contains("RUNTIME"));
    assert!(dump.contains("github"));
    assert!(dump.contains("1.2.3"));
    assert!(dump.contains("running"));
    assert!(dump.contains("deno"));
    assert!(dump.contains("broken"));
    assert!(dump.contains("error"));
    assert!(dump.contains("2 extensions · 1 active · 0 idle · 1 error"));
    assert!(!dump.contains("rrunning"));
    assert!(!dump.contains("eerror"));
    assert!(!dump.contains("fetching from /v1/extensions"));
}
