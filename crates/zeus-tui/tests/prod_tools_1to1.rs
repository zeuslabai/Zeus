//! Render-fidelity guard for the Production TUI Tools tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`ToolsTab`, JSX 816–947).
//! This phase keeps the tab wired to the live `/v1/tools` registry when present
//! while guarding the prototype structure: categories rail, searchable registry,
//! selected tool details, schema block, execute controls, and sandbox badges.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use zeus_tui::prod::tools_tab::{ToolEntry, ToolsTab};

fn render_tools(widget: ToolsTab<'_>) -> (Buffer, String) {
    let backend = TestBackend::new(120, 34);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            widget.render(f.area(), f.buffer_mut());
        })
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

static LIVE_TOOLS: &[ToolEntry] = &[
    ToolEntry {
        name: "shell",
        category: "shell",
        desc: "Execute shell command (sandboxed)",
        danger: true,
        schema: r#"{"command":"string","cwd?":"string"}"#,
    },
    ToolEntry {
        name: "read_file",
        category: "files",
        desc: "Read file contents",
        danger: false,
        schema: r#"{"path":"string"}"#,
    },
    ToolEntry {
        name: "web_search",
        category: "core",
        desc: "Search the web",
        danger: false,
        schema: r#"{"query":"string"}"#,
    },
];

fn live_widget<'a>(selected_tool: &'a str, filter: &'a str) -> ToolsTab<'a> {
    ToolsTab {
        selected_category: Some("shell"),
        selected_tool,
        tool_filter: filter,
        scroll_offset: 0,
        tools: Some(LIVE_TOOLS),
    }
}

#[test]
fn tools_tab_matches_prototype_structure_with_live_registry() {
    let (_buf, dump) = render_tools(live_widget("shell", ""));
    std::fs::create_dir_all("/Users/mike/.zeus/workspace/tools_phase").ok();
    std::fs::write("/Users/mike/.zeus/workspace/tools_phase/prod_tools_1to1_dump.log", &dump)
        .expect("write render dump");

    for expected in [
        "CATEGORIES",
        "3 tools",
        "Shell",
        "Search tools…",
        "shell",
        "Execute shell command (sandboxed)",
        "● SANDBOXED",
        "SANDBOXED",
        "SCHEMA",
        "EXECUTE",
        "VALIDATE",
        "last run · 14:32 · ✓ 24l",
        "live /v1/tools",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }
}

#[test]
fn tools_tab_filter_renders_only_matching_tools() {
    let (_buf, dump) = render_tools(live_widget("read_file", "read"));

    assert!(dump.contains("read_file"), "filtered tool missing:\n{dump}");
    assert!(dump.contains("Read file contents"), "filtered desc missing:\n{dump}");
    assert!(!dump.contains("web_search"), "non-matching tool leaked:\n{dump}");
}
