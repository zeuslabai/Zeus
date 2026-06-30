//! Zeus REST API Gateway
//!
//! Provides HTTP endpoints for programmatic access to Zeus:
//! - POST /v1/chat - Send message to agent
//! - GET /v1/sessions - List sessions
//! - POST /v1/sessions - Create new session
//! - GET /v1/tools - List available tools
//! - POST /v1/tools/:name - Execute a tool directly
//! - GET /v1/memory - Get workspace context
//! - POST /v1/memory/remember - Add to memory

pub mod api_key;
pub mod approvals;
pub mod chat_broadcast;
pub mod channels;
pub mod config_watcher;
pub mod cook_wiring;
pub mod cost_router;
mod db;
pub mod docs;
pub mod extensions;
pub mod ide_bridge;
pub mod handlers;
pub mod inbound;
pub mod mdns;
pub mod middleware;
pub mod node_client;
pub mod node_registry;
pub mod node_ws;
pub mod plan_store;
pub mod rate_limit;
pub mod registry;
mod routes;
pub mod security_headers;
pub mod session_maintenance;
mod upload_handlers;
pub mod uploads;
pub mod url_validator;
pub mod webhook_outbound;
pub mod webhook_triggers;
pub mod websocket;
pub mod ws_auth;

pub use api_key::ApiKeyValidator;
pub use approvals::{ApprovalQueue, ApprovalStatus, PendingApproval, start_expiry_task};
pub use channels::ChannelStore;
pub use config_watcher::{ConfigChangeEntry, ConfigHistory, start_config_watcher};
pub use cost_router::{CostRouter, CostSummary, ProviderCost, TaskTier};
pub use extensions::{ExtensionInfo, ExtensionStatus, ExtensionStore};
pub use inbound::{InboundConfig, start_inbound_loop};
pub use mdns::{MdnsDiscovery, ZeusPeer};
pub use node_registry::NodeRegistry;
pub use plan_store::PlanStore;
pub use handlers::discord_history::{CachedMessage, DiscordHistoryStore};
pub use handlers::slack_history::{CachedSlackMessage, SlackHistoryStore};
pub use handlers::task_store::{AgentTask, TaskStatus, TaskStore};
pub use rate_limit::{HttpRateLimiter, RateLimitConfig, RateLimitLayer};
pub use registry::AgentRegistry;
pub use routes::create_test_router;
pub use routes::{create_router, create_router_with_auth};
pub use security_headers::{SecurityHeadersConfig, SecurityHeadersLayer};
pub use session_maintenance::{MaintenanceConfig, MaintenanceMode, SessionMaintenance};
pub use uploads::UploadStore;
pub use webhook_outbound::WebhookManager;
pub use webhook_triggers::{
    TriggerAction, TriggerCondition, TriggerEngine, WebhookEvent, WebhookTrigger,
};
pub use websocket::{OfficeBroadcast, OfficeMessage, OrchestrationBroadcast, PlanBroadcast};

// Re-export Pantheon types for gateway/main access
pub use handlers::PantheonStore;
pub use handlers::{Room, RoomMember, RoomType};

use dashmap::DashMap;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use zeus_aegis::CredentialVault;
use zeus_agent::ToolRegistry;
use zeus_channels::ChannelManager;
use zeus_core::Config;
use zeus_economy::TokenLedger;
use zeus_agora::{Marketplace as AgoraMarketplace, MarketplaceConfig as AgoraMarketplaceConfig};
use zeus_memory::Workspace;
use zeus_mnemosyne::Mnemosyne;
use zeus_orchestra::Orchestra;
use zeus_orchestra::peer_review::{PeerReviewSystem, ReviewPolicy};
use zeus_orchestra::scheduler::Scheduler;
use zeus_orchestra::state::GlobalStateManager;
use zeus_prometheus::orchestrate::OrchestrationManager;
use zeus_prometheus::strategic::StrategicPlanner;
use zeus_session::BranchManager;
use zeus_extensions::ExtensionRegistry;
use zeus_talos::TalosRegistry;

/// API Server configuration
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub cors: bool,
    /// Bearer token for API authentication. If None, auth is disabled.
    pub auth_token: Option<String>,
    /// Allowed CORS origins. Defaults to localhost only.
    pub allowed_origins: Vec<String>,
    /// Rate limiting configuration. If None, rate limiting is disabled.
    pub rate_limit: Option<RateLimitConfig>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            cors: true,
            auth_token: None,
            allowed_origins: vec![
                "http://127.0.0.1".to_string(),
                "http://localhost".to_string(),
            ],
            rate_limit: Some(RateLimitConfig::default()),
        }
    }
}

/// Workflow node execution status
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowNodeStatus {
    pub node_id: String,
    pub status: String, // "pending", "running", "completed", "failed"
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub dependencies: Vec<String>,
}

/// Workflow execution state
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowState {
    pub workflow_id: String,
    pub status: String, // "pending", "running", "completed", "failed", "cancelled"
    pub message: String,
    pub nodes: Vec<WorkflowNodeStatus>,
    pub progress_percentage: f64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub total_nodes: usize,
    pub completed_nodes: usize,
    pub failed_nodes: usize,
}

impl WorkflowState {
    pub fn new(workflow_id: String, message: String, nodes: Vec<WorkflowNodeStatus>) -> Self {
        let total_nodes = nodes.len();
        Self {
            workflow_id,
            status: "pending".to_string(),
            message,
            nodes,
            progress_percentage: 0.0,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            total_nodes,
            completed_nodes: 0,
            failed_nodes: 0,
        }
    }

