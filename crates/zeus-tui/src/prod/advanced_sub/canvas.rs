//! Canvas — Visual plan / workflow builder.
//!
//! Advanced subview (id: `canvas`). No specific JSX `AdvancedSubview` block
//! exists for this id, so this is a clean representative panel consistent with
//! the agents/skills/projects siblings: a summary line + NEW FLOW button, then
//! a vertical node graph — each step is a status-colored node (`◆`) with a
//! connector (`│`) to the next, showing the workflow stage + assigned agent.
//! Theme tokens, geometric glyphs, no emoji, opaque-bg inherited. No live
//! data → honest "awaiting" line, no fabricated graph (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::WorkflowResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// A workflow node. Owned so live `/v1/workflows` entries can carry their own
/// strings; the const placeholder uses `'static` strs promoted into `String`.
struct Node {
    stage: String,
    agent: String,
    state: NodeState,
}

#[derive(Clone, Copy)]
enum NodeState {
    Done,
    Running,
    Pending,
}

impl NodeState {
    /// Map a `/v1/workflows` status string onto a node state (honest: anything
    /// not clearly done/running is Pending).
    fn from_status(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "completed" | "complete" | "done" | "succeeded" | "success" => NodeState::Done,
            "running" | "in_progress" | "active" | "executing" => NodeState::Running,
            _ => NodeState::Pending,
        }
    }
    fn color(self) -> Color {
        match self {
            NodeState::Done => theme::GREEN,
            NodeState::Running => theme::ACCENT,
            NodeState::Pending => theme::DIM,
        }
    }
    fn label(self) -> &'static str {
        match self {
            NodeState::Done => "DONE",
            NodeState::Running => "RUNNING",
            NodeState::Pending => "PENDING",
        }
    }
}

/// Build the node graph: overlay live `/v1/workflows` instances when present
/// (each workflow = one node), else empty.
///
/// Honest mapping — `WorkflowResponse` exposes no per-node agent, so the agent
/// column shows the live node-progress (`completed/total`) instead of a
/// fabricated agent name; `status` drives the state, `message`/`workflow_id`
/// the stage label.
fn build_nodes(live: Option<&[WorkflowResponse]>) -> Vec<Node> {
    match live {
        Some(ws) if !ws.is_empty() => ws
            .iter()
            .map(|w| {
                let stage = if !w.message.is_empty() {
                    w.message.clone()
                } else if !w.workflow_id.is_empty() {
                    w.workflow_id.clone()
                } else {
                    "workflow".to_string()
                };
                // Agent column carries honest node-progress, not a fake name.
                let agent = format!("{}/{} nodes", w.completed_nodes, w.total_nodes);
                Node { stage, agent, state: NodeState::from_status(&w.status) }
            })
            .collect(),
        // Pre-fetch or no workflows → no fabricated graph (#284 de-mock).
        _ => Vec::new(),
    }
}

/// Render the `canvas` subview body into `area`.
///
/// `live` overlays real `/v1/workflows` instances onto the node graph when
/// present; `None` (pre-fetch) or empty shows an honest "awaiting" line — no
/// fabricated graph (#284 de-mock).
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[WorkflowResponse]>) {
    Clear.render(area, buf);
    if area.width < 4 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());

    let nodes = build_nodes(live);

    // Summary line: "research-flow · <N> nodes" (N = live workflow count).
    let mut x = area.x + 2;
    x = set_str(x, area.y + 1, "research-flow", Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
    x = set_str(x, area.y + 1, "   ", Style::default().fg(theme::TEXT), right, buf);
    x = set_str(x, area.y + 1, &format!("{}", nodes.len()), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), right, buf);
    let _ = set_str(x, area.y + 1, " nodes", Style::default().fg(theme::TEXT), right, buf);
    // NEW FLOW button (right-aligned, accent).
    let btn = " NEW FLOW ";
    let bx = right.saturating_sub(btn.len() as u16 + 1);
    paint_button(bx, area.y + 1, btn, theme::ACCENT, theme::BG, buf);

    // Node graph (vertical). Each node = 1 row, connector = 1 row between.
    let mut y = area.y + 3;
    let node_x = area.x + 3;
    if nodes.is_empty() {
        // Honest empty state — no fabricated workflow graph.
        set_str(area.x + 2, y, "No workflows — fetching from /v1/workflows…", Style::default().fg(theme::DIM), right, buf);
        return;
    }
    for (i, n) in nodes.iter().enumerate() {
        if y >= area.bottom() {
            break;
        }
        let col = n.state.color();
        // Node glyph.
        let mut cx = set_str(node_x, y, "◆ ", Style::default().fg(col).add_modifier(Modifier::BOLD), right, buf);
        // Stage name.
        cx = set_str(cx, y, &n.stage, Style::default().fg(theme::TEXT), right, buf);
        cx = set_str(cx, y, "  ", Style::default().fg(theme::TEXT), right, buf);
        // Agent.
        cx = set_str(cx, y, "▸ ", Style::default().fg(theme::DIM), right, buf);
        cx = set_str(cx, y, &n.agent, Style::default().fg(theme::CYAN), right, buf);
        let _ = cx;
        // State label (right-aligned).
        let lbl = n.state.label();
        let lx = right.saturating_sub(lbl.len() as u16 + 2);
        let _ = set_str(lx, y, lbl, Style::default().fg(col).add_modifier(Modifier::BOLD), right, buf);

        // Connector to next node (skip after last).
        if i + 1 < nodes.len() && y + 1 < area.bottom() {
            let _ = set_str(node_x, y + 1, "│", Style::default().fg(theme::DIM), right, buf);
        }
        y += 2;
    }
}

