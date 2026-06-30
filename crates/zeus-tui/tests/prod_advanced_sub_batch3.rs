use ratatui::backend::TestBackend;
use ratatui::Terminal;
use zeus_tui::api::{
    CommunityResponse, DeployStatsResponse, DeployTargetResponse, DeploymentResponse,
    FileCounts, NodeResponse, SpawnResponse, TtsProviderResponse, TtsVoiceResponse,
    VectorStoreResponse,
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

fn write_evidence(name: &str, content: &str) {
    let dir = "/Users/mike/.zeus/workspace/advanced_sub_batch3";
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(format!("{dir}/{name}"), content).unwrap();
}

#[test]
fn deploy_subview_renders_live_targets_history_and_honest_empty_state() {
    let targets = vec![
        DeployTargetResponse {
            name: "api-prod".into(),
            provider: "fly".into(),
            environment: "prod".into(),
            url: "https://api.example.test".into(),
            active: true,
        },
        DeployTargetResponse {
            name: "worker-staging".into(),
            provider: "docker".into(),
            environment: "staging".into(),
            url: "".into(),
            active: false,
        },
    ];
    let history = vec![DeploymentResponse {
        target_name: "api-prod".into(),
        version: "v1.2.3".into(),
        status: "success".into(),
        trigger: "git push".into(),
    }];
    let stats = DeployStatsResponse {
        total_targets: 2,
        total_deployments: 9,
        live_deployments: 1,
        failed_deployments: 0,
    };

    let dump = render_dump(
        "deploy",
        AdvancedLive {
            deploy_targets: Some(&targets),
            deploy_history: Some(&history),
            deploy_stats: Some(&stats),
            ..Default::default()
        },
    );

    write_evidence("deploy_dump.log", &dump);

    assert!(dump.contains("TARGETS"));
    assert!(dump.contains("RECENT DEPLOYMENTS"));
    assert!(dump.contains("api-prod"));
    assert!(dump.contains("fly"));
    assert!(dump.contains("prod"));
    assert!(dump.contains("https://api.example.test"));
    assert!(dump.contains("worker-staging"));
    assert!(dump.contains("v1.2.3"));
    assert!(dump.contains("git push"));
    assert!(dump.contains("2 targets · 9 deployments · 1 live · 0 failed"));
    assert!(!dump.contains("vercel-web"));
    assert!(!dump.contains("zeus-daemon"));

    let empty = render_dump("deploy", AdvancedLive::default());
    assert!(empty.contains("fetching from /v1/deploy/targets"));
    assert!(empty.contains("fetching from /v1/deploy/history"));
    assert!(!empty.contains("vercel-web"));
}

#[test]
fn knowledge_graph_and_vectorstores_render_live_rows_without_fixtures() {
    let communities = vec![
        CommunityResponse {
            name: "core-memory".into(),
            entity_count: 42,
        },
        CommunityResponse {
            name: "ops".into(),
            entity_count: 7,
        },
    ];
    let kg_dump = render_dump(
        "knowledge-graph",
        AdvancedLive {
            communities: Some(&communities),
            ..Default::default()
        },
    );

    assert!(kg_dump.contains("COMMUNITY"));
    assert!(kg_dump.contains("NODES"));
    assert!(kg_dump.contains("EDGES"));
    assert!(kg_dump.contains("core-memory"));
    assert!(kg_dump.contains("ops"));
    assert!(kg_dump.contains("42"));
    assert!(kg_dump.contains("2 communities · 49 nodes"));
    assert!(!kg_dump.contains("Apollo"));
    assert!(!kg_dump.contains("fetching from /v1/memory/communities"));

    let stores = vec![
        VectorStoreResponse {
            name: "docs-index".into(),
            file_counts: FileCounts { total: 12 },
            status: "active".into(),
        },
        VectorStoreResponse {
            name: "tickets".into(),
            file_counts: FileCounts { total: 3 },
            status: "indexing".into(),
        },
    ];
    let vs_dump = render_dump(
        "vectorstores",
        AdvancedLive {
            vector_stores: Some(&stores),
            ..Default::default()
        },
    );

    write_evidence(
        "knowledge_vector_dump.log",
        &format!("{kg_dump}\n\n--- vectorstores ---\n{vs_dump}"),
    );

    assert!(vs_dump.contains("COLLECTIONS"));
    assert!(vs_dump.contains("SEMANTIC SEARCH"));
    assert!(vs_dump.contains("docs-index"));
    assert!(vs_dump.contains("tickets"));
    assert!(vs_dump.contains("12"));
    assert!(vs_dump.contains("active"));
    assert!(vs_dump.contains("indexing"));
    assert!(vs_dump.contains("12     files"));
    assert!(vs_dump.contains("3      files"));
    assert!(vs_dump.contains("no recent-query history"));
    assert!(!vs_dump.contains("mnemosyne-facts"));
    assert!(!vs_dump.contains("fetching from /v1/vector_stores"));
}

#[test]
fn nodecomms_and_spawner_render_live_rows_and_honest_feed_gaps() {
    let nodes = vec![
        NodeResponse {
            node_id: "zeus106".into(),
            host: "worker-a".into(),
            connected_at: "2026-06-28T00:00:00Z".into(),
            capabilities: vec!["tui".into()],
            rtt_ms: 18,
        },
        NodeResponse {
            node_id: "zeus-freebsd".into(),
            host: "freebsd".into(),
            connected_at: "2026-06-28T00:01:00Z".into(),
            capabilities: vec![],
            rtt_ms: 0,
        },
    ];
    let node_dump = render_dump(
        "nodecomms",
        AdvancedLive {
            nodes: Some(&nodes),
            ..Default::default()
        },
    );

    write_evidence("nodecomms_dump.log", &node_dump);

    assert!(node_dump.contains("FLEET LINKS"));
    assert!(node_dump.contains("RECENT MESSAGES"));
    assert!(node_dump.contains("zeus106"));
    assert!(node_dump.contains("zeus-freebsd"));
    assert!(node_dump.contains("18ms"));
    assert!(node_dump.contains("No message feed — awaiting /v1/nodes message endpoint"));
    assert!(!node_dump.contains("ping sweep"));
    assert!(!node_dump.contains("fetching from /v1/nodes"));

    let spawns = vec![SpawnResponse {
        agent_id: "zeus-spark".into(),
        task: "solana audit".into(),
        role: "auditor".into(),
        started_at: "2026-06-28T00:00:00Z".into(),
    }];
    let spawn_dump = render_dump(
        "spawner",
        AdvancedLive {
            spawns: Some(&spawns),
            ..Default::default()
        },
    );

    write_evidence(
        "node_spawner_dump.log",
        &format!("{node_dump}\n\n--- spawner ---\n{spawn_dump}"),
    );

    assert!(spawn_dump.contains("NAME"));
    assert!(spawn_dump.contains("TASK"));
    assert!(spawn_dump.contains("STATUS"));
    assert!(spawn_dump.contains("zeus-spark"));
    assert!(spawn_dump.contains("solana audit"));
    assert!(spawn_dump.contains("running"));
    assert!(spawn_dump.contains("CH"));
    assert!(spawn_dump.contains("1 subagents · 1 running"));
    assert!(!spawn_dump.contains("research-bot"));
    assert!(!spawn_dump.contains("awaiting /v1/spawner/active"));
}

#[test]
fn voice_subview_renders_live_tts_overlay_and_honest_unwired_cards() {
    let providers = vec![
        TtsProviderResponse {
            name: "OpenAI".into(),
            status: Some("ready".into()),
            description: Some("Realtime voices".into()),
        },
        TtsProviderResponse {
            name: "ElevenLabs".into(),
            status: Some("standby".into()),
            description: None,
        },
    ];
    let voices = vec![
        TtsVoiceResponse {
            provider: "OpenAI".into(),
            voice_id: "alloy".into(),
            name: "Alloy".into(),
            gender: None,
        },
        TtsVoiceResponse {
            provider: "OpenAI".into(),
            voice_id: "echo".into(),
            name: "Echo".into(),
            gender: None,
        },
    ];
    let dump = render_dump(
        "voice",
        AdvancedLive {
            tts_providers: Some(&providers),
            tts_voices: Some(&voices),
            ..Default::default()
        },
    );

    write_evidence("voice_dump.log", &dump);

    assert!(dump.contains("T T S   P R O V I D E R"));
    assert!(dump.contains("OpenAI"));
    assert!(dump.contains("2 voices · Alloy · ready"));
    assert!(dump.contains("S T T   P R O V I D E R"));
    assert!(dump.contains("Whisper · Groq"));
    assert!(dump.contains("T W I L I O"));
    assert!(dump.contains("not configured"));
    assert!(dump.contains("R E C O R D I N G S"));
    assert!(dump.contains("~/.zeus/voice/"));
    assert!(!dump.contains("fetching"));
}
