//! Knowledge Graph — Memory graph, communities
//!
//! Advanced subview (id: `knowledge-graph`). COMMUNITY · NODES wire live off
//! `GET /v1/memory/communities` (#185 P-knowledge-graph). The list endpoint
//! backs the panel's real subject — KG communities — exposing `name`
//! (→ COMMUNITY) and `entity_count` (→ NODES). There is **no per-community
//! edge count**: `Community` carries only `id/name/description/entity_count`,
//! and edges exist solely as a global relationship summary (`/v1/memory/graph/
//! edges`), never sliced by community. So the EDGES column is honest-dashed
//! (server-extension gap, batched for merakizzz), NOT fabricated. The summary
//! line's `facts` count likewise has no backing field and is dashed; community
//! and node totals are derived from the live rows.
//!
//! The original stub premise ("no `/v1/knowledge*` backend") was drifted — the
//! backend exists under `/v1/memory/graph/*`. This is the vectorstores pattern:
//! same subject, real backend, one unbacked column → honest-dash, not stub.
//! No live data → honest "fetching from /v1/memory/communities…" empty state,
//! no fabricated fallback (#284 de-mock). Theme tokens, geometric glyphs, no emoji.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::CommunityResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// Accent-bar color cycle for community rows (live rows have no server color,
/// so we cycle the theme palette deterministically by row index).
const ACCENTS: &[Color] = &[
    theme::BLUE,
    theme::GREEN,
    theme::CYAN,
    theme::PURPLE,
    theme::AMBER,
];

/// One COMMUNITY row. Owned (not `&'static`) so live entries carry
/// server-fetched names/node-counts; `edges` is always an honest dash (no
/// per-community edge backend).
struct Row {
    name: String,
    nodes: String,
    edges: String,
    color: Color,
}

/// Build the COMMUNITY rows: live overlay when `Some` and non-empty; empty
/// otherwise (no fabricated fallback — #284 de-mock). Live `nodes` ←
/// `entity_count`; `edges` is always dashed (no per-community edge field —
/// server-extension gap).
fn build_rows(live: Option<&[CommunityResponse]>) -> Vec<Row> {
    match live {
        Some(comms) if !comms.is_empty() => comms
            .iter()
            .enumerate()
            .map(|(i, c)| Row {
                name: if c.name.is_empty() {
                    "—".to_string()
                } else {
                    c.name.clone()
                },
                nodes: c.entity_count.to_string(),
                edges: "—".to_string(),
                color: ACCENTS[i % ACCENTS.len()],
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Render the `knowledge-graph` subview body into `area`.
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[CommunityResponse]>) {
    Clear.render(area, buf);
    if area.width < 20 || area.height < 6 {
        return;
    }

    let mut y = area.y;

    // Header row
    let header = "COMMUNITY            NODES     EDGES";
    buf.set_string_clamped(
        area.x + 1,
        y,
        header,
        Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
    );
    y += 1;

    // Separator
    let sep_width = (area.width as usize).saturating_sub(2).min(42);
    let sep = "─".repeat(sep_width);
    buf.set_string_clamped(area.x + 1, y, &sep, Style::default().fg(theme::DARK));
    y += 1;

    let rows = build_rows(live);

    // Honest empty state — no fabricated community roster (#284 de-mock).
    if rows.is_empty() {
        buf.set_string_clamped(
            area.x + 1,
            y,
            "No communities — fetching from /v1/memory/communities…",
            Style::default().fg(theme::DIM),
        );
    }

    // Rows
    for r in &rows {
        if y >= area.y + area.height {
            break;
        }
        let line = format!("{:20} {:9} {}", r.name, r.nodes, r.edges);
        buf.set_string_clamped(area.x + 1, y, &line, Style::default().fg(theme::TEXT));
        // Left accent bar
        if let Some(c) = cell_mut_clamped(buf, area.x, y) { c.set_style(Style::default().fg(r.color)); }
        y += 1;
    }

    // Summary — community + node totals are live-derived; edges/facts have no
    // backing field and are honest-dashed.
    if y + 1 < area.y + area.height {
        y += 1;
        let summary = match live {
            Some(comms) if !comms.is_empty() => {
                let nodes: u32 = comms.iter().map(|c| c.entity_count).sum();
                format!(
                    "{} communities · {} nodes · — edges · — facts",
                    comms.len(),
                    nodes
                )
            }
            _ => "— communities · — nodes · — edges · — facts".to_string(),
        };
        buf.set_string_clamped(area.x + 1, y, &summary, Style::default().fg(theme::DIM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comm(name: &str, entity_count: u32) -> CommunityResponse {
        CommunityResponse {
            name: name.to_string(),
            entity_count,
        }
    }

    #[test]
    fn live_overlays_communities() {
        let comms = vec![comm("Architecture", 847), comm("Security", 412)];
        let rows = build_rows(Some(&comms));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Architecture");
        assert_eq!(rows[0].nodes, "847");
        // edges always honest-dashed — no per-community edge backend
        assert_eq!(rows[0].edges, "—");
        assert_eq!(rows[1].nodes, "412");
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
    fn empty_name_renders_dash() {
        let comms = vec![comm("", 0)];
        let rows = build_rows(Some(&comms));
        assert_eq!(rows[0].name, "—");
        assert_eq!(rows[0].nodes, "0");
        assert_eq!(rows[0].edges, "—");
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None);
    }
}
