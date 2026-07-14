//! Render-fidelity tests for the prod Wallet tab (#190 → #274 de-mock).
//!
//! Built to the prototype `docs/zeuswalletprototypes/zeus-tui-production.jsx`
//! (WalletTab, JSX 1106–1430). After the #274 de-mock closeout the tab no
//! longer ships a fabricated sample roster/ledger: with no live data every
//! view renders an honest empty state, and live `/v1/economy/*` data drives
//! the Balance fleet list + Activity ledger. ZEUS token is on-chain
//! (zeus-wallet) now overlays live `/v1/wallet/onchain` data → honest waiting/dash.
//!
//! These exercise the WalletTab Widget directly, asserting:
//!   - header glyph "⊟ WALLET" + the 1–6 sub-view switcher row (JSX VIEWS);
//!   - Balance/Activity honest-empty states when no live data is present;
//!   - NO fabricated titans (Hermes/Atlas/…) or sample balances ever render;
//!   - live wallets/transactions render real gateway data;
//!   - wfmt thousands-separator formatting + enum label helpers.

use ratatui::backend::TestBackend;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use zeus_tui::api::{
    EconomyTxResponse, EconomyWalletResponse, OnchainTransferPlanResponse,
    OnchainTransferResponse, OnchainTxResponse, OnchainWalletResponse,
};
use zeus_tui::prod::wallet_tab::{wfmt, TxKind, TxStatus, WalletLive, WalletTab, WalletView};

/// Render a standalone (no-live) WalletTab into a 120×44 TestBackend → String.
fn render_view(view: WalletView, titan_sel: usize) -> String {
    render_into(WalletTab::with_view(view, titan_sel))
}

/// Render a live-overlaid WalletTab → String.
fn render_live(view: WalletView, titan_sel: usize, live: WalletLive<'_>) -> String {
    render_into(WalletTab::with_live(view, titan_sel, live))
}

fn render_into(tab: WalletTab<'_>) -> String {
    let backend = TestBackend::new(120, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            let area = f.area();
            tab.render(area, f.buffer_mut());
        })
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row);
    }
    lines.join("\n")
}

/// Titan names that the pre-#274 mock fabricated. None may ever render.
const FABRICATED_TITANS: &[&str] = &[
    "Hermes",
    "Hephaestus",
    "Atlas",
    "Aegis",
    "Calliope",
    "Prometheus",
    "Argus",
];

// ── Header + sub-view switcher (every view) ─────────────────────────────────

#[test]
fn header_and_view_switcher_render() {
    let s = render_view(WalletView::Balance, 0);
    assert!(s.contains("⊟ WALLET"), "header glyph + title missing:\n{s}");
    for label in [
        "Balance", "Send", "Receive", "Activity", "Economy", "Security",
    ] {
        assert!(s.contains(label), "switcher label {label:?} missing:\n{s}");
    }
}

// ── Balance view — honest empty (no live) ───────────────────────────────────

#[test]
fn wallet_live_render_dump_smoke() {
    let wallets = vec![
        EconomyWalletResponse {
            agent_id: "zeus-titan".into(),
            balance: 18_492,
            total_earned: 25_000,
            total_spent: 6_508,
        },
        EconomyWalletResponse {
            agent_id: "zeus100".into(),
            balance: 4_200,
            total_earned: 8_000,
            total_spent: 3_800,
        },
    ];
    let txs = vec![EconomyTxResponse {
        kind: serde_json::json!("earn"),
        reason: serde_json::json!("human_sent_tokens"),
        from_agent: Some("merakizzz".into()),
        to_agent: Some("zeus-titan".into()),
        amount: 3_000,
    }];
    let live = WalletLive {
        wallets: Some(&wallets),
        transactions: Some(&txs),
        ..WalletLive::default()
    };
    let balance = render_live(WalletView::Balance, 0, live);
    let live = WalletLive {
        wallets: Some(&wallets),
        transactions: Some(&txs),
        ..WalletLive::default()
    };
    let economy = render_live(WalletView::Economy, 0, live);
    let dump = format!("== BALANCE ==\n{balance}\n\n== ECONOMY ==\n{economy}\n");
    assert!(dump.contains("HUMAN WALLET"));
    assert!(dump.contains("AGORA WALLET"));
    assert!(dump.contains("zeus-titan"));
    assert!(dump.contains("18,492 CR"));
    assert!(dump.contains("22,692 CR"));
    if let Ok(path) = std::env::var("ZEUS_WALLET_RENDER_DUMP") {
        std::fs::write(path, dump).expect("write wallet render dump");
    }
}

