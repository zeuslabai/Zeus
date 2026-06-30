//! Economy — Agora wallet, marketplace, x402
//!
//! Advanced subview (id: `economy`). Mirrors the JSX `AdvancedSubview`
//! `economy` branch (docs/zeus-tui-production.jsx line 1518): an AGORA WALLET
//! summary card followed by a RECENT TRANSACTIONS list.
//!
//! Wired live (#185, merakizzz "wire all" read-side):
//! - **AGORA WALLET card** ← `GET /v1/economy/wallets`. The fleet ledger is the
//!   real subject; the card shows the aggregate balance across all agent
//!   wallets in integer credits + the earned/spent split. The old
//!   "$ 247.83 USDC" mock had no backend (credits are an integer ledger
//!   balance, not a USDC float) → dropped, not faked.
//! - **RECENT TRANSACTIONS** ← `GET /v1/economy/transactions`. Each row shows
//!   time (created_at HH:MM), the counterparty agent, the reason, and the
//!   amount in credits, colored by kind (earn/mint = green, spend/burn = red,
//!   transfer/fee = amber).
//!
//! #190 human→titan token-SEND stays future-scope; this panel is read-only.
//! When no fetch has landed yet, an honest "awaiting" line paints — no
//! fabricated balance or transactions (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::{EconomyTxResponse, EconomyWalletResponse};
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// A single recent-transaction row. Owned so live rows (built from
/// `EconomyTxResponse`) share one render path.
struct Row {
    t: String,
    agent: String,
    action: String,
    item: String,
    amt: String,
    color: ratatui::style::Color,
}

/// Color a transaction amount by its kind label: earn/mint = green (inflow),
/// spend/burn = red (outflow), transfer/fee/other = amber.
fn kind_color(kind: &str) -> ratatui::style::Color {
    match kind {
        "earn" | "mint" => theme::GREEN,
        "spend" | "burn" => theme::RED,
        _ => theme::AMBER,
    }
}

/// Build live transaction rows from `/v1/economy/transactions`. Empty input →
/// `None` so the caller falls back to the const rows.
fn live_rows(txs: Option<&[EconomyTxResponse]>) -> Option<Vec<Row>> {
    let txs = txs?;
    if txs.is_empty() {
        return None;
    }
    Some(
        txs.iter()
            .map(|tx| {
                let kind = tx.kind_label();
                let reason = tx.reason_label();
                // Counterparty: prefer from_agent (the actor), else to_agent.
                let agent = tx
                    .from_agent
                    .clone()
                    .or_else(|| tx.to_agent.clone())
                    .unwrap_or_else(|| "system".to_string());
                // Sign the amount by kind: inflow (earn/mint) positive.
                let sign = if matches!(kind.as_str(), "earn" | "mint") { "+" } else { "-" };
                Row {
                    t: String::new(), // ledger txs have no display time column here
                    agent,
                    action: kind.clone(),
                    item: reason.replace('_', " "),
                    amt: format!("{sign}{} cr", tx.amount),
                    color: kind_color(&kind),
                }
            })
            .collect(),
    )
}

