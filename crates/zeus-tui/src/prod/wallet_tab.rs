//! Wallet tab — web4 economy (zeus-economy + zeus-wallet). #190.
//!
//! Built to the prototype `docs/zeuswalletprototypes/zeus-tui-production.jsx`
//! (WalletTab, JSX 1138–1374), with live gateway wiring where endpoints exist.
//!
//! Data model: CR credits are live from the `zeus-economy` SQLite ledger
//! (`/v1/economy/wallets` + `/v1/economy/transactions`). ZEUS token balances
//! and wallet addresses live in `zeus-wallet`/x402 and currently have no TUI
//! gateway endpoint, so the tab renders honest dashes / disabled affordances
//! instead of prototype mock balances or fabricated titan rosters.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::prod::draw::BufferClampExt;
use crate::theme;

/// Wallet sub-views, switched by number keys 1–6 (JSX `VIEWS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletView {
    Balance,
    Send,
    Receive,
    Activity,
    Economy,
    Security,
}

impl WalletView {
    /// Ordered views with their 1-indexed switch key + label (JSX `VIEWS`).
    pub const ALL: &'static [(WalletView, &'static str)] = &[
        (WalletView::Balance, "Balance"),
        (WalletView::Send, "Send"),
        (WalletView::Receive, "Receive"),
        (WalletView::Activity, "Activity"),
        (WalletView::Economy, "Economy"),
        (WalletView::Security, "Security"),
    ];

    /// Map a 1-indexed number key (1–6) to a view.
    pub fn from_key(n: usize) -> Option<WalletView> {
        Self::ALL.get(n.checked_sub(1)?).map(|(v, _)| *v)
    }
}

/// A fleet titan's wallet row. Legacy static schema retained for public API and
/// unit helpers; the production tab renders live `/v1/economy/wallets` rows.
#[derive(Debug, Clone, Copy)]
pub struct TitanWallet {
    pub name: &'static str,
    pub role: &'static str,
    pub addr: &'static str,
    pub token: u64,
    pub credit: u64,
    pub active: bool,
}

/// Transaction kind → recv/send/mint mapping (JSX `WALLET_KIND`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    Recv,
    Sent,
    Multi,
    Spend,
    Mint,
    Burn,
}

impl TxKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recv => "RECV",
            Self::Sent => "SENT",
            Self::Multi => "MULTI",
            Self::Spend => "SPND",
            Self::Mint => "MINT",
            Self::Burn => "BURN",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Recv | Self::Sent => "▸",
            Self::Multi => "⋔",
            Self::Spend => "◇",
            Self::Mint => "◈",
            Self::Burn => "✕",
        }
    }

    pub fn color(self) -> ratatui::style::Color {
        match self {
            Self::Recv | Self::Mint => theme::GREEN,
            Self::Sent => theme::ACCENT,
            Self::Multi => theme::CYAN,
            Self::Spend => theme::AMBER,
            Self::Burn => theme::RED,
        }
    }

    /// Inflow (+) vs outflow (−) for amount sign.
    pub fn is_inflow(self) -> bool {
        matches!(self, Self::Recv | Self::Mint)
    }
}

/// Transaction status (prototype `WALLET_STC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Ok,
    Pending,
    Failed,
}

impl TxStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Pending => "pend",
            Self::Failed => "fail",
        }
    }

    pub fn color(self) -> ratatui::style::Color {
        match self {
            Self::Ok => theme::GREEN,
            Self::Pending => theme::AMBER,
            Self::Failed => theme::RED,
        }
    }
}

/// A wallet transaction (legacy static schema retained for helper tests).
#[derive(Debug, Clone, Copy)]
pub struct WalletTx {
    pub kind: TxKind,
    pub who: &'static str,
    pub amount: u64,
    pub unit: &'static str,
    pub status: TxStatus,
    pub when: &'static str,
    pub note: &'static str,
}

/// Wallet data accessor — honest-empty baseline, live-overlaid via WalletLive.
#[derive(Debug, Clone)]
pub struct WalletData {
    pub human_zeus: u64,
    pub human_credit: u64,
    pub address: &'static str,
    pub titans: &'static [TitanWallet],
    pub activity: &'static [WalletTx],
    pub send_recipient: &'static str,
    pub send_amount: u64,
    pub send_memo: &'static str,
    pub send_chips: &'static [&'static str],
}

