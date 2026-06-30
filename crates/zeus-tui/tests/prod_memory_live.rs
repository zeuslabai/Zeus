//! C2b render-diff tests for the prod Memory tab — proves the de-mock:
//! a populated `MemoryLive` renders REAL gateway data, not the static consts
//! (`WORKSPACE_FILES` / `SESSIONS` / `SEARCH_RESULTS`), and that an empty
//! `MemoryLive` still falls back to those consts.
//!
//! Mirrors the Wave-1 de-mock cut on `feat/wire-memory-settings-tabs` (C2b):
//! the 3 sub-tabs (Workspace/Sessions/Mnemosyne) each borrow their live slice
//! from `App` (`prod_memory_files` / `prod_sessions` / `prod_memory_search`),
//! landed by the lib.rs `run()` poll-workers. These tests render the fn
//! directly with hand-built live data so they assert the overlay path without
//! a live gateway.
//!
//! Separate file = conflict-free with the other agents' tests.

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

fn maybe_dump(label: &str, dump: &str) {
    if std::env::var_os("ZEUS_DUMP_RENDER").is_some() {
        println!("--- {label} ---\n{dump}");
    }
}

fn buf_to_string(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

// ── Workspace ────────────────────────────────────────────────────────────────

#[test]
fn workspace_renders_live_files_not_const() {
    let files = vec![
        MemoryFileEntry {
            path: "ZZTOPMARKER_live_file.md".into(),
            size: 42,
            modified: "now".into(),
        },
        MemoryFileEntry {
            path: "subdir/".into(),
            size: 0,
            modified: "now".into(),
        },
    ];
    let live = MemoryLive {
        files: Some(&files),
        sessions: None,
        search: None,
    };
    let out = render(MemorySubTab::Workspace, live);

    // Live path is present...
    assert!(
        out.contains("ZZTOPMARKER_live_file.md"),
        "live file path must render"
    );
    // ...and the const tree is NOT (its known leaf node is gone).
    assert!(
        !out.contains("MEMORY.md"),
        "const WORKSPACE_FILES must not render when live present"
    );
}

#[test]
fn workspace_falls_back_to_const_when_empty() {
    let out = render(MemorySubTab::Workspace, MemoryLive::default());
    // A const file tree node renders when no live data has landed.
    assert!(
        out.contains("scratch.md"),
        "const fallback must render when live absent"
    );
}

#[test]
fn memory_workspace_matches_production_prototype_shell() {
    let out = render(MemorySubTab::Workspace, MemoryLive::default());
    maybe_dump("memory/workspace", &out);

    for expected in [
        "Workspace 847 files",
        "Sessions 147 sessions",
        "Mnemosyne 12,847 facts",
        "~/.zeus/workspace/",
        "SOUL.md",
        "│ ├ 2026-05-03.md",
        "JOURNAL",
        "2026-05-03.md",
        "last modified · 2 minutes ago",
        "# Journal · 2026-05-03",
        "## Sessions",
        "## Decisions",
        "Render-gate every prod tab",
    ] {
        assert!(out.contains(expected), "missing {expected:?}:\n{out}");
    }
}

// ── Sessions ─────────────────────────────────────────────────────────────────

#[test]
fn sessions_renders_live_rows_not_const() {
    let sessions = vec![SessionSummary {
        id: "abcd1234ef".into(),
        created: "2026-06-22".into(),
        message_count: 7,
        est_tokens: 1234,
        last_preview: "ZZSESSIONMARKER preview text".into(),
    }];
    let live = MemoryLive {
        files: None,
        sessions: Some(&sessions),
        search: None,
    };
    let out = render(MemorySubTab::Sessions, live);

    assert!(
        out.contains("ZZSESSIONMARKER"),
        "live session preview must render"
    );
    assert!(
        out.contains("7 msgs"),
        "live message_count must render in stats"
    );
}

#[test]
fn memory_sessions_matches_production_prototype_rows() {
    let out = render(MemorySubTab::Sessions, MemoryLive::default());
    maybe_dump("memory/sessions", &out);

    for expected in [
        "Workspace 847 files",
        "Sessions 147 sessions",
        "Mnemosyne 12,847 facts",
        "s_2847",
        "14:30",
        "TUI prototype design",
        "12m · 47 tools · 23 msgs",
        "s_2842",
        "Pitch deck v5",
    ] {
        assert!(out.contains(expected), "missing {expected:?}:\n{out}");
    }
}

#[test]
fn memory_mnemosyne_matches_production_prototype_cards() {
    let out = render(MemorySubTab::Mnemosyne, MemoryLive::default());
    maybe_dump("memory/mnemosyne", &out);

    for expected in [
        "Workspace 847 files",
        "Sessions 147 sessions",
        "Mnemosyne 12,847 facts",
        "/  hybrid search · BM25 + vector embeddings",
        "● ollama embedded",
        "RECENT FACTS · 12,847 indexed",
        "0.94 · session 2847 · 8m ago",
        "Track C blockers ship in Phase 0",
        "Mac Studio M5 Ultra",
    ] {
        assert!(out.contains(expected), "missing {expected:?}:\n{out}");
    }
}

// ── Mnemosyne / search ───────────────────────────────────────────────────────

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
    let live = MemoryLive {
        files: None,
        sessions: None,
        search: Some(&hits),
    };
    let out = render(MemorySubTab::Mnemosyne, live);

    assert!(
        out.contains("ZZSEARCHMARKER"),
        "live search hit content must render"
    );
}

#[test]
fn mnemosyne_falls_back_to_const_when_empty() {
    let out = render(MemorySubTab::Mnemosyne, MemoryLive::default());
    // SEARCH_RESULTS const carries this distinctive token.
    assert!(
        out.contains("Mac Studio") || out.contains("inference"),
        "const SEARCH_RESULTS must render when live absent"
    );
}
