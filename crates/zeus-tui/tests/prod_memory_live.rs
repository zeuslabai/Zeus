//! Render-diff tests for the prod Memory tab live gateway wiring.
//!
//! The Memory tab must render real `/v1/memory/files`, `/v1/sessions`, and
//! `/v1/memory/search` payloads, or honest waiting/empty states. It must not
//! fall back to prototype file trees, session rows, journal text, or facts.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use zeus_tui::api::{MemoryFileEntry, MemorySearchHit, SessionSummary};
use zeus_tui::prod::memory_tab::{render_memory_tab, MemoryLive, MemorySubTab};

fn render(sub: MemorySubTab, live: MemoryLive<'_>) -> String {
    let area = Rect::new(0, 0, 120, 30);
    let mut buf = Buffer::empty(area);
    render_memory_tab(area, &mut buf, sub, 0, live);
    buf_to_string(&buf)
}

fn buf_to_string(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn assert_no_prototype_memory(out: &str) {
    for fake in [
        "847 files",
        "147 sessions",
        "12,847 facts",
        "scratch.md",
        "s_2847",
        "TUI prototype design",
        "Mac Studio M5 Ultra",
        "Track C blockers ship in Phase 0",
        "2026-05-03.md",
        "Render-gate every prod tab",
    ] {
        assert!(!out.contains(fake), "prototype memory fixture {fake:?} rendered:
{out}");
    }
}

#[test]
fn memory_tab_counts_wait_for_real_payloads() {
    let out = render(MemorySubTab::Workspace, MemoryLive::default());

    for expected in [
        "Workspace awaiting /v1/memory/files",
        "Sessions awaiting /v1/sessions",
        "Mnemosyne awaiting /v1/memory/search",
    ] {
        assert!(out.contains(expected), "missing {expected:?}:
{out}");
    }
    assert_no_prototype_memory(&out);
}

#[test]
fn workspace_waits_for_files_without_mock_tree_or_journal() {
    let out = render(MemorySubTab::Workspace, MemoryLive::default());

    assert!(out.contains("Waiting for /v1/memory/files"), "waiting state missing:
{out}");
    assert!(out.contains("FILE PREVIEW"), "preview shell missing:
{out}");
    assert!(out.contains("waiting for /v1/memory/files"), "preview wait missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn workspace_empty_response_is_empty_not_mocked() {
    let files: Vec<MemoryFileEntry> = Vec::new();
    let live = MemoryLive { files: Some(&files), sessions: None, search: None };
    let out = render(MemorySubTab::Workspace, live);

    assert!(out.contains("Workspace 0 files"), "live zero count missing:
{out}");
    assert!(out.contains("No workspace files returned by /v1/memory/files"), "empty files state missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn workspace_renders_live_files_not_const() {
    let files = vec![
        MemoryFileEntry { path: "ZZTOPMARKER_live_file.md".into(), size: 42, modified: "2026-06-22".into() },
        MemoryFileEntry { path: "live-dir/".into(), size: 0, modified: "".into() },
    ];
    let live = MemoryLive { files: Some(&files), sessions: None, search: None };
    let out = render(MemorySubTab::Workspace, live);

    assert!(out.contains("ZZTOPMARKER_live_file.md"), "live file path must render:
{out}");
    assert!(out.contains("42 bytes · 2026-06-22"), "live metadata must render:
{out}");
    assert!(out.contains("# Live memory file"), "live preview shell must render:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn sessions_waits_for_endpoint_without_mock_rows() {
    let out = render(MemorySubTab::Sessions, MemoryLive::default());

    assert!(out.contains("Waiting for /v1/sessions"), "waiting sessions state missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn sessions_empty_response_is_empty_not_mocked() {
    let sessions: Vec<SessionSummary> = Vec::new();
    let live = MemoryLive { files: None, sessions: Some(&sessions), search: None };
    let out = render(MemorySubTab::Sessions, live);

    assert!(out.contains("Sessions 0 sessions"), "live zero sessions count missing:
{out}");
    assert!(out.contains("No sessions returned by /v1/sessions"), "empty sessions state missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn sessions_renders_live_rows_not_const() {
    let sessions = vec![SessionSummary {
        id: "abcd1234ef".into(),
        created: "2026-06-22".into(),
        message_count: 7,
        est_tokens: 1234,
        last_preview: "ZZSESSIONMARKER preview text".into(),
    }];
    let live = MemoryLive { files: None, sessions: Some(&sessions), search: None };
    let out = render(MemorySubTab::Sessions, live);

    assert!(out.contains("abcd1234"), "live session id must render:
{out}");
    assert!(out.contains("ZZSESSIONMARKER"), "live session preview must render:
{out}");
    assert!(out.contains("~1234 tok · 7 msgs"), "live session stats must render:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn mnemosyne_waits_for_search_without_mock_facts() {
    let out = render(MemorySubTab::Mnemosyne, MemoryLive::default());

    assert!(out.contains("RECENT FACTS · awaiting"), "awaiting search title missing:
{out}");
    assert!(out.contains("Waiting for /v1/memory/search"), "waiting search state missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn mnemosyne_empty_response_is_empty_not_mocked() {
    let hits: Vec<MemorySearchHit> = Vec::new();
    let live = MemoryLive { files: None, sessions: None, search: Some(&hits) };
    let out = render(MemorySubTab::Mnemosyne, live);

    assert!(out.contains("Mnemosyne 0 facts"), "live zero hit count missing:
{out}");
    assert!(out.contains("RECENT FACTS · 0"), "zero facts title missing:
{out}");
    assert!(out.contains("No memory hits returned by /v1/memory/search"), "empty hits state missing:
{out}");
    assert_no_prototype_memory(&out);
}

#[test]
fn mnemosyne_renders_live_hits_not_const() {
    let hits = vec![MemorySearchHit {
        content: "ZZSEARCHMARKER hybrid hit content".into(),
        score: 0.91,
        id: Some("m-1".into()),
        session_id: Some("s-1".into()),
        memory_type: Some("semantic".into()),
        importance: Some(0.8),
        path: None,
    }];
    let live = MemoryLive { files: None, sessions: None, search: Some(&hits) };
    let out = render(MemorySubTab::Mnemosyne, live);

    assert!(out.contains("ZZSEARCHMARKER"), "live search hit content must render:
{out}");
    assert!(out.contains("0.91 · semantic · live"), "live search metadata must render:
{out}");
    assert_no_prototype_memory(&out);
}