#[test]
fn balance_human_wallet_card_labels_present() {
    let s = render_view(WalletView::Balance, 0);
    assert!(s.contains("HUMAN WALLET"), "card title missing:\n{s}");
    assert!(
        s.contains("ON-CHAIN ZEUS"),
        "ON-CHAIN ZEUS label missing (on-chain layer)"
    );
    assert!(
        s.contains("CREDIT"),
        "CREDIT label missing (off-chain layer)"
    );
}

#[test]
fn balance_empty_renders_no_fabricated_data() {
    let s = render_view(WalletView::Balance, 0);
    for fake in FABRICATED_TITANS {
        assert!(
            !s.contains(fake),
            "fabricated titan {fake:?} rendered:\n{s}"
        );
    }
    // Old fabricated sample balances must be gone.
    assert!(
        !s.contains("184,920"),
        "fabricated ZEUS balance leaked:\n{s}"
    );
    assert!(!s.contains("4,680"), "fabricated CR balance leaked:\n{s}");
    assert!(
        !s.contains("48,210"),
        "fabricated titan ZEUS column leaked:\n{s}"
    );
    // Honest empty fleet state.
    assert!(
        s.contains("No internal wallets"),
        "expected honest empty fleet state:\n{s}"
    );
}

#[test]
fn balance_live_renders_real_wallets() {
    let wallets = vec![
        EconomyWalletResponse {
            agent_id: "zeus106".into(),
            balance: 1_234,
            ..Default::default()
        },
        EconomyWalletResponse {
            agent_id: "zeus100".into(),
            balance: 5_678,
            ..Default::default()
        },
    ];
    let live = WalletLive {
        wallets: Some(&wallets),
        transactions: None,
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Balance, 0, live);
    assert!(
        s.contains("FLEET TITAN WALLETS"),
        "titan list header missing:\n{s}"
    );
    assert!(s.contains("zeus106"), "live agent_id row missing:\n{s}");
    assert!(s.contains("zeus100"), "live agent_id row missing:\n{s}");
    assert!(s.contains("1,234"), "live CR balance missing:\n{s}");
    assert!(
        s.contains("▶ zeus106"),
        "selected-titan marker missing on idx 0:\n{s}"
    );
    for fake in FABRICATED_TITANS {
        assert!(
            !s.contains(fake),
            "fabricated titan {fake:?} rendered with live data:\n{s}"
        );
    }
}

#[test]
fn balance_live_selection_marker_moves() {
    let wallets = vec![
        EconomyWalletResponse {
            agent_id: "zeus106".into(),
            balance: 1,
            ..Default::default()
        },
        EconomyWalletResponse {
            agent_id: "zeus100".into(),
            balance: 2,
            ..Default::default()
        },
    ];
    let live = WalletLive {
        wallets: Some(&wallets),
        transactions: None,
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Balance, 1, live);
    assert!(
        s.contains("▶ zeus100"),
        "marker should be on titan idx 1:\n{s}"
    );
    assert!(
        !s.contains("▶ zeus106"),
        "marker should NOT be on idx 0:\n{s}"
    );
}

