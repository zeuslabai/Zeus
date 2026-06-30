use ratatui::backend::TestBackend;
use ratatui::Terminal;
use zeus_tui::api::{
    AgentResponse, EconomyTxResponse, EconomyWalletResponse, SkillResponse,
};
use zeus_tui::prod::advanced_sub::{self, AdvancedLive};

fn render_dump(tab_id: &'static str, live: AdvancedLive<'_>) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            advanced_sub::render(tab_id, area, f.buffer_mut(), &live);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
}

#[test]
fn agents_subview_matches_prototype_roster_card_shape() {
    let agents = vec![
        AgentResponse {
            id: "local".into(),
            name: "zeus-titan".into(),
            status: "active".into(),
            agent_type: "local".into(),
            ..Default::default()
        },
        AgentResponse {
            id: "remote".into(),
            name: "zeus106".into(),
            status: "idle".into(),
            agent_type: "channel".into(),
            ..Default::default()
        },
    ];
    let live = AdvancedLive { agents: Some(&agents), ..AdvancedLive::default() };
    let dump = render_dump("agents", live);

    assert!(dump.contains("zeus-titan"));
    assert!(dump.contains("LOCAL"));
    assert!(dump.contains("zeus106"));
    assert!(dump.contains("channels"));
    assert!(dump.contains("[MSG]"));
    assert!(dump.contains("—"), "host/role columns must stay honest when the API lacks them");
    assert!(!dump.contains("Hermes"), "prototype fixture names must not leak over live data");
}

#[test]
fn skills_subview_matches_marketplace_summary_and_rows() {
    let skills = vec![
        SkillResponse { name: "code-review".into(), description: "Review patches".into(), enabled: true },
        SkillResponse { name: "image-gen".into(), description: "Generate images".into(), enabled: false },
    ];
    let live = AdvancedLive { skills: Some(&skills), ..AdvancedLive::default() };
    let dump = render_dump("skills", live);

    assert!(dump.contains("2 installed"));
    assert!(dump.contains("marketplace"));
    assert!(dump.contains("BROWSE MARKETPLACE"));
    assert!(dump.contains("code-review"));
    assert!(dump.contains("image-gen"));
    assert!(dump.contains("[VIEW]"));
    assert!(dump.contains("[DISABLE]"));
    assert!(dump.contains("[ENABLE]"));
    assert!(!dump.contains("147 in marketplace"), "unwired marketplace counts must not be fabricated");
}

#[test]
fn economy_subview_matches_agora_wallet_and_transactions() {
    let wallets = vec![
        EconomyWalletResponse {
            agent_id: "zeus-titan".into(),
            balance: 120,
            total_earned: 200,
            total_spent: 80,
        },
        EconomyWalletResponse {
            agent_id: "zeus106".into(),
            balance: 30,
            total_earned: 40,
            total_spent: 10,
        },
    ];
    let txs = vec![EconomyTxResponse {
        kind: serde_json::json!("earn"),
        reason: serde_json::json!("marketplace_sale"),
        from_agent: Some("zeus106".into()),
        to_agent: Some("zeus-titan".into()),
        amount: 12,
    }];
    let live = AdvancedLive {
        economy_wallets: Some(&wallets),
        economy_txs: Some(&txs),
        ..AdvancedLive::default()
    };
    let dump = render_dump("economy", live);

    assert!(dump.contains("AGORA WALLET"));
    assert!(dump.contains("150"), "aggregate wallet credit balance should render");
    assert!(dump.contains("credits"));
    assert!(dump.contains("RECENT TRANSACTIONS"));
    assert!(dump.contains("marketplace sale"));
    assert!(dump.contains("zeus106"), "economy rows render the counterparty/actor column");
    assert!(dump.contains("+12 cr"));
    assert!(!dump.contains("247.83"), "old USDC mock balance must not leak");
    assert!(!dump.contains("advanced-codegen"), "prototype transaction fixture must not leak");
}

#[test]
fn batch1_empty_states_are_honest_not_mocked() {
    let agents_dump = render_dump("agents", AdvancedLive::default());
    let skills_dump = render_dump("skills", AdvancedLive::default());
    let economy_dump = render_dump("economy", AdvancedLive::default());

    assert!(agents_dump.contains("fetching from /v1/network/agents"));
    assert!(skills_dump.contains("fetching from /v1/skills"));
    assert!(economy_dump.contains("awaiting /v1/economy/wallets"));
    assert!(!agents_dump.contains("Hermes"));
    assert!(!skills_dump.contains("git-flow"));
    assert!(!economy_dump.contains("Hephaestus"));
}