impl WalletData {
    /// Honest blank baseline. The prototype sample values are intentionally not
    /// compiled into the production tab; live gateway data fills what exists.
    pub fn sample() -> Self {
        WalletData {
            human_zeus: 0,
            human_credit: 0,
            address: "—",
            titans: &[],
            activity: &[],
            send_recipient: "",
            send_amount: 0,
            send_memo: "",
            send_chips: &[],
        }
    }
}

/// Format a number with thousands separators (JSX `wfmt`).
pub fn wfmt(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Live gateway data overlay for the Wallet tab.
#[derive(Default, Clone, Copy)]
pub struct WalletLive<'a> {
    /// Live wallet balances from `GET /v1/economy/wallets`.
    pub wallets: Option<&'a [crate::api::EconomyWalletResponse]>,
    /// Live ledger transactions from `GET /v1/economy/transactions`.
    pub transactions: Option<&'a [crate::api::EconomyTxResponse]>,
}

/// The wallet tab widget.
pub struct WalletTab<'a> {
    pub view: WalletView,
    pub titan_sel: usize,
    pub data: WalletData,
    pub live: Option<WalletLive<'a>>,
}

impl WalletTab<'_> {
    pub fn new() -> Self {
        WalletTab {
            view: WalletView::Balance,
            titan_sel: 0,
            data: WalletData::sample(),
            live: None,
        }
    }

    pub fn with_view(view: WalletView, titan_sel: usize) -> Self {
        WalletTab {
            view,
            titan_sel,
            data: WalletData::sample(),
            live: None,
        }
    }

    /// Build a tab that overlays live gateway data onto the wallet schema.
    pub fn with_live<'a>(
        view: WalletView,
        titan_sel: usize,
        live: WalletLive<'a>,
    ) -> WalletTab<'a> {
        WalletTab {
            view,
            titan_sel,
            data: WalletData::sample(),
            live: Some(live),
        }
    }
}

impl Default for WalletTab<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for WalletTab<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        fill_bg(area, buf);

        let header_style = Style::default()
            .fg(theme::TEXT_BRIGHT)
            .add_modifier(Modifier::BOLD);
        buf.set_string_clamped(area.x + 2, area.y, "⊟ WALLET", header_style);
        self.render_view_switcher(area, buf);

        let body = Rect {
            x: area.x,
            y: area.y + 4,
            width: area.width,
            height: area.height.saturating_sub(4),
        };

        match self.view {
            WalletView::Balance => self.render_balance(body, buf),
            WalletView::Send => self.render_send(body, buf),
            WalletView::Receive => self.render_receive(body, buf),
            WalletView::Activity => self.render_activity(body, buf),
            WalletView::Economy => self.render_economy(body, buf),
            WalletView::Security => self.render_security(body, buf),
        }
    }
}

