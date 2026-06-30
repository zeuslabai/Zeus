//! NodeComms — Inter-agent fleet messaging
//!
//! Advanced subview (id: `nodecomms`). No dedicated JSX block exists, so this
//! is a clean representative panel consistent with the other subviews: a
//! FLEET LINKS status section over a RECENT MESSAGES feed. Theme tokens,
//! geometric glyphs, no emoji.
//!
//! FLEET LINKS wires live off `GET /v1/nodes` (`NodeResponse`): peer←node_id,
//! up=true (registry presence == connected), rtt←`rtt_ms` (#249 — keepalive
//! ping→pong delta, `—` until first pong). `transport` remains unbacked by the
//! node registry (no such field) → rendered `—`; server-extension gap. No live
//! data → honest "fetching from /v1/nodes…" empty state, no fabricated fallback
//! (#284 de-mock).
//! RECENT MESSAGES: no inter-node message-feed endpoint exists (`/v1/nodes` and
//! `/v1/fleet` are registries, not feeds) → honest empty state, no fabricated
//! message list (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::NodeResponse;
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// An owned fleet-link row — built from live `/v1/nodes` data. `rtt` is backed
/// by `rtt_ms` (#249, `—` until first pong); `transport` is still unbacked →
/// live rows render `—`. No live data → empty (no fabricated fallback, #284).`—`.
struct Link {
    peer: String,
    transport: String,
    rtt: String,
    up: bool,
}

/// Build the FLEET LINKS rows. Live overlay when `Some` and non-empty
/// (peer←node_id, up=true; transport/rtt honestly `—`); empty otherwise
/// (no fabricated fallback — #284 de-mock).
fn build_links(live: Option<&[NodeResponse]>) -> Vec<Link> {
    match live {
        Some(ns) if !ns.is_empty() => ns
            .iter()
            .map(|n| Link {
                peer: n.node_id.clone(),
                transport: "—".to_string(),
                // #249: rtt_ms from the keepalive ping→pong delta. 0 = no pong
                // measured yet → honest `—`; otherwise render `{ms}ms`.
                rtt: if n.rtt_ms == 0 {
                    "—".to_string()
                } else {
                    format!("{}ms", n.rtt_ms)
                },
                up: true,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Render the `nodecomms` subview body into `area`.
///
/// `live` carries `/v1/nodes` when fetched; FLEET LINKS overlays it, RECENT
/// MESSAGES shows an honest empty state (no message-feed backend, #284 de-mock).
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[NodeResponse]>) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 3 {
        return;
    }

    let left = area.x + 2;
    let mut y = area.y + 1;
    let max_y = area.y + area.height;

    // ── FLEET LINKS ─────────────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "FLEET LINKS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    let links = build_links(live);

    if links.is_empty() {
        buf.set_string_clamped(
            left,
            y,
            "No peers — fetching from /v1/nodes…",
            Style::default().fg(theme::DIM),
        );
        y += 1;
    }

    for l in &links {
        if y >= max_y {
            return;
        }
        let (dot, dot_color) = if l.up {
            ("●", theme::GREEN)
        } else {
            ("○", theme::DIM)
        };
        buf.set_string_clamped(left, y, dot, Style::default().fg(dot_color));
        buf.set_string_clamped(
            left + 2,
            y,
            &l.peer,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(left + 16, y, &l.transport, Style::default().fg(theme::DIM));
        buf.set_string_clamped(left + 30, y, &l.rtt, Style::default().fg(theme::MUTED));
        y += 1;
    }

    y += 1;
    if y >= max_y {
        return;
    }

    // ── RECENT MESSAGES ─────────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "RECENT MESSAGES",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    // No inter-node message-feed endpoint exists — honest empty state, no
    // fabricated message list (#284 de-mock).
    buf.set_string_clamped(
        left,
        y,
        "No message feed — awaiting /v1/nodes message endpoint",
        Style::default().fg(theme::DIM),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, rtt_ms: u64) -> NodeResponse {
        NodeResponse {
            node_id: id.to_string(),
            host: "h".to_string(),
            connected_at: "now".to_string(),
            capabilities: vec![],
            rtt_ms,
        }
    }

    #[test]
    fn none_yields_empty_links() {
        let links = build_links(None);
        assert!(links.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn empty_live_yields_empty_links() {
        let links = build_links(Some(&[]));
        assert!(links.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn live_overlays_const_links() {
        let live = vec![node("zeus106", 18), node("zeus100", 0)];
        let links = build_links(Some(&live));
        assert_eq!(links.len(), 2);
        // peer←node_id is honestly backed.
        assert_eq!(links[0].peer, "zeus106");
        assert_eq!(links[1].peer, "zeus100");
        // registry presence == connected.
        assert!(links[0].up);
        // transport/rtt unbacked by the node registry → honestly `—`, never fabricated.
        assert_eq!(links[0].transport, "—");
        // #249: rtt_ms now flows live — nonzero renders `{ms}ms`,
        // zero (no pong yet) honestly renders `—`, never fabricated.
        assert_eq!(links[0].rtt, "18ms");
        assert_eq!(links[1].rtt, "—");
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None);
    }
}