#[test]
fn balance_live_renders_onchain_devnet_overlay() {
    let wallets = vec![EconomyWalletResponse {
        agent_id: "zeus-titan".into(),
        balance: 42,
        total_earned: 100,
        total_spent: 58,
    }];
    let onchain = OnchainWalletResponse {
        address: "4gHmZHyndwo3hkxqebpfWghZDaw3j3Nhy8fNc2X1oDck".into(),
        sol_lamports: 1_250_000_000,
        sol: 1.25,
        token_balance: 12_345_000,
        token_decimals: 6,
        mint: "ZEUSDEVNETMINT111111111111111111111111111".into(),
        cluster: "devnet".into(),
    };
    let live = WalletLive {
        wallets: Some(&wallets),
        onchain_wallet: Some(&onchain),
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Balance, 0, live);
    assert!(s.contains("12.345 ZEUS"), "ZEUS token balance missing:
{s}");
    assert!(s.contains("1.25 SOL"), "SOL balance missing:
{s}");
    assert!(s.contains("DEVNET"), "cluster badge missing:
{s}");
    assert!(s.contains("4gHm"), "short address prefix missing:
{s}");
}

#[test]
fn balance_without_onchain_data_is_honest_waiting_state() {
    let s = render_view(WalletView::Balance, 0);
    assert!(
        s.contains("Fetching /v1/wallet/onchain"),
        "wallet should wait for live on-chain endpoint, not fake balances:
{s}"
    );
    assert!(s.contains("— ZEUS"), "token balance should be dashed before live data:
{s}");
    assert!(!s.contains("12.345 ZEUS"), "prototype/on-chain fake balance leaked:
{s}");
}

#[test]
fn send_view_renders_onchain_preflight_plan() {
    let transfer = OnchainTransferResponse {
        signature: "5NfZeusDevnetSignature111111111111111111111111111111111".into(),
        sender: "4gHmZHyndwo3hkxqebpfWghZDaw3j3Nhy8fNc2X1oDck".into(),
        recipient: "9xRecipientDevnet111111111111111111111111111111".into(),
        amount: 7_500_000,
        mint: "ZEUSDEVNETMINT111111111111111111111111111".into(),
        ata_created: true,
        cluster: "devnet".into(),
        plan: OnchainTransferPlanResponse {
            sender_sol_lamports: 2_000_000_000,
            sender_token_balance: 9_000_000,
            token_balance_sufficient: true,
            recipient_ata_exists: false,
            ata_create_required: true,
        },
    };
    let live = WalletLive {
        onchain_transfer: Some(&transfer),
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Send, 0, live);
    assert!(s.contains("PREFLIGHT PLAN"), "preflight card missing:
{s}");
    assert!(s.contains("sufficient yes"), "sufficiency flag missing:
{s}");
    assert!(s.contains("ATA create required yes"), "ATA create flag missing:
{s}");
    assert!(s.contains("7,500,000 raw ZEUS"), "transfer amount missing:
{s}");
}

// ── Activity view ───────────────────────────────────────────────────────────

#[test]
fn activity_empty_renders_no_fabricated_ledger() {
    let s = render_view(WalletView::Activity, 0);
    assert!(s.contains("ACTIVITY"), "activity card header missing:\n{s}");
    for fake in [
        "Agora→Calliope",
        "Prometheus→Ledger",
        "x402 content sale",
        "audit retainer",
    ] {
        assert!(!s.contains(fake), "fabricated tx {fake:?} rendered:\n{s}");
    }
    assert!(
        s.contains("No transactions"),
        "expected honest empty activity state:\n{s}"
    );
}

#[test]
fn activity_live_renders_onchain_signatures() {
    let onchain = vec![OnchainTxResponse {
        signature: "5NfZeusDevnetSignature111111111111111111111111111111111".into(),
        slot: 42,
        block_time: Some(1_725_000_000),
        confirmation_status: Some("confirmed".into()),
        err: None,
    }];
    let live = WalletLive {
        onchain_transactions: Some(&onchain),
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Activity, 0, live);
    assert!(s.contains("ON-CHAIN SIGNATURES"), "signature card missing:
{s}");
    assert!(s.contains("5NfZ"), "short signature prefix missing:
{s}");
    assert!(s.contains("slot 42"), "signature slot missing:
{s}");
    assert!(s.contains("confirmed · ok"), "signature status missing:
{s}");
}

#[test]
fn activity_live_renders_real_transactions() {
    let txs = vec![EconomyTxResponse {
        from_agent: Some("zeus106".into()),
        to_agent: Some("zeus100".into()),
        amount: 2_400,
        ..Default::default()
    }];
    let live = WalletLive {
        wallets: None,
        transactions: Some(&txs),
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Activity, 0, live);
    assert!(
        s.contains("zeus106→zeus100"),
        "live tx parties missing:\n{s}"
    );
    assert!(s.contains("2,400"), "live tx amount missing:\n{s}");
    assert!(
        !s.contains("No transactions"),
        "should not show empty state with live txs:\n{s}"
    );
}

// ── Economy + Security views — implemented, honest live wiring ──────────────

#[test]
fn economy_view_renders_agora_wallet_card_without_usdc_mock() {
    let wallets = vec![
        EconomyWalletResponse {
            agent_id: "zeus-titan".into(),
            balance: 9_876,
            total_earned: 12_000,
            total_spent: 2_124,
        },
        EconomyWalletResponse {
            agent_id: "zeus100".into(),
            balance: 123,
            total_earned: 500,
            total_spent: 377,
        },
    ];
    let live = WalletLive {
        wallets: Some(&wallets),
        transactions: None,
        ..WalletLive::default()
    };
    let s = render_live(WalletView::Economy, 0, live);
    assert!(s.contains("AGORA WALLET"), "economy card missing:\n{s}");
    assert!(s.contains("9,999 CR"), "aggregate CR balance missing:\n{s}");
    assert!(
        s.contains("zeus-titan"),
        "selected live wallet missing:\n{s}"
    );
    assert!(!s.contains("$ 247.83"), "prototype USDC mock leaked:\n{s}");
}

#[test]
fn security_view_renders_key_safety_state() {
    let s = render_view(WalletView::Security, 0);
    assert!(s.contains("SECURITY"), "security title missing:\n{s}");
    assert!(
        s.contains("private keys: never rendered"),
        "key-safety copy missing:\n{s}"
    );
    assert!(
        !s.contains("later phase"),
        "security should be implemented now:\n{s}"
    );
}

// ── Pure helpers (data-independent) ─────────────────────────────────────────

#[test]
fn tx_kind_maps_to_color_sign_and_label() {
    assert!(TxKind::Recv.is_inflow());
    assert!(TxKind::Mint.is_inflow());
    assert!(!TxKind::Sent.is_inflow());
    assert!(!TxKind::Spend.is_inflow());
    assert_eq!(TxKind::Recv.label(), "RECV");
    assert_eq!(TxKind::Multi.label(), "MULTI");
}

#[test]
fn tx_status_field_present() {
    assert_eq!(TxStatus::Ok.label(), "ok");
    assert_eq!(TxStatus::Pending.label(), "pend");
    assert_eq!(TxStatus::Failed.label(), "fail");
}

#[test]
fn view_from_key_maps_1_to_6() {
    assert_eq!(WalletView::from_key(1), Some(WalletView::Balance));
    assert_eq!(WalletView::from_key(4), Some(WalletView::Activity));
    assert_eq!(WalletView::from_key(6), Some(WalletView::Security));
    assert_eq!(WalletView::from_key(0), None);
    assert_eq!(WalletView::from_key(7), None);
}

#[test]
fn wfmt_thousands_separators() {
    assert_eq!(wfmt(0), "0");
    assert_eq!(wfmt(999), "999");
    assert_eq!(wfmt(1_000), "1,000");
    assert_eq!(wfmt(184_920), "184,920");
    assert_eq!(wfmt(1_000_000), "1,000,000");
}