impl WalletTab<'_> {
    fn wallets(&self) -> &[crate::api::EconomyWalletResponse] {
        self.live.and_then(|l| l.wallets).unwrap_or(&[])
    }

    fn txs(&self) -> &[crate::api::EconomyTxResponse] {
        self.live.and_then(|l| l.transactions).unwrap_or(&[])
    }

    fn selected_wallet(&self) -> Option<&crate::api::EconomyWalletResponse> {
        let wallets = self.wallets();
        wallets.get(self.titan_sel.min(wallets.len().saturating_sub(1)))
    }

    fn render_view_switcher(&self, area: Rect, buf: &mut Buffer) {
        let mut x = area.x + 2;
        let y = area.y + 2;
        for (i, (view, label)) in WalletView::ALL.iter().enumerate() {
            let selected = *view == self.view;
            let num_style = Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD);
            buf.set_string_clamped(x, y, format!("{}", i + 1), num_style);
            x += 2;

            let label_style = if selected {
                Style::default()
                    .fg(theme::WHITE)
                    .bg(theme::BG_HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };
            buf.set_string_clamped(x, y, *label, label_style);
            x += label.len() as u16 + 3;
        }
    }

    /// Balance view — HUMAN WALLET card + live fleet wallet rows.
    fn render_balance(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let wallets = self.wallets();
        let human_credit = wallets.first().map(|w| w.balance);
        let total_credit: u64 = wallets.iter().map(|w| w.balance).sum();
        let total_earned: u64 = wallets.iter().map(|w| w.total_earned).sum();
        let total_spent: u64 = wallets.iter().map(|w| w.total_spent).sum();

        card_title(buf, x, y, area.width, "HUMAN WALLET", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "ZEUS TOKEN", dim_bold());
        buf.set_string_clamped(x + 30, y, "CREDIT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(
            x + 3,
            y,
            "— ZEUS",
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
        );
        let credit = human_credit.map(wfmt).unwrap_or_else(|| "—".to_string());
        buf.set_string_clamped(
            x + 30,
            y,
            format!("{credit} CR"),
            Style::default()
                .fg(theme::AMBER)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(
            x + 3,
            y,
            "zeus-wallet address",
            Style::default().fg(theme::DIM),
        );
        buf.set_string_clamped(x + 25, y, "—", Style::default().fg(theme::MUTED));
        buf.set_string_clamped(
            x + 30,
            y,
            "x402 endpoint not exposed",
            Style::default().fg(theme::DIM),
        );
        y += 1;
        bottom_rule(buf, x, y, area.width, theme::ACCENT_DIM);
        y += 2;

        buf.set_string_clamped(x, y, "FLEET TITAN WALLETS", dim_bold());
        buf.set_string_clamped(
            x + 24,
            y,
            format!("{} agents · {} CR", wallets.len(), wfmt(total_credit)),
            Style::default().fg(theme::ACCENT_DIM),
        );
        y += 1;
        buf.set_string_clamped(
            x,
            y,
            format!(
                "earned {} CR · spent {} CR",
                wfmt(total_earned),
                wfmt(total_spent)
            ),
            Style::default().fg(theme::DIM),
        );
        y += 1;

        if wallets.is_empty() {
            buf.set_string_clamped(
                x,
                y,
                "No wallets — fetching /v1/economy/wallets…",
                Style::default().fg(theme::DIM),
            );
            return;
        }

        for (i, wallet) in wallets.iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            let selected = i == self.titan_sel.min(wallets.len().saturating_sub(1));
            let marker = if selected { "▸" } else { " " };
            let row_style = if selected {
                Style::default()
                    .fg(theme::WHITE)
                    .bg(theme::BG_HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT)
            };
            buf.set_string_clamped(x, y, marker, Style::default().fg(theme::ACCENT));
            buf.set_string_clamped(x + 2, y, short(&wallet.agent_id, 22), row_style);
            buf.set_string_clamped(x + 27, y, "ZEUS —", Style::default().fg(theme::DIM));
            buf.set_string_clamped(
                x + 37,
                y,
                format!("{} CR", wfmt(wallet.balance)),
                Style::default()
                    .fg(theme::AMBER)
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string_clamped(
                x + 55,
                y,
                format!(
                    "earn {} · spend {}",
                    wfmt(wallet.total_earned),
                    wfmt(wallet.total_spent)
                ),
                Style::default().fg(theme::DIM),
            );
            y += 1;
        }
    }

    /// Send view — prototype SEND TOKENS + x402 pay-flow, disabled honestly until
    /// zeus-wallet exposes a send endpoint to the TUI.
    fn render_send(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let selected = self
            .selected_wallet()
            .map(|w| w.agent_id.as_str())
            .unwrap_or("select live wallet");

        card_title(buf, x, y, area.width / 2, "SEND TOKENS", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "RECIPIENT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(
            x + 3,
            y,
            format!("▸ @{selected}"),
            Style::default().fg(theme::TEXT),
        );
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "AMOUNT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(
            x + 3,
            y,
            "— ZEUS",
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(x + 14, y, "[MAX disabled]", Style::default().fg(theme::DIM));
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "MEMO", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(
            x + 3,
            y,
            "waiting for zeus-wallet send endpoint",
            Style::default().fg(theme::DIM),
        );
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "fee — · x402", Style::default().fg(theme::DIM));
        buf.set_string_clamped(
            x + 21,
            y,
            " ▸ SIGN [disabled] ",
            Style::default().fg(theme::MUTED).bg(theme::BG_HIGHLIGHT),
        );
        y += 1;
        bottom_rule(buf, x, y, area.width / 2, theme::ACCENT_DIM);

        let px = x + 50;
        let mut py = area.y;
        card_title(
            buf,
            px,
            py,
            area.width.saturating_sub(52),
            "x402 PAY-FLOW",
            theme::AMBER,
        );
        py += 1;
        for (i, step) in [
            "compose intent",
            "sign with zeus-wallet",
            "settle x402 payment",
            "ledger mirror",
        ]
        .iter()
        .enumerate()
        {
            if py >= area.bottom() {
                break;
            }
            let style = if i == 0 {
                Style::default().fg(theme::AMBER)
            } else {
                Style::default().fg(theme::DIM)
            };
            buf.set_string_clamped(px, py, "│", Style::default().fg(theme::AMBER));
            buf.set_string_clamped(px + 3, py, format!("{}. {step}", i + 1), style);
            py += 1;
        }
        bottom_rule(buf, px, py, area.width.saturating_sub(52), theme::AMBER);
    }

    /// Receive view — human receive card plus live titan wallet targets.
    fn render_receive(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let wallets = self.wallets();

        card_title(buf, x, y, area.width / 2, "RECEIVE", theme::GREEN);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::GREEN));
        buf.set_string_clamped(x + 3, y, "YOUR ADDRESS", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::GREEN));
        buf.set_string_clamped(x + 3, y, "—", Style::default().fg(theme::TEXT_BRIGHT));
        buf.set_string_clamped(
            x + 8,
            y,
            "zeus-wallet public key not exposed",
            Style::default().fg(theme::DIM),
        );
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::GREEN));
        buf.set_string_clamped(
            x + 3,
            y,
            "[QR pending]",
            Style::default().fg(theme::ACCENT_DIM),
        );
        buf.set_string_clamped(
            x + 18,
            y,
            "↗ SHARE [s] disabled",
            Style::default().fg(theme::DIM),
        );
        y += 1;
        bottom_rule(buf, x, y, area.width / 2, theme::GREEN);

        let tx = x + 50;
        let mut ty = area.y;
        card_title(
            buf,
            tx,
            ty,
            area.width.saturating_sub(52),
            "RECEIVE TO A TITAN",
            theme::ACCENT,
        );
        ty += 1;
        if wallets.is_empty() {
            buf.set_string_clamped(
                tx,
                ty,
                "No wallets — fetching /v1/economy/wallets…",
                Style::default().fg(theme::DIM),
            );
            return;
        }
        for wallet in wallets.iter().take(6) {
            if ty >= area.bottom() {
                break;
            }
            buf.set_string_clamped(tx, ty, "│", Style::default().fg(theme::ACCENT_DIM));
            buf.set_string_clamped(tx + 2, ty, "◈", Style::default().fg(theme::ACCENT));
            buf.set_string_clamped(
                tx + 4,
                ty,
                short(&wallet.agent_id, 20),
                Style::default().fg(theme::WHITE),
            );
            buf.set_string_clamped(
                tx + 27,
                ty,
                format!("{} CR", wfmt(wallet.balance)),
                Style::default().fg(theme::AMBER),
            );
            buf.set_string_clamped(tx + 43, ty, "[QR —]", Style::default().fg(theme::DIM));
            ty += 1;
        }
        bottom_rule(
            buf,
            tx,
            ty,
            area.width.saturating_sub(52),
            theme::ACCENT_DIM,
        );
    }

    /// Activity view — live ledger transactions.
    fn render_activity(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let txs = self.txs();

        card_title(buf, x, y, area.width, "ACTIVITY", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(
            x,
            y,
            "TYPE   PARTY / REASON          AMOUNT        STATUS",
            dim_bold(),
        );
        y += 1;

        if txs.is_empty() {
            buf.set_string_clamped(
                x,
                y,
                "No transactions — fetching /v1/economy/transactions…",
                Style::default().fg(theme::DIM),
            );
            return;
        }

        for tx in txs {
            if y >= area.bottom() {
                break;
            }
            let kind = tx_kind(tx.kind_label().as_str());
            let who = tx_party(tx);
            let sign = if kind.is_inflow() { "+" } else { "−" };
            let amt_color = if kind.is_inflow() {
                theme::GREEN
            } else {
                theme::AMBER
            };
            buf.set_string_clamped(x, y, kind.glyph(), Style::default().fg(kind.color()));
            buf.set_string_clamped(
                x + 2,
                y,
                format!("{:<5}", kind.label()),
                Style::default()
                    .fg(kind.color())
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string_clamped(x + 9, y, short(&who, 22), Style::default().fg(theme::TEXT));
            buf.set_string_clamped(
                x + 33,
                y,
                format!("{sign}{} CR", wfmt(tx.amount)),
                Style::default().fg(amt_color).add_modifier(Modifier::BOLD),
            );
            buf.set_string_clamped(x + 49, y, "● ok", Style::default().fg(theme::GREEN));
            let reason = tx.reason_label();
            if !reason.is_empty() {
                buf.set_string_clamped(
                    x + 55,
                    y,
                    short(&reason, 28),
                    Style::default().fg(theme::DIM),
                );
            }
            y += 1;
        }
    }

    /// Economy view — the prototype AGORA WALLET card, backed by live CR ledger.
    fn render_economy(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let wallets = self.wallets();
        let txs = self.txs();
        let total_balance: u64 = wallets.iter().map(|w| w.balance).sum();
        let total_earned: u64 = wallets.iter().map(|w| w.total_earned).sum();
        let total_spent: u64 = wallets.iter().map(|w| w.total_spent).sum();

        card_title(buf, x, y, area.width, "AGORA WALLET", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT));
        let balance = if wallets.is_empty() {
            "—".to_string()
        } else {
            wfmt(total_balance)
        };
        buf.set_string_clamped(
            x + 3,
            y,
            format!("{balance} CR"),
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(
            x + 22,
            y,
            "USDC mock dropped — credits are live",
            Style::default().fg(theme::DIM),
        );
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT));
        buf.set_string_clamped(
            x + 3,
            y,
            format!(
                "+{} earned · −{} spent · {} wallets",
                wfmt(total_earned),
                wfmt(total_spent),
                wallets.len()
            ),
            Style::default().fg(theme::GREEN),
        );
        y += 1;
        bottom_rule(buf, x, y, area.width, theme::ACCENT_DIM);
        y += 2;

        if let Some(wallet) = self.selected_wallet() {
            buf.set_string_clamped(x, y, "SELECTED TITAN", dim_bold());
            y += 1;
            buf.set_string_clamped(
                x,
                y,
                short(&wallet.agent_id, 26),
                Style::default()
                    .fg(theme::WHITE)
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string_clamped(
                x + 30,
                y,
                format!("{} CR", wfmt(wallet.balance)),
                Style::default().fg(theme::AMBER),
            );
            buf.set_string_clamped(
                x + 46,
                y,
                format!(
                    "earned {} · spent {}",
                    wfmt(wallet.total_earned),
                    wfmt(wallet.total_spent)
                ),
                Style::default().fg(theme::DIM),
            );
            y += 2;
        }

        buf.set_string_clamped(x, y, "RECENT TRANSACTIONS", dim_bold());
        y += 1;
        if txs.is_empty() {
            buf.set_string_clamped(
                x,
                y,
                "Awaiting /v1/economy/transactions…",
                Style::default().fg(theme::DIM),
            );
            return;
        }
        for tx in txs.iter().take(8) {
            if y >= area.bottom() {
                break;
            }
            let kind = tx_kind(tx.kind_label().as_str());
            let sign = if kind.is_inflow() { "+" } else { "−" };
            let party = tx_party(tx);
            buf.set_string_clamped(x, y, kind.glyph(), Style::default().fg(kind.color()));
            buf.set_string_clamped(
                x + 2,
                y,
                short(&party, 24),
                Style::default().fg(theme::TEXT_BRIGHT),
            );
            buf.set_string_clamped(
                x + 29,
                y,
                short(&tx.reason_label(), 24),
                Style::default().fg(theme::DIM),
            );
            buf.set_string_clamped(
                x + 56,
                y,
                format!("{sign}{} CR", wfmt(tx.amount)),
                Style::default().fg(if kind.is_inflow() {
                    theme::GREEN
                } else {
                    theme::AMBER
                }),
            );
            y += 1;
        }
    }

    /// Security view — no secret/key material is exposed in the TUI.
    fn render_security(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        card_title(buf, x, y, area.width, "SECURITY", theme::RED);
        y += 1;
        for line in [
            "zeus-wallet keypair: stored outside the TUI",
            "private keys: never rendered",
            "x402 signing: disabled until gateway endpoint exists",
            "ledger mirror: /v1/economy/* read-only",
        ] {
            if y >= area.bottom() {
                break;
            }
            buf.set_string_clamped(x, y, "│", Style::default().fg(theme::RED));
            buf.set_string_clamped(x + 3, y, line, Style::default().fg(theme::TEXT));
            y += 1;
        }
        bottom_rule(buf, x, y, area.width, theme::RED);
    }
}

