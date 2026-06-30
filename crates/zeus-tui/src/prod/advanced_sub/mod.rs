//! Advanced subviews — per-tab detail renderers.
//!
//! Dispatcher for the 13 ADVANCED_TABS subviews (JSX AdvancedSubview, line 1426).
//! Each tab gets its own file so the fleet can fill them conflict-free.
//! Scaffold stage: every stub draws just the header; seats flesh out the body.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Widget};

use crate::api::{
    AgentResponse, CommunityResponse, DeployStatsResponse, DeployTargetResponse,
    DeploymentResponse, EconomyTxResponse, EconomyWalletResponse, ExtensionResponse,
    McpServerResponse, NodeResponse, ProjectResponse, SkillResponse, SpawnResponse,
    TtsProviderResponse, TtsVoiceResponse, VectorStoreResponse, WorkflowResponse,
};

/// Live gateway data threaded into the advanced subviews.
///
/// One borrow carries every wired panel's fetched state; each field is `None`
/// until its poll-worker lands the first fetch, and the matching panel falls
/// back to its const placeholder while absent. Panels with no backend keep an
/// honest const stub and read nothing here. Grows one field per wired panel
/// (#185 incremental wiring — skills first).
#[derive(Default)]
pub struct AdvancedLive<'a> {
    /// Fleet agents (`GET /v1/network/agents`) — Advanced→Agents subview.
    pub agents: Option<&'a [AgentResponse]>,
    /// Installed skills (`GET /v1/skills`) — Advanced→Skills subview.
    pub skills: Option<&'a [SkillResponse]>,
    /// MCP servers (`GET /v1/mcp/servers`) — Advanced→MCP subview.
    pub mcp: Option<&'a [McpServerResponse]>,
    /// TTS providers (`GET /v1/tts/providers`) — Advanced→Voice subview.
    pub tts_providers: Option<&'a [TtsProviderResponse]>,
    /// TTS voices (`GET /v1/tts/voices`) — Advanced→Voice subview.
    pub tts_voices: Option<&'a [TtsVoiceResponse]>,
    /// Workflow instances (`GET /v1/workflows`) — Advanced→Canvas subview
    /// (chat→DAG execution drives the node graph).
    pub workflows: Option<&'a [WorkflowResponse]>,
    /// Installed extensions (`GET /v1/extensions`) — Advanced→Extensions subview.
    pub extensions: Option<&'a [ExtensionResponse]>,
    /// Configured projects (`GET /v1/projects`) — Advanced→Projects subview.
    pub projects: Option<&'a [ProjectResponse]>,
    /// Fleet nodes (`GET /v1/nodes`) — Advanced→NodeComms FLEET LINKS section.
    pub nodes: Option<&'a [NodeResponse]>,
    /// Active spawns (`GET /v1/spawner/active`) — Advanced→Spawner subview.
    pub spawns: Option<&'a [SpawnResponse]>,
    /// Vector stores (`GET /v1/vector_stores`) — Advanced→VectorStores subview.
    pub vector_stores: Option<&'a [VectorStoreResponse]>,
    /// KG communities (`GET /v1/memory/communities`) — Advanced→Knowledge-Graph subview.
    pub communities: Option<&'a [CommunityResponse]>,
    /// Deploy targets (`GET /v1/deploy/targets`) — Advanced→Deploy TARGETS.
    pub deploy_targets: Option<&'a [DeployTargetResponse]>,
    /// Recent deployments (`GET /v1/deploy/history`) — Advanced→Deploy history.
    pub deploy_history: Option<&'a [DeploymentResponse]>,
    /// Deploy fleet stats (`GET /v1/deploy/stats`) — Advanced→Deploy summary.
    pub deploy_stats: Option<&'a DeployStatsResponse>,
    /// Agent wallets (`GET /v1/economy/wallets`) — Advanced→Economy WALLET card.
    pub economy_wallets: Option<&'a [EconomyWalletResponse]>,
    /// Recent ledger txs (`GET /v1/economy/transactions`) — Advanced→Economy TXs.
    pub economy_txs: Option<&'a [EconomyTxResponse]>,
}

pub mod agents;
pub mod skills;
pub mod mcp;
pub mod projects;
pub mod canvas;
pub mod voice;
pub mod nodecomms;
pub mod vectorstores;
pub mod economy;
pub mod extensions;
pub mod knowledge_graph;
pub mod spawner;
pub mod deploy;

/// Render the subview body for the given advanced-tab `id` into `area`.
///
/// `id` matches `ADVANCED_TABS[i].id`. The detail-nav header (← Advanced /
/// glyph / name / desc) is drawn by `frame_prod`; this renders the body below it.
pub fn render(id: &str, area: Rect, buf: &mut Buffer, live: &AdvancedLive) {
    Clear.render(area, buf);
    match id {
        "agents" => agents::render(area, buf, live.agents),
        "skills" => skills::render(area, buf, live.skills),
        "mcp" => mcp::render(area, buf, live.mcp),
        "projects" => projects::render(area, buf, live.projects),
        "canvas" => canvas::render(area, buf, live.workflows),
        "voice" => voice::render(area, buf, live.tts_providers, live.tts_voices),
        "nodecomms" => nodecomms::render(area, buf, live.nodes),
        "vectorstores" => vectorstores::render(area, buf, live.vector_stores),
        "economy" => economy::render(area, buf, live.economy_wallets, live.economy_txs),
        "extensions" => extensions::render(area, buf, live.extensions),
        "knowledge-graph" => knowledge_graph::render(area, buf, live.communities),
        "spawner" => spawner::render(area, buf, live.spawns),
        "deploy" => deploy::render(
            area,
            buf,
            live.deploy_targets,
            live.deploy_history,
            live.deploy_stats,
        ),
        _ => {}
    }
}