    pub fn update_progress(&mut self) {
        self.completed_nodes = self
            .nodes
            .iter()
            .filter(|n| n.status == "completed")
            .count();
        self.failed_nodes = self.nodes.iter().filter(|n| n.status == "failed").count();

        if self.total_nodes > 0 {
            self.progress_percentage =
                (self.completed_nodes as f64 / self.total_nodes as f64) * 100.0;
        }

        // Update overall status
        if self.failed_nodes > 0 {
            self.status = "failed".to_string();
            if self.completed_at.is_none() {
                self.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        } else if self.completed_nodes == self.total_nodes {
            self.status = "completed".to_string();
            if self.completed_at.is_none() {
                self.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        } else if self.completed_nodes > 0 {
            self.status = "running".to_string();
            if self.started_at.is_none() {
                self.started_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }
    }

    /// Mark the workflow as cancelled. No-op if already in a terminal state.
    pub fn cancel(&mut self) {
        if matches!(self.status.as_str(), "completed" | "failed" | "cancelled") {
            return;
        }
        self.status = "cancelled".to_string();
        if self.completed_at.is_none() {
            self.completed_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }
}

/// A message sent between agents via the gateway network.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentMessage {
    /// Unique message ID (UUID)
    pub id: String,
    /// Sender agent name (e.g. "@zeus_bot")
    pub from_agent: String,
    /// Sender host IP or hostname (e.g. "192.168.1.112")
    pub from_host: String,
    /// Target agent name (None = broadcast)
    pub to_agent: Option<String>,
    /// Message content
    pub content: String,
    /// When the message was created (RFC 3339)
    pub timestamp: String,
    /// Whether the message was delivered to the local tmux session
    pub delivered: bool,
}

/// In-memory ring buffer of recent agent messages (capped at 200).
pub struct MessageInbox {
    pub messages: VecDeque<AgentMessage>,
    pub capacity: usize,
}

impl MessageInbox {
    pub fn new(capacity: usize) -> Self {
        Self {
            messages: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a message, evicting the oldest if at capacity.
    pub fn push(&mut self, msg: AgentMessage) {
        if self.messages.len() >= self.capacity {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }
}

impl Default for MessageInbox {
    fn default() -> Self {
        Self::new(200)
    }
}

/// Shared application state
/// S93: Agent discovered from channel messages (Discord bots seen in chat)
/// S94: Now tracks ALL senders — bots and humans
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChannelAgent {
    pub id: String,
    pub name: String,
    pub last_seen: i64,
    pub last_message: String,
    pub status: String, // active, idle, offline
    pub agent_type: String, // "bot" or "human"
}

pub struct AppState {
    pub config: Config,
    pub workspace: Workspace,
    pub tools: ToolRegistry,
    pub mnemosyne: Option<Arc<Mnemosyne>>,
    /// Default gateway agent (shared session for TUI + channels)
    pub default_agent: Option<Arc<tokio::sync::RwLock<zeus_agent::Agent>>>,
    /// Recent config change history (ring buffer, last 10)
    pub config_history: ConfigHistory,
    /// Exec approval queue for tool confirmation
    pub approvals: ApprovalQueue,
    /// Chat token broadcast — subscribers receive every token emitted
    /// by the agent's LLM stream (TUI, WebSocket clients, etc.)
    pub chat_broadcast: chat_broadcast::ChatBroadcast,
    /// Agent registry for multi-agent routing
    pub agent_registry: AgentRegistry,
    /// Channel config store (channels.json)
    pub channel_store: ChannelStore,
    /// Session branch manager (branches.json)
    pub branch_manager: BranchManager,
    /// Cost-based LLM routing and budget tracking
    pub cost_router: CostRouter,
    /// Outbound webhook manager for event notifications
    pub webhook_manager: WebhookManager,
    /// Cron/scheduler for recurring tasks
    pub scheduler: OnceLock<Scheduler>,
    /// Agora — skill listing marketplace (zeus-agora)
    pub agora: AgoraMarketplace,
    /// Global agent state manager (for peer review + orchestration)
    pub global_state: OnceLock<Arc<GlobalStateManager>>,
    /// Peer review system for agent work verification
    pub peer_review: OnceLock<PeerReviewSystem>,
    /// Strategic planner for DAG-based task analysis
    pub strategic_planner: OnceLock<StrategicPlanner>,
    /// Broadcast channel for Prometheus plan execution updates (WebSocket streaming)
    pub plan_broadcast: PlanBroadcast,
    /// S63: Broadcast channel for office message stream
    pub office_broadcast: OfficeBroadcast,
    /// S93: Channel presence — tracks bots seen in Discord channels
    /// Maps bot_id -> (display_name, last_seen_unix, last_task)
    pub channel_presence: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, ChannelAgent>>>,
    /// Orchestra for team management and delegation
    pub orchestra: OnceLock<Orchestra>,
    /// Token economy ledger (SQLite-backed)
    pub ledger: TokenLedger,
    /// Pending OAuth PKCE flows: state -> (verifier, created_at)
    pub oauth_pending: HashMap<String, (String, std::time::Instant)>,
    /// File upload store
    pub upload_store: UploadStore,
    /// S98: Serialization lock for agent.run() — prevents concurrent calls from
    /// TUI/API that would corrupt the session. Callers acquire this before agent.run().
    pub agent_run_lock: Arc<tokio::sync::Mutex<()>>,
    /// Unified agent inbox — preferred over agent_run_lock for new call sites.
    /// Set by gateway after inbox is created; None until gateway wires it in.
    pub agent_inbox: Option<zeus_core::inbox::InboxSender>,
    /// Extension store (DashMap + file persistence)
    pub extension_store: ExtensionStore,
    /// Extension registry — Deno subprocess lifecycle manager.
    /// Registered and started extensions correspond to enabled entries in `extension_store`.
    pub extension_registry: Arc<ExtensionRegistry>,
    /// Orchestration engine for autonomous project workflows
    pub orchestration: OnceLock<OrchestrationManager>,
    /// Broadcast channel for orchestration progress events (WebSocket streaming)
    pub orchestration_broadcast: OrchestrationBroadcast,
    /// Workflow state tracking for real-time status queries
    pub workflow_states: Arc<DashMap<String, WorkflowState>>,
    /// Key rotation state for auth middleware
    pub key_rotation: Option<middleware::KeyRotation>,
    /// Shared channel manager for inbound message routing
    pub channel_manager: Arc<ChannelManager>,
    /// Credential vault for skill API key injection (never exposed to LLM)
    pub credential_vault: Arc<CredentialVault>,
    /// Webhook trigger-action automation engine
    pub trigger_engine: TriggerEngine,
    /// mDNS discovery for local network Zeus peer detection
    /// DM pairing manager for channel auth
    pub pairing_manager: zeus_channels::PairingManager,
    pub mdns_discovery: OnceLock<MdnsDiscovery>,
    /// Ed25519 key pair for WebSocket v3 auth (None = auth disabled)
    pub ws_keypair: Option<Arc<ws_auth::WsKeyPair>>,
    /// Prometheus cron scheduler for proactive brain tasks
    pub cron_scheduler: OnceLock<Arc<zeus_prometheus::CronScheduler>>,
    /// Shared HTTP client for connection pooling across handlers
    pub http_client: reqwest::Client,
    /// Delegation store (in-memory DashMap for agent task delegations)
    pub delegations: Arc<DashMap<String, serde_json::Value>>,
    /// Prometheus plan store (in-memory ring buffer, capped at 100)
    pub plan_store: PlanStore,
    /// Security permissions persistence path
    pub permissions_path: std::path::PathBuf,
    /// Custom sandbox policies (user-created, persisted to sandbox_policies.json)
    pub sandbox_policies: Arc<DashMap<String, serde_json::Value>>,
    /// Sandbox policies persistence path
    pub sandbox_policies_path: std::path::PathBuf,
    /// Inter-agent message inbox (ring buffer, last 200 messages)
    pub message_inbox: Arc<tokio::sync::Mutex<MessageInbox>>,
    /// Node registry for connected WebSocket fleet agents (hub-spoke)
    pub node_registry: Arc<NodeRegistry>,
    /// Pantheon multi-agent mission store + broadcast
    pub pantheon: handlers::PantheonStore,
    /// Agora marketplace persistence (skill listings, trades, token ledger, ratings)
    pub marketplace_store: handlers::MarketplaceStore,
    /// One-Click Deploy persistence (targets, deployments, rollback snapshots)
    pub deploy_store: handlers::DeployStore,
    /// Agent Studio persistence (sessions, puppet actions, artifacts)
    pub studio_store: handlers::StudioStore,
    /// Vector store registry persistence (SQLite-backed, survives restarts)
    pub vector_store_db: handlers::VectorStoreDb,
    /// Pantheon orchestrator — Prometheus-wired team assembly + task lifecycle
    pub pantheon_orchestrator: OnceLock<Arc<zeus_orchestra::pantheon::PantheonOrchestrator>>,
    /// Agent Director for Studio puppet sessions (UI automation orchestrator)
    pub agent_director: Arc<zeus_prometheus::AgentDirector>,
    /// Studio → War Room event broadcast (live action streaming to observers)
    pub studio_broadcast: crate::websocket::StudioBroadcast,
    /// Real tool executor for Pantheon mission execution (set by gateway at startup)
    pub tool_executor: Option<Arc<dyn zeus_prometheus::ToolExecutor>>,
    /// Cancellation flags for running missions (mission_id → cancelled flag)
    pub mission_cancels: Arc<DashMap<String, Arc<AtomicBool>>>,
    /// Nous cognitive engine (intent, reasoning, learning, meta-cognition)
    pub nous: Option<Arc<zeus_nous::Nous>>,
    /// Proactive agent spawner (predictive spawning recommendations)
    pub spawner: Arc<std::sync::Mutex<zeus_prometheus::ProactiveSpawner>>,
    /// Per-agent compute quota tracker. Agents are registered here on spawn
    /// so LLM calls and tool invocations can be budget-checked before execution.
    /// Unregistered agents are allowed through (see `QuotaCheck::Allowed`), so
    /// any code path that hasn't wired enforcement yet fails open, not closed.
    pub compute_provisioner: Arc<tokio::sync::RwLock<zeus_prometheus::ComputeProvisioner>>,
    /// Conway-style agent replication manager (cost-aware spawning with lineage)
    pub replication_manager: Arc<tokio::sync::RwLock<zeus_prometheus::ReplicationManager>>,
    /// Fleet provisioning job tracker (S10-7)
    pub provision_jobs: Option<handlers::fleet_provisioner::ProvisionJobs>,
    /// Outcome template registry (S12-7) — built-ins + user-defined templates
    pub template_registry: zeus_templates::TemplateRegistry,
    /// Feedback loop for strategy learning from execution outcomes
    pub feedback: Arc<zeus_prometheus::FeedbackLoop>,
    /// TOTP 2FA persistence for blog admin (S23)
    pub totp_store: handlers::TotpStore,
    /// Agent task store for checkpoint/resume across restarts (S52-T1)
    pub task_store: handlers::TaskStore,
    /// Discord message history cache for context across restarts (S52-T2)
    pub discord_history: handlers::DiscordHistoryStore,
    /// Slack message history cache for context across restarts (S55-T11)
    pub slack_history: handlers::SlackHistoryStore,
    /// S65: Agent zone assignments (agent_id -> zone name)
    pub agent_zones: Arc<DashMap<String, String>>,
    /// S65: Broadcast channel for agent status/zone change events (SSE)
    pub agent_status_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
}

/// Construct the Agora marketplace, opt-in to on-chain SPL settlement when
/// the environment is configured for it.
///
/// When all five `ZEUS_SOLANA_*` env vars are present and valid, wire
/// `zeus_solana::SolanaSettlement` as the marketplace settlement backend so
/// skill trades settle against a real SPL token on Solana. When any are
/// missing or malformed, fall back to the default in-memory settlement
/// (previous behaviour) and log a single startup line explaining why.
///
/// Required env vars:
///
/// | Var | Example | Notes |
/// |---|---|---|
/// | `ZEUS_SOLANA_RPC_URL` | `https://api.devnet.solana.com` | Solana JSON-RPC endpoint |
/// | `ZEUS_SOLANA_KEYPAIR_PATH` | `~/.zeus/solana/sender.json` | 64-byte keypair JSON (solana-keygen format) |
/// | `ZEUS_SOLANA_MINT` | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` | SPL mint, base58 |
/// | `ZEUS_SOLANA_DECIMALS` | `6` | Token decimal places |
/// | `ZEUS_SOLANA_BASE_UNITS_PER_CREDIT` | `1` | Conversion from marketplace credits to base units |
fn build_agora_marketplace() -> AgoraMarketplace {
    let rpc_url = match std::env::var("ZEUS_SOLANA_RPC_URL") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            tracing::info!("Agora: using in-memory settlement (ZEUS_SOLANA_RPC_URL not set)");
            return AgoraMarketplace::with_defaults();
        }
    };
    let keypair_path = match std::env::var("ZEUS_SOLANA_KEYPAIR_PATH") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            tracing::warn!(
                "Agora: ZEUS_SOLANA_RPC_URL set but ZEUS_SOLANA_KEYPAIR_PATH missing — \
                 falling back to in-memory settlement"
            );
            return AgoraMarketplace::with_defaults();
        }
    };
    let mint = match std::env::var("ZEUS_SOLANA_MINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            tracing::warn!("Agora: ZEUS_SOLANA_MINT missing — falling back to in-memory");
            return AgoraMarketplace::with_defaults();
        }
    };
    let decimals: u8 = match std::env::var("ZEUS_SOLANA_DECIMALS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(v) => v,
        None => {
            tracing::warn!(
                "Agora: ZEUS_SOLANA_DECIMALS missing or unparseable — falling back to in-memory"
            );
            return AgoraMarketplace::with_defaults();
        }
    };
    let base_units_per_credit: u64 = match std::env::var("ZEUS_SOLANA_BASE_UNITS_PER_CREDIT")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(v) => v,
        None => {
            tracing::warn!(
                "Agora: ZEUS_SOLANA_BASE_UNITS_PER_CREDIT missing or unparseable — \
                 falling back to in-memory"
            );
            return AgoraMarketplace::with_defaults();
        }
    };

