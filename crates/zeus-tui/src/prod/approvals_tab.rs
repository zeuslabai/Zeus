use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

const FG: ratatui::style::Color = theme::TEXT;
const BG: ratatui::style::Color = theme::BG;
const BG2: ratatui::style::Color = theme::BG_PANEL;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    High,
    Medium,
    Low,
}

impl Risk {
    fn color(self) -> ratatui::style::Color {
        match self {
            Risk::High => theme::RED,
            Risk::Medium => theme::AMBER,
            Risk::Low => theme::YELLOW,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Risk::High => "HIGH RISK",
            Risk::Medium => "MED RISK",
            Risk::Low => "LOW RISK",
        }
    }
}

/// Live gateway data overlay for the Approvals tab (#235 de-mock).
/// When present, the tab renders live pending approvals from `GET /v1/approvals`.
pub type ApprovalsLive<'a> = Option<&'a [crate::api::ApprovalResponse]>;

pub struct ApprovalsTab<'a> {
    pub focused: usize,
    pub expanded: bool,
    pub live: ApprovalsLive<'a>,
}

struct ApprovalCard {
    id: String,
    agent: String,
    tool: String,
    args: String,
    reason: String,
    risk: Risk,
    time: String,
}

impl Default for ApprovalsTab<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalsTab<'_> {
    pub fn new() -> Self {
        Self {
            focused: 0,
            expanded: false,
            live: None,
        }
    }

    pub fn with_live<'a>(live: ApprovalsLive<'a>) -> ApprovalsTab<'a> {
        ApprovalsTab {
            focused: 0,
            expanded: false,
            live,
        }
    }

    #[allow(dead_code)]
    pub fn selected(self, focused: usize) -> Self {
        Self { focused, ..self }
    }

    #[allow(dead_code)]
    pub fn expanded(self, expanded: bool) -> Self {
        Self { expanded, ..self }
    }

    pub fn pending_count(&self) -> usize {
        self.cards().len()
    }

    fn cards(&self) -> Vec<ApprovalCard> {
        self.live
            .unwrap_or(&[])
            .iter()
            .filter(|approval| approval.is_pending())
            .map(card_from_live)
            .collect()
    }
}

impl Widget for ApprovalsTab<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        Block::default()
            .style(Style::default().bg(BG))
            .render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(0)])
            .split(area);

        let cards = self.cards();
        self.render_header(chunks[0], buf, cards.len());
        self.render_list(chunks[1], buf, &cards);
    }
}

impl ApprovalsTab<'_> {
    fn render_header(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, pending: usize) {
        Block::default()
            .style(Style::default().bg(BG2))
            .render(area, buf);
        if area.width == 0 || area.height == 0 {
            return;
        }

        for x in area.x..area.x + area.width {
            buf[(x, area.y + area.height.saturating_sub(1))]
                .set_symbol("─")
                .set_fg(theme::MUTED)
                .set_bg(BG2);
        }

        let plural = if pending == 1 { "" } else { "s" };
        let title = Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(theme::AMBER).bg(BG2)),
            Span::styled(
                pending.to_string(),
                Style::default()
                    .fg(theme::AMBER)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" pending approval{plural}"),
                Style::default().fg(FG).bg(BG2).add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(area.x + 2, area.y + 1, &title, area.width.saturating_sub(4));

        let keys = Line::from(vec![
            Span::styled(
                "a",
                Style::default()
                    .fg(theme::GREEN)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" approve  ", Style::default().fg(theme::DIM).bg(BG2)),
            Span::styled(
                "d",
                Style::default()
                    .fg(theme::RED)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" deny  ", Style::default().fg(theme::DIM).bg(BG2)),
            Span::styled(
                "v",
                Style::default()
                    .fg(theme::CYAN)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" expand  ", Style::default().fg(theme::DIM).bg(BG2)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme::DIM)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" dismiss", Style::default().fg(theme::DIM).bg(BG2)),
        ]);
        let key_width = 36.min(area.width.saturating_sub(2));
        let key_x = area.x + area.width.saturating_sub(key_width + 2);
        buf.set_line(key_x, area.y + 1, &keys, key_width);
    }

    fn render_list(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cards: &[ApprovalCard]) {
        Block::default()
            .style(Style::default().bg(BG))
            .render(area, buf);
        if cards.is_empty() {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::MUTED))
                .style(Style::default().bg(BG));
            block.render(area, buf);
            let cx = area.x + 3;
            let cy = area.y + area.height.saturating_div(2).saturating_sub(1);
            buf.set_line(
                cx,
                cy,
                &Line::from(vec![Span::styled(
                    "✓ no pending approvals",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                )]),
                area.width.saturating_sub(6),
            );
            buf.set_line(
                cx,
                cy.saturating_add(1),
                &Line::from(vec![Span::styled(
                    "Aegis queue is clear · waiting on /v1/approvals",
                    Style::default().fg(theme::DIM),
                )]),
                area.width.saturating_sub(6),
            );
            return;
        }

        let list_area = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };
        let mut y = list_area.y;
        for (idx, card) in cards.iter().enumerate() {
            let focused = idx == self.focused.min(cards.len().saturating_sub(1));
            let card_height = if focused && self.expanded { 9 } else { 7 };
            if y >= list_area.y + list_area.height {
                break;
            }
            let remaining = list_area.y + list_area.height - y;
            if remaining < 4 {
                break;
            }
            let h = card_height.min(remaining);
            let rect = Rect {
                x: list_area.x,
                y,
                width: list_area.width,
                height: h,
            };
            render_card(buf, rect, card, focused, self.expanded);
            y = y.saturating_add(h + 1);
        }
    }
}