/// Render the `economy` subview body into `area`.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    wallets: Option<&[EconomyWalletResponse]>,
    txs: Option<&[EconomyTxResponse]>,
) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 3 {
        return;
    }

    let left = area.x + 2;
    let mut y = area.y + 1;
    let max_y = area.y + area.height;

    // ── AGORA WALLET card ───────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "AGORA WALLET",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    if y >= max_y {
        return;
    }

    // Live: aggregate the fleet ledger. Fallback: the const mock balance.
    let (balance_line, unit, sub_line) = match wallets {
        Some(ws) if !ws.is_empty() => {
            let total: u64 = ws.iter().map(|w| w.balance).sum();
            let earned: u64 = ws.iter().map(|w| w.total_earned).sum();
            let spent: u64 = ws.iter().map(|w| w.total_spent).sum();
            (
                format!("{total}"),
                "credits",
                format!(
                    "{} wallets · {earned} earned · {spent} spent (fleet ledger)",
                    ws.len()
                ),
            )
        }
        _ => (
            "—".to_string(),
            "credits",
            "fleet ledger · awaiting /v1/economy/wallets".to_string(),
        ),
    };

    buf.set_string_clamped(
        left,
        y,
        &balance_line,
        Style::default()
            .fg(theme::TEXT_BRIGHT)
            .add_modifier(Modifier::BOLD),
    );
    buf.set_string_clamped(
        left + balance_line.len() as u16 + 1,
        y,
        unit,
        Style::default().fg(theme::DIM),
    );
    y += 1;
    if y >= max_y {
        return;
    }
    buf.set_string_clamped(left, y, &sub_line, Style::default().fg(theme::GREEN));
    y += 2;
    if y >= max_y {
        return;
    }

    // ── RECENT TRANSACTIONS ─────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "RECENT TRANSACTIONS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    let rows = live_rows(txs);

    if rows.is_none() {
        // Honest empty state — no fabricated transactions.
        buf.set_string_clamped(
            left,
            y,
            "awaiting /v1/economy/transactions…",
            Style::default().fg(theme::DIM),
        );
        return;
    }
    let rows = rows.unwrap();

    for row in &rows {
        if y >= max_y {
            break;
        }
        // left accent marker (mirrors the JSX borderLeft per-tx color)
        buf.set_string_clamped(left, y, "│", Style::default().fg(row.color));
        let mut x = left + 2;
        if !row.t.is_empty() {
            buf.set_string_clamped(x, y, &row.t, Style::default().fg(theme::MUTED));
        }
        x += 7;
        buf.set_string_clamped(
            x,
            y,
            &row.agent,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        x += 12;
        buf.set_string_clamped(x, y, &row.action, Style::default().fg(theme::DIM));
        x += 11;
        buf.set_string_clamped(x, y, &row.item, Style::default().fg(theme::TEXT));
        // amount, right-aligned within area
        let amt_x = area.x + area.width.saturating_sub(row.amt.len() as u16 + 2);
        if amt_x > x + row.item.len() as u16 {
            buf.set_string_clamped(
                amt_x,
                y,
                &row.amt,
                Style::default().fg(row.color).add_modifier(Modifier::BOLD),
            );
        }
        y += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(kind: serde_json::Value, reason: &str, from: Option<&str>, amount: u64) -> EconomyTxResponse {
        EconomyTxResponse {
            kind,
            reason: serde_json::Value::String(reason.to_string()),
            from_agent: from.map(|s| s.to_string()),
            to_agent: None,
            amount,
        }
    }

    fn wallet(id: &str, bal: u64, earned: u64, spent: u64) -> EconomyWalletResponse {
        EconomyWalletResponse {
            agent_id: id.to_string(),
            balance: bal,
            total_earned: earned,
            total_spent: spent,
        }
    }

    #[test]
    fn live_txs_overlay_const() {
        let txs = vec![tx(serde_json::json!("earn"), "task_completion", Some("hermes"), 42)];
        let rows = live_rows(Some(&txs)).expect("live rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent, "hermes");
        assert_eq!(rows[0].action, "earn");
        assert_eq!(rows[0].item, "task completion"); // underscore→space
        assert_eq!(rows[0].amt, "+42 cr"); // earn → positive sign
    }

    #[test]
    fn spend_kind_is_negative_and_red() {
        let txs = vec![tx(serde_json::json!("spend"), "llm_call", Some("hermes"), 4)];
        let rows = live_rows(Some(&txs)).unwrap();
        assert_eq!(rows[0].amt, "-4 cr");
        assert_eq!(rows[0].color, theme::RED);
    }

    #[test]
    fn unknown_kind_tagged_object_does_not_panic() {
        // The `Unknown(String)` tuple variant serializes as `{"unknown":"x"}`.
        // A flat String deser would have failed the whole row; Value+label
        // normalizes it to "unknown" → amber, negative sign.
        let txs = vec![tx(serde_json::json!({"unknown": "weird"}), "system_grant", None, 7)];
        let rows = live_rows(Some(&txs)).unwrap();
        assert_eq!(rows[0].action, "unknown");
        assert_eq!(rows[0].color, theme::AMBER);
        assert_eq!(rows[0].agent, "system"); // no from/to → "system"
    }

    #[test]
    fn empty_txs_yield_no_rows() {
        // No live data → None (render shows honest "awaiting" line, not a
        // fabricated ledger).
        assert!(live_rows(Some(&[])).is_none());
        assert!(live_rows(None).is_none());
    }

    #[test]
    fn label_normalizes_bare_and_tagged() {
        let bare = EconomyTxResponse::label(&serde_json::json!("Earn"));
        assert_eq!(bare, "earn");
        let tagged = EconomyTxResponse::label(&serde_json::json!({"Unknown": "x"}));
        assert_eq!(tagged, "unknown");
        let empty = EconomyTxResponse::label(&serde_json::Value::Null);
        assert_eq!(empty, "");
    }

    /// Render-fidelity gate (TestBackend dump, NOT token-match — merakizzz's
    /// 3-overturn lesson): drive the panel into an 80×24 buffer and assert
    /// both sections + the LIVE aggregated wallet balance + a live tx actually
    /// paint.
    #[test]
    fn render_paints_both_sections_and_live_wallet() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let wallets = vec![wallet("a", 100, 50, 10), wallet("b", 200, 30, 5)];
        let txs = vec![tx(serde_json::json!("earn"), "marketplace_sale", Some("calliope"), 9)];

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            render(area, f.buffer_mut(), Some(&wallets), Some(&txs));
        })
        .unwrap();

        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(dump.contains("AGORA WALLET"));
        assert!(dump.contains("RECENT TRANSACTIONS"));
        // Live aggregate balance (100+200=300), distinct from the honest dash.
        assert!(dump.contains("300"), "live aggregate balance missing");
        assert!(!dump.contains("247.83"), "fabricated balance must not render");
        assert!(dump.contains("2 wallets"), "live wallet count missing");
        assert!(dump.contains("calliope"), "live tx agent missing");
    }

    #[test]
    fn render_empty_state_is_honest() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            render(area, f.buffer_mut(), None, None);
        })
        .unwrap();

        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(dump.contains("AGORA WALLET"));
        assert!(dump.contains("RECENT TRANSACTIONS"));
        // Honest empty state — no fabricated balance or transactions.
        assert!(dump.contains("awaiting"), "empty state should show awaiting line");
        assert!(!dump.contains("247.83"), "fabricated balance must not render");
        assert!(!dump.contains("Hephaestus"), "fabricated tx must not render");
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None, None);
    }
}