    // solana-keygen writes a JSON array of 64 u8s; accept that shape.
    // Expand a leading ~ without pulling in shellexpand as a new dependency.
    let expanded: String = if let Some(rest) = keypair_path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest).to_string_lossy().into_owned())
            .unwrap_or(keypair_path.clone())
    } else if keypair_path == "~" {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or(keypair_path.clone())
    } else {
        keypair_path.clone()
    };
    let keypair_bytes: Vec<u8> = match std::fs::read_to_string(&expanded)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<u8>>(&s).ok())
    {
        Some(bytes) if bytes.len() == 64 => bytes,
        Some(bytes) => {
            tracing::error!(
                "Agora: keypair at {} has length {} (expected 64) — falling back to in-memory",
                expanded,
                bytes.len()
            );
            return AgoraMarketplace::with_defaults();
        }
        None => {
            tracing::error!(
                "Agora: failed to read or parse keypair at {} — falling back to in-memory",
                expanded
            );
            return AgoraMarketplace::with_defaults();
        }
    };

    tracing::info!(
        %rpc_url,
        %mint,
        decimals,
        base_units_per_credit,
        "Agora: wiring on-chain Solana SPL settlement"
    );

    AgoraMarketplace::with_settlement(
        AgoraMarketplaceConfig::default(),
        Box::new(zeus_solana::SolanaSettlement::new(
            rpc_url,
            keypair_bytes,
            mint,
            decimals,
            base_units_per_credit,
        )),
    )
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, String> {
        let workspace = Workspace::from_config(&config);
        let mut tools = if config.talos.is_some() {
            let talos = TalosRegistry::with_defaults();
            let browser = zeus_browser::BrowserRegistry::default();
            tracing::info!(
                "Gateway initialized: {} Talos + {} Browser tools",
                talos.len(),
                browser.len()
            );
            ToolRegistry::with_talos_and_browser(talos, browser)
        } else {
            ToolRegistry::with_defaults()
        };
        let (confirm_tools, timeout) = if let Some(ref aegis) = config.aegis {
            (
                aegis.require_confirmation_for.clone(),
                aegis.approval_timeout_secs,
            )
        } else {
            (Vec::new(), 1800)
        };
        let approvals = ApprovalQueue::new(confirm_tools, timeout);
        let agent_registry = AgentRegistry::new(config.clone());
        let channel_store = ChannelStore::new(&config.workspace);
        let branch_manager = BranchManager::new(&config.sessions);
        let cost_router = CostRouter::with_defaults(Some(100.0));
        let webhook_manager = WebhookManager::new(&config.workspace);
        let trigger_engine = TriggerEngine::new(&config.workspace);
        let plan_broadcast = PlanBroadcast::new(256);
        let office_broadcast = OfficeBroadcast::new(256);
        let ledger = TokenLedger::new(config.workspace.join("economy.db"))
            .or_else(|e| {
                tracing::warn!("Failed to init economy ledger at workspace: {e}, trying temp dir");
                TokenLedger::new(std::env::temp_dir().join("zeus-economy-fallback.db"))
            })
            .unwrap_or_else(|e| {
                tracing::error!(
                    "Failed to init economy ledger even in temp dir: {e}, using ephemeral"
                );
                TokenLedger::new(
                    std::env::temp_dir().join(format!("zeus-economy-{}.db", std::process::id())),
                )
                .expect("ephemeral economy ledger: all fallbacks failed")
            });
        // Seed default agent with initial tokens (idempotent — no-op if already has balance)
        if ledger.balance("default").unwrap_or(0) == 0 {
            let _ = ledger.mint(
                "default",
                10_000,
                zeus_economy::TransactionReason::SystemGrant,
                "initial system grant",
            );
        }

        let permissions_path = config.workspace.join("permissions.json");
        let sandbox_policies_path = config.workspace.join("sandbox_policies.json");
        let sandbox_policies = Arc::new(DashMap::new());
        // Load persisted sandbox policies from disk (sync, runs once at startup)
        if sandbox_policies_path.exists()
            && let Ok(content) = std::fs::read_to_string(&sandbox_policies_path)
            && let Ok(policies) = serde_json::from_str::<Vec<serde_json::Value>>(&content)
        {
            for p in policies {
                if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                    sandbox_policies.insert(id.to_string(), p);
                }
            }
        }
        let extension_store = ExtensionStore::new(&config.workspace);
        let extension_registry = Arc::new(ExtensionRegistry::new());
        let channel_manager = Arc::new(ChannelManager::new(256));
        // Wire ChannelManager into ToolRegistry so platform channels (Discord, Slack, etc.)
        // are reachable from the HTTP /v1/tools execution path (fixes "Unknown channel 'discord'" on FreeBSD).
        tools.set_channels(channel_manager.clone());
        let pairing_path = config.workspace.join("pairings.json");
        let pairing_mgr = zeus_channels::PairingManager::load(pairing_path.clone())
            .unwrap_or_else(|_| zeus_channels::PairingManager::new(pairing_path));
        let upload_store = UploadStore::new(&config.workspace).unwrap_or_else(|e| {
            tracing::warn!("Failed to init upload store: {e}, using temp dir fallback");
            UploadStore::new(
                &std::env::temp_dir().join(format!("zeus-uploads-{}", std::process::id())),
            )
            .unwrap_or_else(|e2| {
                tracing::error!("Upload store fallback also failed: {e2}, using ephemeral");
                UploadStore::new(
                    &std::env::temp_dir()
                        .join(format!("zeus-uploads-ephemeral-{}", std::process::id())),
                )
                .expect("ephemeral upload store: all fallbacks failed")
            })
        });
        // Load or generate Ed25519 key pair for WebSocket v3 auth
        let ws_keypair = match config.ws_auth.as_ref() {
            Some(ws_cfg) if ws_cfg.enabled => match ws_auth::load_or_generate(&ws_cfg.key_path) {
                Ok(kp) => {
                    tracing::info!(
                        "WebSocket v3 auth enabled, pubkey={}",
                        ws_auth::public_key_hex(&kp)
                    );
                    Some(Arc::new(kp))
                }
                Err(e) => {
                    tracing::error!("Failed to load WS auth key: {e}, auth disabled");
                    None
                }
            },
            _ => None,
        };
        let pantheon_store = handlers::PantheonStore::new(&config.workspace.join("pantheon.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init Pantheon SQLite at workspace: {e}, using in-memory");
                handlers::PantheonStore::in_memory().expect("in-memory store should work")
            });
        let marketplace_store =
            handlers::MarketplaceStore::new(&config.workspace.join("marketplace.db"))
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to init Marketplace SQLite: {e}, using in-memory");
                    handlers::MarketplaceStore::in_memory()
                        .expect("in-memory marketplace store should work")
                });
        let deploy_store = handlers::DeployStore::new(&config.workspace.join("deploy.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init Deploy SQLite: {e}, using in-memory");
                handlers::DeployStore::in_memory().expect("in-memory deploy store should work")
            });
        let studio_store = handlers::StudioStore::new(&config.workspace.join("studio.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init Studio SQLite: {e}, using in-memory");
                handlers::StudioStore::in_memory().expect("in-memory studio store should work")
            });
        let vector_store_db =
            handlers::VectorStoreDb::new(&config.workspace.join("vector_stores.db"))
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to init vector_stores.db: {e}, using in-memory");
                    handlers::VectorStoreDb::in_memory()
                        .expect("in-memory vector store DB should work")
                });
        let credential_vault = Arc::new(CredentialVault::new(
            config.credentials.clone(),
            config.workspace.clone(),
        ));
        let totp_store = handlers::TotpStore::new(&config.workspace.join("totp.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init TOTP SQLite: {e}, using in-memory");
                handlers::TotpStore::in_memory().expect("in-memory TOTP store should work")
            });
        let task_store = handlers::TaskStore::new(&config.workspace.join("tasks.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init task SQLite: {e}, using in-memory");
                handlers::TaskStore::in_memory().expect("in-memory task store should work")
            });
        let discord_history = handlers::DiscordHistoryStore::new(&config.workspace.join("discord_history.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init Discord history SQLite: {e}, using in-memory");
                handlers::DiscordHistoryStore::in_memory().expect("in-memory discord history should work")
            });
        let slack_history = handlers::SlackHistoryStore::new(&config.workspace.join("slack_history.db"))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init Slack history SQLite: {e}, using in-memory");
                handlers::SlackHistoryStore::in_memory().expect("in-memory slack history should work")
            });
        // Warn loudly if workspace path looks like a temp dir (config corruption indicator)
        let ws_str = config.workspace.to_string_lossy();
        if ws_str.contains("/var/folders/") || ws_str.contains("/tmp/") {
            tracing::error!(
                "⚠ WORKSPACE PATH LOOKS CORRUPT: {} — agent will load default personality! \
                 Run config-guard.sh --fix to restore from backup.",
                ws_str
            );
        }
        tracing::info!("AppState initialized (subsystems deferred for lazy init)");
        Ok(Self {
            config,
            workspace,
            tools,
            mnemosyne: None,
            default_agent: None,
            config_history: ConfigHistory::default(),
            approvals,
            chat_broadcast: chat_broadcast::ChatBroadcast::default(),
            agent_registry,
            channel_store,
            branch_manager,
            cost_router,
            webhook_manager,
            scheduler: OnceLock::new(),
            agora: build_agora_marketplace(),
            global_state: OnceLock::new(),
            peer_review: OnceLock::new(),
            strategic_planner: OnceLock::new(),
            plan_broadcast,
            office_broadcast,
            channel_presence: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            orchestra: OnceLock::new(),
            ledger,
            oauth_pending: HashMap::new(),
            upload_store,
            agent_run_lock: Arc::new(tokio::sync::Mutex::new(())),
            agent_inbox: None,
            extension_store,
            extension_registry,
            orchestration: OnceLock::new(),
            orchestration_broadcast: OrchestrationBroadcast::new(256),
            workflow_states: Arc::new(DashMap::new()),
            key_rotation: None,
            channel_manager,
            trigger_engine,
            pairing_manager: pairing_mgr,
            mdns_discovery: OnceLock::new(),
            ws_keypair,
            cron_scheduler: OnceLock::new(),
            http_client: reqwest::Client::new(),
            delegations: Arc::new(DashMap::new()),
            plan_store: PlanStore::default(),
            permissions_path,
            sandbox_policies,
            sandbox_policies_path,
            message_inbox: Arc::new(tokio::sync::Mutex::new(MessageInbox::default())),
            node_registry: Arc::new(NodeRegistry::new()),
            pantheon: pantheon_store,
            marketplace_store,
            deploy_store,
            studio_store,
            vector_store_db,
            pantheon_orchestrator: OnceLock::new(),
            agent_director: Arc::new(zeus_prometheus::AgentDirector::new()),
            studio_broadcast: crate::websocket::StudioBroadcast::new(256),
            credential_vault,
            tool_executor: None,
            mission_cancels: Arc::new(DashMap::new()),
            nous: None,
            spawner: Arc::new(std::sync::Mutex::new(
                zeus_prometheus::ProactiveSpawner::default(),
            )),
            compute_provisioner: Arc::new(tokio::sync::RwLock::new(
                zeus_prometheus::ComputeProvisioner::new(),
            )),
            replication_manager: Arc::new(tokio::sync::RwLock::new(
                zeus_prometheus::ReplicationManager::new(zeus_prometheus::ReplicationConfig::default()),
            )),
            provision_jobs: Some(handlers::fleet_provisioner::new_provision_jobs()),
            template_registry: {
                let path = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zeus")
                    .join("templates");
                zeus_templates::TemplateRegistry::load(path)
            },
            feedback: Arc::new(zeus_prometheus::FeedbackLoop::new()),
            totp_store,
            task_store,
            discord_history,
            slack_history,
            agent_zones: Arc::new(DashMap::new()),
            agent_status_tx: tokio::sync::broadcast::channel(256).0,
        })
    }

    /// Register an agent with the compute provisioner so its LLM/tool budget
    /// is tracked. Called from every spawn path (static `spawn_agent` handler
    /// and all `spawn_dynamic` call sites). `priority` scales the agent's
    /// quota (see `ResourceQuota::scale`); pass `1.0` for dynamic/ephemeral
    /// agents that don't carry a configured priority.
    pub async fn register_agent_compute(&self, agent_id: &str, priority: f64) {
        self.compute_provisioner
            .write()
            .await
            .register_agent(agent_id, priority);
    }

    /// Drop an agent's quota tracking. Must be called by every `unregister`
    /// path that removes an agent from `agent_registry`, otherwise the
    /// provisioner's map grows unbounded as agents churn.
    pub async fn deregister_agent_compute(&self, agent_id: &str) {
        self.compute_provisioner
            .write()
            .await
            .deregister_agent(agent_id);
    }

    /// Bootstrap fleet agents, recover stale missions, and other startup tasks.
    /// Call this after creating AppState and wrapping in Arc<RwLock<>>.
    pub async fn boot(state: &SharedState) {
        // Export DeploymentConfig service URLs as ZEUS_* env vars so all crates
        // and child processes inherit correct URLs without reading config.toml.
        {
            let s = state.read().await;
            let deployment = s.config.deployment.clone().unwrap_or_default();
            deployment.export_env_vars();
            // Also export Ollama URL for crates that read OLLAMA_HOST
            // SAFETY: Called at single-threaded gateway startup.
            unsafe {
                std::env::set_var("OLLAMA_HOST", &s.config.ollama.url);

                // S70-A1: Export ALL credentials from config.toml to env vars
                // Bridge pattern: config.toml is SSoT, env vars for crates that
                // haven't been migrated yet (zeus-llm, zeus-channels, etc.)
                for (key, value) in s.config.credentials.iter() {
                    if !value.is_empty() {
                        std::env::set_var(key, value);
                    }
                }

                // Export channel credentials
                if let Some(ref channels) = s.config.channels {
                    if let Some(ref dc) = channels.discord {
                        if !dc.token.is_empty() {
                            std::env::set_var("DISCORD_BOT_TOKEN", &dc.token);
                        }
                    }
                    if let Some(ref tg) = channels.telegram {
                        if let Some(ref bt) = tg.bot_token {
                            if !bt.is_empty() {
                                std::env::set_var("TELEGRAM_BOT_TOKEN", bt);
                            }
                        }
                    }
                }

                // Azure/AWS credentials come through the credentials map above
            }
            tracing::info!(
                "Service URLs + credentials exported to environment from config.toml"
            );
        }

        // Initialize Nous cognitive engine (async — can't run in sync new())
        {
            let mnemosyne = state.read().await.mnemosyne.clone();
            let nous = match mnemosyne {
                Some(ref m) => zeus_nous::Nous::with_mnemosyne(m.clone()).await,
                None => zeus_nous::Nous::new().await,
            };
            match nous {
                Ok(n) => {
                    state.write().await.nous = Some(std::sync::Arc::new(n));
                    tracing::info!("Nous cognitive engine initialized");
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize Nous: {e} — intelligence endpoints disabled"
                    );
                }
            }
        }

        // Start all enabled extensions from the persisted extension store.
        // Each enabled extension is registered with ExtensionRegistry and its
        // Deno subprocess is spawned. Failures are logged as warnings — a missing
        // Deno binary or bad extension path should not block gateway startup.
        {
            let s = state.read().await;
            let registry = s.extension_registry.clone();
            let enabled: Vec<_> = s.extension_store
                .list()
                .into_iter()
                .filter(|e| e.enabled)
                .collect();
            drop(s);
            if !enabled.is_empty() {
                tracing::info!("Starting {} enabled extension(s) at boot", enabled.len());
                tokio::spawn(async move {
                    for info in enabled {
                        let ext = crate::extensions::info_to_registry_extension(&info);
                        // register() is idempotent — no-op if already in registry
                        let _ = registry.register(ext).await;
                        match registry.start(&info.id).await {
                            Ok(()) => tracing::info!(
                                "Extension '{}' started (id={})", info.name, info.id
                            ),
                            Err(e) => tracing::warn!(
                                "Extension '{}' failed to start: {}", info.name, e
                            ),
                        }
                    }
                });
            }
        }

        handlers::fleet::boot_fleet_agents(state).await;

        // S69: Seed channel store from config.toml channels
        {
            let s = state.read().await;
            let config = &s.config;
            if let Some(ref channels) = config.channels {
                let mut seeded = 0usize;
                if let Some(ref dc) = channels.discord {
                    let ch = crate::channels::Channel {
                        id: "discord-default".to_string(),
                        channel_type: crate::channels::ChannelType::Discord,
                        name: "Discord".to_string(),
                        config: std::collections::HashMap::new(),
                        enabled: !dc.token.is_empty(),
                        created_at: chrono::Utc::now(),
                        last_message_at: None,
                    };
                    let _ = s.channel_store.add(ch).await;
                    for (acct_name, _acct) in &dc.accounts {
                        let ch = crate::channels::Channel {
                            id: format!("discord-{}", acct_name),
                            channel_type: crate::channels::ChannelType::Discord,
                            name: format!("Discord ({})", acct_name),
                            config: std::collections::HashMap::new(),
                            enabled: true,
                            created_at: chrono::Utc::now(),
                            last_message_at: None,
                        };
                        let _ = s.channel_store.add(ch).await;
                        seeded += 1;
                    }
                    seeded += 1;
                }
                if let Some(ref _tg) = channels.telegram {
                    let ch = crate::channels::Channel {
                        id: "telegram-default".to_string(),
                        channel_type: crate::channels::ChannelType::Telegram,
                        name: "Telegram".to_string(),
                        config: std::collections::HashMap::new(),
                        enabled: true,
                        created_at: chrono::Utc::now(),
                        last_message_at: None,
                    };
                    let _ = s.channel_store.add(ch).await;
                    seeded += 1;
                }
                if let Some(ref _sl) = channels.slack {
                    let ch = crate::channels::Channel {
                        id: "slack-default".to_string(),
                        channel_type: crate::channels::ChannelType::Slack,
                        name: "Slack".to_string(),
                        config: std::collections::HashMap::new(),
                        enabled: true,
                        created_at: chrono::Utc::now(),
                        last_message_at: None,
                    };
                    let _ = s.channel_store.add(ch).await;
                    seeded += 1;
                }
                if seeded > 0 {
                    tracing::info!("Seeded {} channel(s) from config.toml", seeded);
                }
            }
        }

        // S66-P4B: Restore persisted agent zone assignments from SQLite
        {
            let s = state.read().await;
            let zones = s.pantheon.load_agent_zones().await;
            if !zones.is_empty() {
                for (agent_id, zone) in &zones {
                    s.agent_zones.insert(agent_id.clone(), zone.clone());
                }
                tracing::info!("Restored {} agent zone assignment(s) from SQLite", zones.len());
            }
        }

        // Recover missions that were executing when the gateway last shut down.
        // Missions stale for >5 minutes are marked Failed with a recovery activity entry.
        let store = state.read().await.pantheon.clone();
        let recovered = store
            .recover_stale_missions(std::time::Duration::from_secs(300))
            .await;
        if !recovered.is_empty() {
            tracing::info!(
                "Recovered {} stale missions on startup: {:?}",
                recovered.len(),
                recovered
            );
        }

        // Mark agents with stale heartbeats as Offline
        handlers::fleet::cleanup_stale_agents(state, std::time::Duration::from_secs(600)).await;

        // Start mission timeout watchdog (checks every 60s for timed-out missions)
        Self::spawn_mission_watchdog(state).await;

        // Start announcement hook — broadcasts key Pantheon events to all channels
        {
            let s = state.read().await;
            let cm = s.channel_manager.clone();
            handlers::pantheon::spawn_announcement_hook(&s.pantheon, cm);
            drop(s);
            tracing::info!("Announcement hook started");
        }

        // Sync builtin skills into marketplace SQLite store
        let marketplace_store = state.read().await.marketplace_store.clone();
        handlers::sync_builtins_to_marketplace(&marketplace_store).await;

        // Spawn Nous confidence decay loop (6h interval)
        if let Some(nous) = state.read().await.nous.clone() {
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
                loop {
                    interval.tick().await;
                    let n = nous.run_decay().await;
                    tracing::debug!("Nous confidence decay: {n} lesson(s) updated");
                }
            });
            tracing::info!("Nous confidence decay loop started (6h interval)");
        }

        // Spawn ContentQueueDrain loop (5-minute interval)
        // Processes ready media jobs enqueued via ContentQueue and runs them
        // through the FFmpeg + upload pipeline.
        {
            let db_path = state
                .read()
                .await
                .workspace
                .root()
                .join("content_queue.db")
                .to_string_lossy()
                .into_owned();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(5 * 60));
                loop {
                    interval.tick().await;
                    let (ok, msg) =
                        zeus_prometheus::content_queue_drain::execute_content_queue_drain(
                            &db_path,
                        )
                        .await;
                    if ok {
                        tracing::debug!("ContentQueueDrain: {msg}");
                    } else {
                        tracing::warn!("ContentQueueDrain: {msg}");
                    }
                }
            });
            tracing::info!("ContentQueueDrain loop started (5-min interval)");
        }
    }

    /// Spawn a background watchdog that periodically checks for timed-out missions
    /// and stale agents. Runs every 60 seconds.
    async fn spawn_mission_watchdog(state: &SharedState) {
        let store = state.read().await.pantheon.clone();
        let default_timeout = std::time::Duration::from_secs(1800); // 30 minutes

        // Use the existing PantheonStore background task
        let _handle = handlers::PantheonStore::start_timeout_check_task(store, default_timeout);

        tracing::info!("Mission watchdog started (60s interval, 30min default timeout)");
    }

    /// Create with Mnemosyne instance for memory sync support.
    pub fn with_mnemosyne(mut self, mnemosyne: Arc<Mnemosyne>) -> Self {
        self.mnemosyne = Some(mnemosyne);
        self
    }
    /// Access orchestra (lazy-initialized on first use)
    pub fn orchestra(&self) -> &Orchestra {
        self.orchestra.get_or_init(Orchestra::new)
    }

    /// Access scheduler (lazy-initialized on first use)
    pub fn scheduler(&self) -> &Scheduler {
        self.scheduler.get_or_init(Scheduler::new)
    }

    /// Access strategic planner (lazy-initialized on first use)
    pub fn strategic_planner(&self) -> &StrategicPlanner {
        self.strategic_planner.get_or_init(StrategicPlanner::new)
    }

    /// Access global state manager (lazy-initialized on first use)
    pub fn global_state(&self) -> &Arc<GlobalStateManager> {
        self.global_state
            .get_or_init(|| Arc::new(GlobalStateManager::new()))
    }

    /// Access Pantheon orchestrator (lazy-initialized on first use)
    pub fn pantheon_orchestrator(&self) -> &Arc<zeus_orchestra::pantheon::PantheonOrchestrator> {
        self.pantheon_orchestrator.get_or_init(|| {
            let gs = self.global_state().clone();
            let bus = Arc::new(zeus_orchestra::MessageBus::new(256));
            Arc::new(zeus_orchestra::pantheon::PantheonOrchestrator::new(gs, bus))
        })
    }

    /// Access peer review system (lazy-initialized on first use)
    pub fn peer_review(&self) -> &PeerReviewSystem {
        self.peer_review.get_or_init(|| {
            PeerReviewSystem::new(self.global_state().clone(), ReviewPolicy::Single)
        })
    }

    /// Access peer review system mutably (lazy-initialized on first use)
    pub fn peer_review_mut(&mut self) -> &mut PeerReviewSystem {
        if self.peer_review.get().is_none() {
            let gs = self
                .global_state
                .get_or_init(|| Arc::new(GlobalStateManager::new()))
                .clone();
            let _ = self
                .peer_review
                .set(PeerReviewSystem::new(gs, ReviewPolicy::Single));
        }
        self.peer_review
            .get_mut()
            .unwrap_or_else(|| unreachable!("peer_review was initialized in this function"))
    }

    /// Access orchestration manager (lazy-initialized on first use)
    pub fn orchestration(&self) -> &OrchestrationManager {
        self.orchestration.get_or_init(OrchestrationManager::new)
    }

    /// Access mDNS discovery (lazy-initialized on first use)
    pub fn mdns_discovery(&self) -> &MdnsDiscovery {
        self.mdns_discovery.get_or_init(|| {
            let instance =
                std::env::var("ZEUS_INSTANCE_NAME").unwrap_or_else(|_| "zeus".to_string());
            let port = std::env::var("ZEUS_API_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080u16);
            MdnsDiscovery::new(instance, port)
        })
    }

    /// Access cron scheduler (lazy-initialized on first use)
    pub fn cron_scheduler(&self) -> &Arc<zeus_prometheus::CronScheduler> {
        self.cron_scheduler.get_or_init(|| {
            Arc::new(zeus_prometheus::CronScheduler::new(
                zeus_prometheus::SchedulerConfig::with_defaults(),
            ))
        })
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

/// API Server version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default API port
pub const DEFAULT_PORT: u16 = 8080;