fn render_card(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    card: &ApprovalCard,
    focused: bool,
    expanded: bool,
) {
    let risk_color = card.risk.color();
    let border = if focused { risk_color } else { theme::MUTED };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(BG2))
        .render(area, buf);

    if area.width < 8 || area.height < 3 {
        return;
    }

    for y in area.y + 1..area.y + area.height.saturating_sub(1) {
        buf[(area.x + 1, y)]
            .set_symbol("▌")
            .set_fg(risk_color)
            .set_bg(BG2);
    }

    let inner_x = area.x + 3;
    let inner_w = area.width.saturating_sub(6);
    let focus = if focused { "▸ " } else { "  " };
    let title = Line::from(vec![
        Span::styled(focus, Style::default().fg(risk_color).bg(BG2)),
        Span::styled(
            &card.agent,
            Style::default().fg(FG).bg(BG2).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" requests ", Style::default().fg(theme::DIM).bg(BG2)),
        Span::styled(
            &card.tool,
            Style::default()
                .fg(theme::CYAN)
                .bg(BG2)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            card.risk.label(),
            Style::default()
                .fg(risk_color)
                .bg(BG2)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    buf.set_line(inner_x, area.y + 1, &title, inner_w);

    let meta = Line::from(vec![
        Span::styled("id ", Style::default().fg(theme::DIM).bg(BG2)),
        Span::styled(&card.id, Style::default().fg(theme::DIM).bg(BG2)),
        Span::styled(" · ", Style::default().fg(theme::DIM).bg(BG2)),
        Span::styled(&card.time, Style::default().fg(theme::DIM).bg(BG2)),
    ]);
    buf.set_line(inner_x, area.y + 2, &meta, inner_w);

    buf.set_line(
        inner_x,
        area.y + 3,
        &Line::from(vec![
            Span::styled(
                "args ",
                Style::default()
                    .fg(theme::DIM)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate(&card.args, inner_w.saturating_sub(5) as usize),
                Style::default().fg(FG).bg(BG2),
            ),
        ]),
        inner_w,
    );

    buf.set_line(
        inner_x,
        area.y + 4,
        &Line::from(vec![Span::styled(
            "WHY BLOCKED",
            Style::default()
                .fg(theme::DIM)
                .bg(BG2)
                .add_modifier(Modifier::BOLD),
        )]),
        inner_w,
    );
    buf.set_line(
        inner_x,
        area.y + 5,
        &Line::from(vec![Span::styled(
            truncate(&card.reason, inner_w as usize),
            Style::default().fg(risk_color).bg(BG2),
        )]),
        inner_w,
    );

    if focused && area.height > 7 {
        let y = area.y + 6;
        let actions = Line::from(vec![
            Span::styled(
                " a APPROVE ",
                Style::default()
                    .fg(BG)
                    .bg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " d DENY ",
                Style::default()
                    .fg(theme::RED)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " v VIEW FULL ",
                Style::default()
                    .fg(theme::CYAN)
                    .bg(BG2)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(inner_x, y, &actions, inner_w);
    }

    if focused && expanded && area.height > 8 {
        buf.set_line(
            inner_x,
            area.y + 7,
            &Line::from(vec![
                Span::styled(
                    "full ",
                    Style::default()
                        .fg(theme::DIM)
                        .bg(BG2)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate(&card.args, inner_w.saturating_sub(6) as usize),
                    Style::default().fg(FG).bg(BG2),
                ),
            ]),
            inner_w,
        );
    }
}

fn card_from_live(approval: &crate::api::ApprovalResponse) -> ApprovalCard {
    let tool = if approval.tool_name.is_empty() {
        "unknown_tool".to_string()
    } else {
        approval.tool_name.clone()
    };
    let args = approval.args_display();
    let risk = infer_risk(&tool, &args);
    ApprovalCard {
        id: fallback(&approval.id, "approval"),
        agent: approval
            .agent_id
            .clone()
            .unwrap_or_else(|| "unknown agent".to_string()),
        tool: tool.clone(),
        args,
        reason: infer_reason(&tool, risk),
        risk,
        time: fallback(&approval.created_at, "pending"),
    }
}

fn infer_risk(tool: &str, args: &str) -> Risk {
    let haystack = format!("{} {}", tool, args).to_ascii_lowercase();
    if haystack.contains("rm -rf")
        || haystack.contains("/etc/")
        || haystack.contains("delete")
        || haystack.contains("apply_patch")
    {
        Risk::High
    } else if haystack.contains("http")
        || haystack.contains("web_fetch")
        || haystack.contains("shell")
        || haystack.contains("curl")
    {
        Risk::Medium
    } else {
        Risk::Low
    }
}

fn infer_reason(tool: &str, risk: Risk) -> String {
    match risk {
        Risk::High => format!("{tool} flagged by Aegis — destructive or protected operation"),
        Risk::Medium => format!("{tool} requires approval — external or sandbox-sensitive access"),
        Risk::Low => format!("{tool} requires confirmation"),
    }
}

fn fallback(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

fn truncate(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() && max > 1 {
        let mut trimmed: String = prefix.chars().take(max - 1).collect();
        trimmed.push('…');
        trimmed
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    fn dump(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn approvals_empty_state_is_honest() {
        let area = Rect::new(0, 0, 96, 24);
        let mut buf = Buffer::empty(area);
        ApprovalsTab::with_live(Some(&[])).render(area, &mut buf);
        let dump = dump(&buf);
        assert!(
            dump.contains("0 pending approvals"),
            "empty count missing:\n{dump}"
        );
        assert!(
            dump.contains("no pending approvals"),
            "empty state missing:\n{dump}"
        );
    }

    #[test]
    fn approvals_live_renders_gateway_data() {
        let live = vec![crate::api::ApprovalResponse {
            id: "ap_1".into(),
            tool_name: "shell".into(),
            args: serde_json::json!({"cmd": "rm -rf node_modules && npm install"}),
            agent_id: Some("zeus106".into()),
            created_at: "now".into(),
            status: serde_json::json!("pending"),
        }];
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        ApprovalsTab::with_live(Some(&live)).render(area, &mut buf);
        let dump = dump(&buf);
        assert!(
            dump.contains("1 pending approval"),
            "live count must render:\n{dump}"
        );
        assert!(
            dump.contains("shell"),
            "live tool name must render:\n{dump}"
        );
        assert!(dump.contains("zeus106"), "live agent must render:\n{dump}");
        assert!(dump.contains("HIGH RISK"), "risk badge missing:\n{dump}");
    }
}