/// Paint a solid filled button (bg=fill, fg=text).
fn paint_button(x: u16, y: u16, label: &str, fill: Color, text: Color, buf: &mut Buffer) {
    for (i, ch) in label.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= buf.area.right() {
            break;
        }
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_fg(text).set_bg(fill).set_style(Style::default().add_modifier(Modifier::BOLD).bg(fill).fg(text)); }
    }
}

/// Write `s` at (x,y), clipped to `max_x`. Returns x after the last cell.
fn set_str(x: u16, y: u16, s: &str, style: Style, max_x: u16, buf: &mut Buffer) -> u16 {
    let mut cx = x;
    for ch in s.chars() {
        if cx >= max_x || cx >= buf.area.right() {
            break;
        }
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_style(style); }
        cx += 1;
    }
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(id: &str, status: &str, completed: u32, total: u32) -> WorkflowResponse {
        WorkflowResponse {
            workflow_id: id.to_string(),
            status: status.to_string(),
            completed_nodes: completed,
            total_nodes: total,
            ..Default::default()
        }
    }

    #[test]
    fn none_yields_no_fabricated_nodes() {
        // Pre-fetch (None) → zero nodes: no invented graph.
        assert!(build_nodes(None).is_empty());
    }

    #[test]
    fn empty_live_yields_no_fabricated_nodes() {
        // Empty workflows → zero nodes: no invented graph.
        assert!(build_nodes(Some(&[])).is_empty());
    }

    #[test]
    fn live_overlays_workflows() {
        let live = vec![
            wf("ingest-flow", "completed", 3, 3),
            wf("embed-flow", "running", 1, 4),
            wf("publish-flow", "queued", 0, 2),
        ];
        let n = build_nodes(Some(&live));
        assert_eq!(n.len(), 3, "one node per live workflow");
        // workflow_id drives the stage label (no `message` set).
        assert_eq!(n[0].stage, "ingest-flow");
        // honest agent column = live node-progress, not a fabricated name.
        assert_eq!(n[1].agent, "1/4 nodes");
        // status → state mapping.
        assert!(matches!(n[0].state, NodeState::Done));
        assert!(matches!(n[1].state, NodeState::Running));
        assert!(matches!(n[2].state, NodeState::Pending));
    }

    #[test]
    fn message_preferred_over_id_for_stage() {
        let mut w = wf("wf-123", "running", 0, 1);
        w.message = "Synthesize report".to_string();
        let n = build_nodes(Some(std::slice::from_ref(&w)));
        assert_eq!(n[0].stage, "Synthesize report");
    }

    #[test]
    fn render_no_panic() {
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None);
        let live = vec![wf("flow", "running", 1, 2)];
        render(area, &mut buf, Some(&live));
        render(Rect::new(0, 0, 3, 1), &mut buf, None);
        render(Rect::new(0, 0, 20, 4), &mut buf, None);
    }
}
