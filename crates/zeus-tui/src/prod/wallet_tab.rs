//! Wallet tab — web4 economy (zeus-economy + zeus-wallet). #190.
//!
//! Built to the prototype `docs/zeuswalletprototypes/zeus-tui-production.jsx`
//! (WalletTab, JSX 1138–1374), with live gateway wiring where endpoints exist.
//!
//! Data model: CR credits are live from the `zeus-economy` SQLite ledger
//! (`/v1/economy/wallets` + `/v1/economy/transactions`). On-chain SOL, ZEUS
//! token balance, public address, devnet cluster, recent signatures, and transfer
//! preflight plans are overlaid from `/v1/wallet/onchain/*` (#352). Missing live
//! data renders honest waiting/empty states, never prototype balances.

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
    /// Live on-chain wallet summary from `GET /v1/wallet/onchain`.
    pub onchain_wallet: Option<&'a crate::api::OnchainWalletResponse>,
    /// Live on-chain signature list from `GET /v1/wallet/onchain/transactions`.
    pub onchain_transactions: Option<&'a [crate::api::OnchainTxResponse]>,
    /// Last on-chain transfer response, including the `build_transfer_plan`
    /// preflight returned by `POST /v1/wallet/onchain/transfer`.
    pub onchain_transfer: Option<&'a crate::api::OnchainTransferResponse>,
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

    fn onchain_wallet(&self) -> Option<&crate::api::OnchainWalletResponse> {
        self.live.and_then(|l| l.onchain_wallet)
    }

    fn onchain_txs(&self) -> Option<&[crate::api::OnchainTxResponse]> {
        self.live.and_then(|l| l.onchain_transactions)
    }

    fn onchain_transfer(&self) -> Option<&crate::api::OnchainTransferResponse> {
        self.live.and_then(|l| l.onchain_transfer)
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
        let onchain = self.onchain_wallet();
        let human_credit = wallets.first().map(|w| w.balance);
        let total_credit: u64 = wallets.iter().map(|w| w.balance).sum();
        let total_earned: u64 = wallets.iter().map(|w| w.total_earned).sum();
        let total_spent: u64 = wallets.iter().map(|w| w.total_spent).sum();

        card_title(buf, x, y, area.width, "HUMAN WALLET", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "ON-CHAIN ZEUS", dim_bold());
        buf.set_string_clamped(x + 33, y, "ECONOMY CREDIT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        let token = onchain
            .map(|w| format!("{} ZEUS", token_amount_fmt(w.token_balance, w.token_decimals)))
            .unwrap_or_else(|| "— ZEUS".to_string());
        buf.set_string_clamped(x + 3, y, token, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD));
        let credit = human_credit.map(wfmt).unwrap_or_else(|| "—".to_string());
        buf.set_string_clamped(x + 33, y, format!("{credit} CR"), Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD));
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        match onchain {
            Some(wallet) => {
                buf.set_string_clamped(x + 3, y, format!("{} SOL", sol_fmt(wallet.sol_lamports)), Style::default().fg(theme::GREEN));
                buf.set_string_clamped(x + 18, y, cluster_badge(&wallet.cluster), Style::default().fg(cluster_color(&wallet.cluster)).add_modifier(Modifier::BOLD));
                buf.set_string_clamped(x + 30, y, format!("addr {}", short_addr(&wallet.address)), Style::default().fg(theme::TEXT));
            }
            None => {
                buf.set_string_clamped(x + 3, y, "Fetching /v1/wallet/onchain…", Style::default().fg(theme::DIM));
            }
        }
        y += 1;
        bottom_rule(buf, x, y, area.width, theme::ACCENT_DIM);
        y += 2;

        buf.set_string_clamped(x, y, "FLEET TITAN WALLETS", dim_bold());
        buf.set_string_clamped(x + 24, y, format!("{} agents · {} CR", wallets.len(), wfmt(total_credit)), Style::default().fg(theme::ACCENT_DIM));
        y += 1;
        buf.set_string_clamped(x, y, format!("earned {} CR · spent {} CR", wfmt(total_earned), wfmt(total_spent)), Style::default().fg(theme::DIM));
        y += 1;

        if wallets.is_empty() {
            buf.set_string_clamped(x, y, "No internal wallets — fetching /v1/economy/wallets…", Style::default().fg(theme::DIM));
            return;
        }

        for (i, wallet) in wallets.iter().enumerate().take(8) {
            if y >= area.bottom() { break; }
            let selected = i == self.titan_sel.min(wallets.len().saturating_sub(1));
            let marker = if selected { "▶" } else { " " };
            let row_style = if selected { Style::default().fg(theme::WHITE).bg(theme::BG_HIGHLIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme::TEXT) };
            buf.set_string_clamped(x, y, marker, Style::default().fg(theme::ACCENT));
            buf.set_string_clamped(x + 2, y, short(&wallet.agent_id, 22), row_style);
            buf.set_string_clamped(x + 27, y, "on-chain —", Style::default().fg(theme::DIM));
            buf.set_string_clamped(x + 42, y, format!("{} CR", wfmt(wallet.balance)), Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD));
            buf.set_string_clamped(x + 60, y, format!("earn {} · spend {}", wfmt(wallet.total_earned), wfmt(wallet.total_spent)), Style::default().fg(theme::DIM));
            y += 1;
        }
    }

    /// Send view — prototype SEND TOKENS + x402 pay-flow. The gateway endpoint
    /// `POST /v1/economy/transfer` is live (#190 P2) and the TUI API client has
    /// `economy_transfer()` ready, but this view is still render-only: interactive
    /// input (recipient/amount fields, SIGN action) needs a follow-up UI pass.
    fn render_send(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let onchain = self.onchain_wallet();
        let transfer = self.onchain_transfer();
        let selected = self.selected_wallet().map(|w| w.agent_id.as_str()).unwrap_or("select internal wallet");

        card_title(buf, x, y, area.width / 2, "SEND ZEUS — DEVNET", theme::ACCENT);
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "SOURCE", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        match onchain {
            Some(wallet) => buf.set_string_clamped(x + 3, y, format!("{} · {}", short_addr(&wallet.address), cluster_badge(&wallet.cluster)), Style::default().fg(theme::TEXT)),
            None => buf.set_string_clamped(x + 3, y, "Waiting for /v1/wallet/onchain…", Style::default().fg(theme::DIM)),
        };
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "RECIPIENT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        let recipient = transfer.map(|t| short_addr(&t.recipient)).unwrap_or_else(|| format!("@{selected} / base58 pending"));
        buf.set_string_clamped(x + 3, y, recipient, Style::default().fg(theme::TEXT));
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "AMOUNT", dim_bold());
        y += 1;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        let amount = transfer.map(|t| format!("{} raw ZEUS", wfmt(t.amount))).unwrap_or_else(|| "— ZEUS".to_string());
        buf.set_string_clamped(x + 3, y, amount, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD));
        y += 2;
        buf.set_string_clamped(x, y, "│", Style::default().fg(theme::ACCENT_DIM));
        buf.set_string_clamped(x + 3, y, "POST /v1/wallet/onchain/transfer returns build_transfer_plan before submit", Style::default().fg(theme::DIM));
        y += 1;
        bottom_rule(buf, x, y, area.width / 2, theme::ACCENT_DIM);

        let px = x + 52;
        let mut py = area.y;
        card_title(buf, px, py, area.width.saturating_sub(52), "PREFLIGHT PLAN", theme::AMBER);
        py += 1;
        match transfer {
            Some(tx) => {
                let rows = [
                    format!("token balance {}", wfmt(tx.plan.sender_token_balance)),
                    format!("needed {} · sufficient {}", wfmt(tx.amount), yes_no(tx.plan.token_balance_sufficient)),
                    format!("recipient ATA {}", if tx.plan.recipient_ata_exists { "exists" } else { "missing" }),
                    format!("ATA create required {}", yes_no(tx.plan.ata_create_required)),
                    format!("fee wallet {} SOL", sol_fmt(tx.plan.sender_sol_lamports)),
                    format!("signature {}", short_addr(&tx.signature)),
                ];
                for row in rows { if py >= area.bottom() { break; } buf.set_string_clamped(px, py, "│", Style::default().fg(theme::AMBER)); buf.set_string_clamped(px + 3, py, row, Style::default().fg(theme::TEXT)); py += 1; }
            }
            None if onchain.is_some() => {
                for row in ["No transfer response yet.", "Enter recipient/amount in the follow-up send UI to build plan.", "Gateway guard: devnet-only; mainnet returns 403."] { if py >= area.bottom() { break; } buf.set_string_clamped(px, py, "│", Style::default().fg(theme::AMBER)); buf.set_string_clamped(px + 3, py, row, Style::default().fg(theme::DIM)); py += 1; }
            }
            None => {
                for row in ["Waiting for on-chain wallet data…", "No preflight plan requested yet."] { if py >= area.bottom() { break; } buf.set_string_clamped(px, py, "│", Style::default().fg(theme::AMBER)); buf.set_string_clamped(px + 3, py, row, Style::default().fg(theme::DIM)); py += 1; }
            }
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

    /// Activity view — live ledger transactions plus recent on-chain signatures.
    fn render_activity(&self, area: Rect, buf: &mut Buffer) {
        let x = area.x + 2;
        let mut y = area.y;
        let txs = self.txs();
        let onchain_txs = self.onchain_txs();

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
            y += 2;
        } else {
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
                buf.set_string_clamped(
                    x,
                    y,
                    format!("{} {}", kind.glyph(), kind.label()),
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
            y += 1;
        }

        if y >= area.bottom() {
            return;
        }
        card_title(buf, x, y, area.width, "ON-CHAIN SIGNATURES", theme::GREEN);
        y += 1;
        match onchain_txs {
            None => {
                buf.set_string_clamped(
                    x,
                    y,
                    "Waiting for /v1/wallet/onchain/transactions…",
                    Style::default().fg(theme::DIM),
                );
            }
            Some([]) => {
                buf.set_string_clamped(
                    x,
                    y,
                    "No on-chain signatures yet.",
                    Style::default().fg(theme::DIM),
                );
            }
            Some(rows) => {
                for tx in rows.iter().take(5) {
                    if y >= area.bottom() {
                        break;
                    }
                    let status = tx.confirmation_status.as_deref().unwrap_or("confirmed");
                    let err = if tx.err.is_some() { "err" } else { "ok" };
                    buf.set_string_clamped(x, y, "│", Style::default().fg(theme::GREEN));
                    buf.set_string_clamped(
                        x + 3,
                        y,
                        format!("sig {}", short_addr(&tx.signature)),
                        Style::default().fg(theme::TEXT),
                    );
                    buf.set_string_clamped(
                        x + 23,
                        y,
                        format!("slot {}", tx.slot),
                        Style::default().fg(theme::DIM),
                    );
                    buf.set_string_clamped(
                        x + 40,
                        y,
                        format!("{status} · {err}"),
                        Style::default().fg(if tx.err.is_some() {
                            theme::RED
                        } else {
                            theme::GREEN
                        }),
                    );
                    y += 1;
                }
            }
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


fn short_addr(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 12 {
        return s.to_string();
    }
    format!("{}…{}", chars[..4].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
}

fn sol_fmt(lamports: u64) -> String {
    let whole = lamports / 1_000_000_000;
    let frac = (lamports % 1_000_000_000) / 1_000_000;
    if frac == 0 {
        format!("{whole}")
    } else {
        format!("{whole}.{frac:03}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn token_amount_fmt(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return wfmt(raw);
    }
    let scale = 10_u64.saturating_pow(decimals as u32);
    if scale == 0 {
        return wfmt(raw);
    }
    let whole = raw / scale;
    let frac = raw % scale;
    if frac == 0 {
        return wfmt(whole);
    }
    let mut frac_s = format!("{frac:0width$}", width = decimals as usize);
    while frac_s.ends_with('0') {
        frac_s.pop();
    }
    format!("{}.{frac_s}", wfmt(whole))
}

fn cluster_badge(cluster: &str) -> String {
    match cluster {
        "devnet" => "DEVNET".to_string(),
        "testnet" => "TESTNET".to_string(),
        "mainnet" => "MAINNET".to_string(),
        other if !other.is_empty() => other.to_ascii_uppercase(),
        _ => "CLUSTER —".to_string(),
    }
}

fn cluster_color(cluster: &str) -> ratatui::style::Color {
    match cluster {
        "devnet" => theme::GREEN,
        "testnet" => theme::AMBER,
        "mainnet" => theme::RED,
        _ => theme::DIM,
    }
}

fn yes_no(v: bool) -> &'static str {
    if v { "yes" } else { "no" }
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
            ..WalletLive::default()
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