fn fill_bg(area: Rect, buf: &mut Buffer) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(theme::BG));
        }
    }
}

fn dim_bold() -> Style {
    Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD)
}

fn card_title(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    title: &str,
    color: ratatui::style::Color,
) {
    let right = "─".repeat(width.saturating_sub(title.len() as u16 + 10).min(60) as usize);
    buf.set_string_clamped(x, y, "╭── ", Style::default().fg(theme::ACCENT_DIM));
    buf.set_string_clamped(
        x + 4,
        y,
        title,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
    buf.set_string_clamped(
        x + 5 + title.len() as u16,
        y,
        format!(" {right}╮"),
        Style::default().fg(theme::ACCENT_DIM),
    );
}

fn bottom_rule(buf: &mut Buffer, x: u16, y: u16, width: u16, color: ratatui::style::Color) {
    let rule = "─".repeat(width.saturating_sub(4).min(72) as usize);
    buf.set_string_clamped(x, y, format!("╰{rule}╯"), Style::default().fg(color));
}

fn tx_kind(label: &str) -> TxKind {
    match label {
        "earn" | "recv" | "receive" => TxKind::Recv,
        "spend" | "fee" => TxKind::Spend,
        "mint" => TxKind::Mint,
        "burn" => TxKind::Burn,
        "transfer" | "send" | "sent" => TxKind::Sent,
        _ => TxKind::Multi,
    }
}

fn tx_party(tx: &crate::api::EconomyTxResponse) -> String {
    match (&tx.from_agent, &tx.to_agent) {
        (Some(from), Some(to)) => format!("{from}→{to}"),
        (Some(from), None) => from.clone(),
        (None, Some(to)) => format!("→{to}"),
        (None, None) => tx.reason_label(),
    }
}

fn short(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let mut out = String::new();
    for _ in 0..max {
        match chars.next() {
            Some(c) => out.push(c),
            None => return out,
        }
    }
    if chars.next().is_some() && max > 1 {
        out.pop();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn wallet_views_render_against_bottom_edge() {
        for view in [
            WalletView::Balance,
            WalletView::Send,
            WalletView::Receive,
            WalletView::Activity,
            WalletView::Economy,
            WalletView::Security,
        ] {
            let area = Rect::new(0, 0, 100, 30);
            let mut buf = Buffer::empty(area);
            WalletTab::with_view(view, 0).render(area, &mut buf);
        }
    }

    #[test]
    fn live_balance_never_renders_prototype_fakes() {
        let wallets = [
            crate::api::EconomyWalletResponse {
                agent_id: "zeus106".into(),
                balance: 1_234,
                total_earned: 2_000,
                total_spent: 766,
            },
            crate::api::EconomyWalletResponse {
                agent_id: "zeus100".into(),
                balance: 5_678,
                total_earned: 7_000,
                total_spent: 1_322,
            },
        ];
        let live = WalletLive {
            wallets: Some(&wallets),
            transactions: None,
        };
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        WalletTab::with_live(WalletView::Balance, 0, live).render(area, &mut buf);
        let dump = buf.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(dump.contains("zeus106"));
        assert!(dump.contains("1,234 CR"));
        for fake in ["Hermes", "Atlas", "Calliope", "184,920"] {
            assert!(!dump.contains(fake), "fabricated {fake:?} rendered");
        }
    }
}
