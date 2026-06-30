//! VectorStores — Mnemosyne collections, semantic search
//!
//! Advanced subview (id: `vectorstores`). COLLECTIONS wires live off
//! `GET /v1/vector_stores` (#185 P3-vectorstores). The list endpoint backs the
//! panel's real subject — vector stores — but its struct exposes only
//! `name / file_counts.total / status`. There is **no vector-count, embedding-
//! dim, or model field** (a file is not a vector: one file → many chunks), so
//! the original design columns vectors·dim·model are honest-dashed and the
//! COLLECTIONS row reshapes to name · files · status. The dropped columns are a
//! server-extension gap (batched for merakizzz), NOT fabrication.
//!
//! SEMANTIC SEARCH stays a const honest-stub: the search endpoint is POST-only
//! (`/v1/vector_stores/:id/search`) with no recent-query history store, so there
//! is no live feed to overlay. No live collections data → honest "fetching from
//! /v1/vector_stores…" empty state, no fabricated fallback (#284 de-mock).
//! Theme tokens, geometric glyphs, no emoji.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::VectorStoreResponse;
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// One COLLECTIONS row. Owned (not `&'static`) so live entries can carry
/// server-fetched names/counts; `files`/`status` render directly, while
/// `dim`/`model` are honest dashes (no backend).
struct Row {
    name: String,
    files: String,
    status: String,
}

/// Build the COLLECTIONS rows: live overlay when `Some` and non-empty; empty
/// otherwise (no fabricated fallback — #284 de-mock). Live `files` ←
/// `file_counts.total`; `status` ← the snake_case server string;
/// vectors/dim/model are unbacked and never shown.
fn build_rows(live: Option<&[VectorStoreResponse]>) -> Vec<Row> {
    match live {
        Some(stores) if !stores.is_empty() => stores
            .iter()
            .map(|s| Row {
                name: if s.name.is_empty() {
                    "—".to_string()
                } else {
                    s.name.clone()
                },
                files: s.file_counts.total.to_string(),
                status: if s.status.is_empty() {
                    "—".to_string()
                } else {
                    s.status.clone()
                },
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Render the `vectorstores` subview body into `area`.
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[VectorStoreResponse]>) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 3 {
        return;
    }

    let left = area.x + 2;
    let mut y = area.y + 1;
    let max_y = area.y + area.height;

    // ── COLLECTIONS ─────────────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "COLLECTIONS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    let rows = build_rows(live);

    if rows.is_empty() {
        buf.set_string_clamped(
            left,
            y,
            "No collections — fetching from /v1/vector_stores…",
            Style::default().fg(theme::DIM),
        );
        y += 1;
    }

    for r in &rows {
        if y >= max_y {
            return;
        }
        buf.set_string_clamped(left, y, "◆", Style::default().fg(theme::AMBER));
        buf.set_string_clamped(
            left + 2,
            y,
            &r.name,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(left + 20, y, &r.files, Style::default().fg(theme::TEXT));
        buf.set_string_clamped(left + 27, y, "files", Style::default().fg(theme::DIM));
        buf.set_string_clamped(left + 35, y, &r.status, Style::default().fg(theme::CYAN));
        y += 1;
    }

    y += 1;
    if y >= max_y {
        return;
    }

    // ── SEMANTIC SEARCH ─────────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "SEMANTIC SEARCH",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    if y >= max_y {
        return;
    }
    buf.set_string_clamped(
        left,
        y,
        "hybrid search · BM25 + vector embeddings",
        Style::default().fg(theme::DIM),
    );
    y += 1;
    if y >= max_y {
        return;
    }
    buf.set_string_clamped(
        left,
        y,
        "POST /v1/vector_stores/:id/search · no recent-query history",
        Style::default().fg(theme::MUTED),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str, files: usize, status: &str) -> VectorStoreResponse {
        VectorStoreResponse {
            name: name.to_string(),
            file_counts: crate::api::FileCounts { total: files },
            status: status.to_string(),
        }
    }

    #[test]
    fn live_overlays_const_roster() {
        let stores = vec![store("docs", 12, "active"), store("code", 3, "indexing")];
        let rows = build_rows(Some(&stores));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "docs");
        assert_eq!(rows[0].files, "12");
        assert_eq!(rows[0].status, "active");
        assert_eq!(rows[1].files, "3");
        assert_eq!(rows[1].status, "indexing");
    }

    #[test]
    fn none_yields_empty() {
        let rows = build_rows(None);
        assert!(rows.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn empty_live_yields_empty() {
        let rows = build_rows(Some(&[]));
        assert!(rows.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn empty_fields_render_dash() {
        let stores = vec![store("", 0, "")];
        let rows = build_rows(Some(&stores));
        assert_eq!(rows[0].name, "—");
        assert_eq!(rows[0].files, "0");
        assert_eq!(rows[0].status, "—");
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None);
    }
}
