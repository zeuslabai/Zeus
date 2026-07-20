#![allow(dead_code)]
//! API client — connects TUI v2 to the Zeus gateway
//!
//! All API calls are async via reqwest. Results update the App state.
//! No rendering code here — pure data fetching.

use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const TUI_CHAT_SESSION_ID: &str = "agent:main:main";

const CHAT_RATE_LIMIT_MAX_RETRIES: usize = 3;
const CHAT_RATE_LIMIT_DEFAULT_DELAY: Duration = Duration::from_secs(1);
const CHAT_RATE_LIMIT_MAX_DELAY: Duration = Duration::from_secs(5);

fn normalize_session_chat_role(role: &str) -> Option<&'static str> {
    match role.trim().to_ascii_lowercase().as_str() {
        "system" => Some("system"),
        "user" => Some("user"),
        "assistant" => Some("assistant"),
        "tool" | "toolcall" | "tool_call" => Some("tool"),
        _ => None,
    }
}

/// Typed SSE events emitted by `chat_stream`.
///
/// The gateway may send standard OpenAI token deltas plus Zeus-specific
/// extension events for tool calls, iteration boundaries, and thinking.
#[derive(Debug, Clone)]
pub enum SseEvent {
    /// A text token chunk from the assistant reply.
    Token(String),
    /// A tool call is starting (Layer 2 — display in TUI).
    ToolStart { name: String, input: String },
    /// A tool call has completed.
    ToolEnd { name: String, output: String },
    /// Iteration boundary — agent is starting iteration N.
    Iter(u32),
    /// Thinking/reasoning text snippet (extended thinking or ThinkingDelta).
    Thinking(String),
    /// Token usage at end of turn.
    Usage { input: usize, output: usize },
}

/// Gateway API client
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct RateLimitRetry {
    delay: Duration,
    body: String,
}

fn parse_retry_after_header(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn parse_retry_after_body(body: &str) -> Option<Duration> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let retry = value
        .get("retry_after")
        .or_else(|| value.pointer("/error/retry_after"))?;
    if let Some(secs) = retry.as_u64() {
        return Some(Duration::from_secs(secs));
    }
    if let Some(secs) = retry.as_f64() {
        return Some(Duration::from_millis((secs.max(0.0) * 1000.0).ceil() as u64));
    }
    if let Some(secs) = retry.as_str().and_then(|s| s.trim().parse::<f64>().ok()) {
        return Some(Duration::from_millis((secs.max(0.0) * 1000.0).ceil() as u64));
    }
    None
}

fn clamp_rate_limit_delay(delay: Duration) -> Duration {
    delay.min(CHAT_RATE_LIMIT_MAX_DELAY)
}

async fn parse_rate_limit_response(resp: reqwest::Response) -> RateLimitRetry {
    let header_delay = parse_retry_after_header(&resp);
    let body = resp.text().await.unwrap_or_else(|_| "<no body>".to_string());
    let delay = header_delay
        .or_else(|| parse_retry_after_body(&body))
        .unwrap_or(CHAT_RATE_LIMIT_DEFAULT_DELAY);
    RateLimitRetry { delay: clamp_rate_limit_delay(delay), body }
}

async fn sleep_rate_limit(delay: Duration) {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
}

fn rate_limit_error(prefix: &str, retry: &RateLimitRetry) -> String {
    format!(
        "{prefix} rate limit exceeded after {} retries; retry_after={}s: {}",
        CHAT_RATE_LIMIT_MAX_RETRIES,
        retry.delay.as_secs_f32(),
        retry.body.trim()
    )
}


#[derive(Debug, Deserialize, Default)]
pub struct StatusResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub tools: usize,
    #[serde(default)]
    pub sessions_count: usize,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub auth_method: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub gateway_url: String,
    /// Agent's configured name (from config.name / onboarding)
    #[serde(default)]
    pub agent_name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct StatsResponse {
    #[serde(default)]
    pub sessions_count: usize,
    #[serde(default)]
    pub tools_count: usize,
    #[serde(default)]
    pub memory_files: usize,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct AgentResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    /// "local" for gateway agents, "channel" for Discord-discovered agents (S93)
    #[serde(default, rename = "type")]
    pub agent_type: String,
    #[serde(default)]
    pub health_score: f32,
    #[serde(default)]
    pub load_pct: f32,
    #[serde(default)]
    pub last_heartbeat: String,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub current_task: Option<String>,
}

/// One installed skill from `/v1/skills` (#185 Advanced→Skills subview).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SkillResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

/// One installed extension from `/v1/extensions` (#185 Advanced→Extensions).
/// `status` is captured as raw JSON because the server serializes the
/// `ExtensionStatus` enum as either a bare string (`"Running"`) or a tagged
/// object (`{"Error": "msg"}`); `status_label()` normalizes it for display.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtensionResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub status: serde_json::Value,
    #[serde(default)]
    pub extension_type: String,
}

impl ExtensionResponse {
    /// Normalize the enum-shaped `status` into a lowercase display word.
    pub fn status_label(&self) -> String {
        match &self.status {
            serde_json::Value::String(s) => s.to_lowercase(),
            serde_json::Value::Object(map) => map
                .keys()
                .next()
                .map(|k| k.to_lowercase())
                .unwrap_or_else(|| "unknown".to_string()),
            _ => "unknown".to_string(),
        }
    }
}

/// One MCP server from `/v1/mcp/servers` (#185 Advanced→MCP subview).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpServerResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub transport: String,
}

/// One project from `/v1/projects` (#185 Advanced→Projects subview).
///
/// The server stores raw JSON blobs in `~/.zeus/projects/*.json` with fields
/// `name/status/agents[]/missions/budget/spent`. The panel's `name`/`status`/
/// agent-count columns are honestly backed; there is NO `lead` field and NO
/// task-progress field (only spend/budget), so those render `—`/0 — same
/// server-extension-gap class as skills(category/tools) and agents(host/role).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub agents: Vec<serde_json::Value>,
    /// Project lead agent name (#249). Server defaults to `""` → rendered `—`.
    #[serde(default)]
    pub lead: String,
    /// Project progress 0–100 (#249). Server defaults to `0`.
    #[serde(default)]
    pub progress: u8,
}

/// One workflow instance from `/v1/workflows` (#185 Advanced→Canvas subview).
/// Chat→DAG execution state: status + node progress drive the canvas node graph.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkflowResponse {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub progress_percentage: f32,
    #[serde(default)]
    pub total_nodes: u32,
    #[serde(default)]
    pub completed_nodes: u32,
    #[serde(default)]
    pub failed_nodes: u32,
    #[serde(default)]
    pub created_at: String,
}

/// One agent task from `GET /v1/tasks/active` (#280 live task-tracker widget).
/// Backs the Claude-Code-style todo panel in the chat tab. `description` is the
/// task content; `status` is one of pending|active|paused|completed|failed.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TaskResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
}

/// One connected node from `/v1/nodes` (#185 Advanced→NodeComms FLEET LINKS).
/// `NodeInfo` exposes node_id/host/connected_at/capabilities, plus `rtt_ms`
/// (#249 — keepalive ping→pong delta, `0` until the first pong). `transport`
/// remains unbacked by the registry → renders `—`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NodeResponse {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub connected_at: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Round-trip time in ms from the keepalive ping→pong delta (#249).
    /// `0` means no pong measured yet → rendered `—`.
    #[serde(default)]
    pub rtt_ms: u64,
}

/// One active spawn from `GET /v1/spawner/active` (#185 Advanced→Spawner subview).
/// All entries are active-by-definition (the endpoint lists only running spawns),
/// so there is no per-entry status field — status is implicitly "running".
/// `channels` is not exposed by the tracker (server-extension gap).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SpawnResponse {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub role: String,
    /// RFC3339 start timestamp — runtime is derived as now − started_at.
    #[serde(default)]
    pub started_at: String,
}

/// One vector store from `GET /v1/vector_stores` (#185 Advanced→VectorStores).
/// The list endpoint serializes the store as OpenAI-style: `file_counts.total`
/// (nested, NOT a flat `file_count`) and `status` as a snake_case string
/// (`active`/`indexing`/`expired`). The backing struct exposes **no
/// vector-count, embedding-dim, or model field** — a file is not a vector
/// (one file → many chunks), so the panel's design columns vectors/dim/model
/// are dropped/honest-dashed (server-extension gap, batched for merakizzz).
/// COLLECTIONS therefore reshapes to the real subject: name · files · status.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VectorStoreResponse {
    #[serde(default)]
    pub name: String,
    /// Nested as `{"file_counts": {"total": N}}` in the list response.
    #[serde(default)]
    pub file_counts: FileCounts,
    #[serde(default)]
    pub status: String,
}

/// Nested file-count object from the vector-store list response.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FileCounts {
    #[serde(default)]
    pub total: usize,
}

/// One knowledge-graph community from `/v1/memory/communities` (#185
/// Advanced→Knowledge-Graph subview). `name` → COMMUNITY label, `entity_count`
/// → NODES. There is no per-community edge field, so the panel's EDGES column
/// is honest-dashed (server-extension gap, batched for merakizzz).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CommunityResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entity_count: u32,
}

/// One deploy target from `GET /v1/deploy/targets` (#185 Advanced→Deploy
/// subview, merakizzz "wire all"). The store's `DeployTargetRow` exposes
/// name/provider/environment/url/active — the TARGETS section wires all five
/// live. There is no per-target uptime/restart field (that was the old daemon-
/// health mock's fabrication), so those columns are dropped, not faked.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DeployTargetResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub active: bool,
}

/// One deployment from `GET /v1/deploy/history` (#185 Advanced→Deploy subview).
/// The store's `DeploymentResponse` exposes target_name/version/status/trigger/
/// duration_secs/created_at among others; RECENT DEPLOYMENTS wires
/// target/version/status/trigger live.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DeploymentResponse {
    #[serde(default)]
    pub target_name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub trigger: String,
}

/// Deploy fleet stats from `GET /v1/deploy/stats` (#185 Advanced→Deploy
/// summary line). All fields are honestly backed by `DeployStats`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DeployStatsResponse {
    #[serde(default)]
    pub total_targets: u64,
    #[serde(default)]
    pub total_deployments: u64,
    #[serde(default)]
    pub live_deployments: u64,
    #[serde(default)]
    pub failed_deployments: u64,
}

/// One agent wallet from `GET /v1/economy/wallets` (#185 Advanced→Economy
/// subview, merakizzz "wire all"). The ledger's `AgentWallet` exposes
/// agent_id/balance/total_earned/total_spent in integer credits — the AGORA
/// WALLET card wires those live. The old "$ 247.83 USDC" mock had no backend
/// (credits are an integer ledger balance, not a USDC float), so it is dropped,
/// not faked.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EconomyWalletResponse {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub balance: u64,
    #[serde(default)]
    pub total_earned: u64,
    #[serde(default)]
    pub total_spent: u64,
}

/// Public on-chain wallet summary from `GET /v1/wallet/onchain` (#352).
/// Zero key material: address + balances + public mint/cluster only.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OnchainWalletResponse {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub sol_lamports: u64,
    #[serde(default)]
    pub sol: f64,
    #[serde(default)]
    pub token_balance: u64,
    #[serde(default)]
    pub token_decimals: u8,
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub cluster: String,
}

/// One recent Solana signature row from `GET /v1/wallet/onchain/transactions`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OnchainTxResponse {
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub slot: u64,
    #[serde(default)]
    pub block_time: Option<i64>,
    #[serde(default)]
    pub confirmation_status: Option<String>,
    #[serde(default)]
    pub err: Option<serde_json::Value>,
}

/// `build_transfer_plan` preflight metadata echoed by `/v1/wallet/onchain/transfer`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OnchainTransferPlanResponse {
    #[serde(default)]
    pub sender_sol_lamports: u64,
    #[serde(default)]
    pub sender_token_balance: u64,
    #[serde(default)]
    pub token_balance_sufficient: bool,
    #[serde(default)]
    pub recipient_ata_exists: bool,
    #[serde(default)]
    pub ata_create_required: bool,
}

/// Devnet on-chain transfer response from `POST /v1/wallet/onchain/transfer`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OnchainTransferResponse {
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub recipient: String,
    #[serde(default)]
    pub amount: u64,
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub ata_created: bool,
    #[serde(default)]
    pub cluster: String,
    #[serde(default)]
    pub plan: OnchainTransferPlanResponse,
}

/// One ledger transaction from `GET /v1/economy/transactions` (#185
/// Advanced→Economy RECENT TRANSACTIONS section). The ledger's `Transaction`
/// exposes kind/reason/from_agent/to_agent/amount/note/created_at; the section
/// wires from_agent/reason/amount live. `kind` (earn/spend/mint/burn) colors
/// the amount.
///
/// `kind` and `reason` are `serde_json::Value` not `String`: the `Transaction`
/// enums are `rename_all="snake_case"` so most variants serialize as bare
/// strings (`"earn"`, `"mint"`), BUT the `Unknown(String)`/`Other(String)`
/// tuple variants serialize as tagged objects (`{"unknown":"…"}`). A flat
/// `String` deser would fail the whole row on those — so we accept `Value` and
/// normalize via `tx_label()` (same pattern as `ExtensionResponse.status`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EconomyTxResponse {
    #[serde(default)]
    pub kind: serde_json::Value,
    #[serde(default)]
    pub reason: serde_json::Value,
    #[serde(default)]
    pub from_agent: Option<String>,
    #[serde(default)]
    pub to_agent: Option<String>,
    #[serde(default)]
    pub amount: u64,
}

impl EconomyTxResponse {
    /// Normalize a `Value`-typed enum field to a lowercase label: a bare
    /// string passes through; a tagged object (`{"unknown":"x"}`) yields its
    /// first key. Empty/absent → "".
    pub(crate) fn label(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.to_ascii_lowercase(),
            serde_json::Value::Object(map) => map
                .keys()
                .next()
                .map(|k| k.to_ascii_lowercase())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The transaction kind label (earn/spend/transfer/mint/burn/fee/…).
    pub fn kind_label(&self) -> String {
        Self::label(&self.kind)
    }

    /// The transaction reason label (task_completion/llm_call/…).
    pub fn reason_label(&self) -> String {
        Self::label(&self.reason)
    }
}

/// One TTS provider from `/v1/tts/providers` (#185 Advanced→Voice subview).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TtsProviderResponse {
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// One TTS voice from `/v1/tts/voices` (#185 Advanced→Voice subview).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TtsVoiceResponse {
    pub provider: String,
    pub voice_id: String,
    pub name: String,
    #[serde(default)]
    pub gender: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ChannelResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub connected_at: Option<String>,
    #[serde(default)]
    pub last_message_at: Option<String>,
}

/// One tool from the gateway's `GET /v1/tools` registry
/// (`{ "tools": [ { name, description, category, parameters } ] }`).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ToolInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonRoomResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub participant_count: usize,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonMissionResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub agent_count: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonMessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub message_type: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub channel_source: Option<SessionChannelSource>,
}

/// One pending approval from `GET /v1/approvals` (#235 Approvals tab de-mock).
/// The gateway's `PendingApproval` serializes with `id`, `tool_name`, `args`,
/// `agent_id`, `created_at`, and `status` (tagged enum). We deserialize
/// loosely: `args` as a JSON value (display string), `status` as a string.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ApprovalResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub created_at: String,
    /// Status tag — "pending", "approved", "denied", "expired".
    /// The gateway serializes as `{"status":"pending"}` etc.; we extract the
    /// tag string for filtering.
    #[serde(default)]
    pub status: serde_json::Value,
}

impl ApprovalResponse {
    /// True if this approval is still pending (not yet resolved).
    pub fn is_pending(&self) -> bool {
        match &self.status {
            serde_json::Value::String(s) => s == "pending",
            serde_json::Value::Object(map) => map
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s == "pending")
                .unwrap_or(true),
            _ => true,
        }
    }

    /// Display string for the args — compact JSON or raw string.
    pub fn args_display(&self) -> String {
        match &self.args {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionChannelSource {
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
}

/// One workspace memory file (`GET /v1/memory/files`).
#[derive(Debug, Deserialize, Clone)]
pub struct MemoryFileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified: String,
}

/// One session summary row (`GET /v1/sessions`).
#[derive(Debug, Deserialize, Clone)]
pub struct SessionSummary {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub message_count: u64,
    #[serde(default)]
    pub est_tokens: u64,
    #[serde(default)]
    pub last_preview: String,
}

/// One memory search hit (`POST /v1/memory/search`).
///
/// The gateway returns one of two shapes depending on whether Mnemosyne is
/// available: the hybrid path emits `id`/`session_id`/`memory_type`/`importance`,
/// the file fallback emits `path`. All variant-specific fields are optional so
/// a single struct deserializes both.
#[derive(Debug, Deserialize, Clone)]
pub struct MemorySearchHit {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub session_id: String,
}

/// Total-request cap for non-streaming API calls (status, sessions, config…)
/// so they still fail fast now that the client has no blanket timeout.
const NON_STREAMING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            // No blanket total-request timeout: it would cap streaming chat
            // responses too (the gateway's cooking loop can run up to 30 min),
            // and reqwest surfaces a mid-stream total-timeout as the misleading
            // "error decoding response body" (#194). Instead:
            //   - connect_timeout: fail fast when the gateway is unreachable
            //   - read_timeout: idle-between-chunks cap. The gateway's SSE
            //     keep-alive fires every 15s of quiet (axum KeepAlive default),
            //     including during long tool calls, so a healthy stream never
            //     trips this — only a genuinely dead connection does.
            //   - non-streaming requests add a per-request .timeout() below.
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn status(&self) -> Result<StatusResponse, String> {
        self.get("/v1/status").await
    }

    pub async fn stats(&self) -> Result<StatsResponse, String> {
        self.get("/v1/stats").await
    }

    pub async fn agents(&self) -> Result<Vec<AgentResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] agents: Vec<AgentResponse> }
        let resp: Resp = self.get("/v1/network/agents").await?;
        Ok(resp.agents)
    }

    /// Active workflows (`/v1/workflows`) — chat→DAG instances for the Canvas subview (#185).
    pub async fn workflows(&self) -> Result<Vec<WorkflowResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] workflows: Vec<WorkflowResponse> }
        let resp: Resp = self.get("/v1/workflows").await?;
        Ok(resp.workflows)
    }

    /// Active agent tasks (`GET /v1/tasks/active`) — the prometheus agent todo
    /// list fed by `todo_write`, for the chat tab's live task-tracker widget
    /// (#280). Returns the `{tasks, count}` envelope's `tasks` array.
    pub async fn active_tasks(&self) -> Result<Vec<TaskResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] tasks: Vec<TaskResponse> }
        let resp: Resp = self.get("/v1/tasks/active").await?;
        Ok(resp.tasks)
    }

    /// Connected fleet nodes (`/v1/nodes`) — FLEET LINKS for the NodeComms
    /// subview (#185). Returns the live node registry; presence == link up.
    pub async fn nodes(&self) -> Result<Vec<NodeResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] nodes: Vec<NodeResponse> }
        let resp: Resp = self.get("/v1/nodes").await?;
        Ok(resp.nodes)
    }

    /// Active spawns (`/v1/spawner/active`) — Advanced→Spawner subview (#185).
    /// Lists only currently-running subagents; status is implicitly "running".
    pub async fn spawner_active(&self) -> Result<Vec<SpawnResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] active: Vec<SpawnResponse> }
        let resp: Resp = self.get("/v1/spawner/active").await?;
        Ok(resp.active)
    }

    /// Vector stores (`GET /v1/vector_stores`) — Advanced→VectorStores subview.
    /// Response is OpenAI-style `{object:"list", data:[...]}`. COLLECTIONS
    /// overlays name·files·status; the design's vectors/dim/model have no
    /// backend (server-extension gap) and stay honest-dashed.
    pub async fn vector_stores(&self) -> Result<Vec<VectorStoreResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] data: Vec<VectorStoreResponse> }
        let resp: Resp = self.get("/v1/vector_stores").await?;
        Ok(resp.data)
    }

    /// Knowledge-graph communities (`GET /v1/memory/communities`) —
    /// Advanced→Knowledge-Graph subview (#185 wiring). The list endpoint backs
    /// the panel's real subject (KG communities): `name` and `entity_count`
    /// (→ NODES) are honest live fields. There is **no per-community edge
    /// count** — `Community` exposes only `id/name/description/entity_count`;
    /// edges exist solely as a global relationship summary, not sliced by
    /// community — so the panel's EDGES column is honest-dashed (server-
    /// extension gap, batched for merakizzz), NOT fabricated.
    pub async fn communities(&self) -> Result<Vec<CommunityResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] communities: Vec<CommunityResponse> }
        let resp: Resp = self.get("/v1/memory/communities").await?;
        Ok(resp.communities)
    }

    /// Deploy targets (`GET /v1/deploy/targets`) — Advanced→Deploy TARGETS
    /// section (#185). Response key `targets`.
    pub async fn deploy_targets(&self) -> Result<Vec<DeployTargetResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] targets: Vec<DeployTargetResponse> }
        let resp: Resp = self.get("/v1/deploy/targets").await?;
        Ok(resp.targets)
    }

    /// Recent deployments (`GET /v1/deploy/history`) — Advanced→Deploy RECENT
    /// DEPLOYMENTS section (#185). Response key `deployments`.
    pub async fn deploy_history(&self) -> Result<Vec<DeploymentResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] deployments: Vec<DeploymentResponse> }
        let resp: Resp = self.get("/v1/deploy/history").await?;
        Ok(resp.deployments)
    }

    /// Deploy fleet stats (`GET /v1/deploy/stats`) — Advanced→Deploy summary
    /// line (#185). The stats object is returned at the top level.
    pub async fn deploy_stats(&self) -> Result<DeployStatsResponse, String> {
        self.get("/v1/deploy/stats").await
    }

    /// Agent wallets (`GET /v1/economy/wallets`) — Advanced→Economy AGORA
    /// WALLET card (#185). Response key `wallets`.
    pub async fn economy_wallets(&self) -> Result<Vec<EconomyWalletResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] wallets: Vec<EconomyWalletResponse> }
        let resp: Resp = self.get("/v1/economy/wallets").await?;
        Ok(resp.wallets)
    }

    /// Recent ledger transactions (`GET /v1/economy/transactions`) —
    /// Advanced→Economy RECENT TRANSACTIONS section (#185). Response key
    /// `transactions`.
    pub async fn economy_transactions(&self) -> Result<Vec<EconomyTxResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] transactions: Vec<EconomyTxResponse> }
        let resp: Resp = self.get("/v1/economy/transactions?limit=20").await?;
        Ok(resp.transactions)
    }


    /// Public on-chain wallet info (`GET /v1/wallet/onchain`). #352.
    pub async fn wallet_onchain(&self) -> Result<OnchainWalletResponse, String> {
        self.get("/v1/wallet/onchain").await
    }

    /// Recent on-chain signatures (`GET /v1/wallet/onchain/transactions`). #352.
    pub async fn wallet_onchain_transactions(&self) -> Result<Vec<OnchainTxResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] transactions: Vec<OnchainTxResponse> }
        let resp: Resp = self.get("/v1/wallet/onchain/transactions?limit=20").await?;
        Ok(resp.transactions)
    }

    /// Submit a devnet token transfer and receive the gateway preflight plan. #352.
    pub async fn wallet_onchain_transfer(
        &self,
        recipient: &str,
        amount: u64,
    ) -> Result<OnchainTransferResponse, String> {
        #[derive(Serialize)]
        struct Req<'a> {
            recipient: &'a str,
            amount: u64,
        }
        self.post("/v1/wallet/onchain/transfer", &Req { recipient, amount }).await
    }

    /// Transfer credits between agents (`POST /v1/economy/transfer`). #190 P2.
    /// Mirrors the ZeusWeb `economy_transfer` wrapper. The gateway's
    /// `TransferRequest` requires `from`, `to`, `amount` (u64); `note` is
    /// optional. Returns the new balance on success.
    pub async fn economy_transfer(
        &self,
        from: &str,
        to: &str,
        amount: u64,
        note: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        #[derive(Serialize)]
        struct Req<'a> {
            from: &'a str,
            to: &'a str,
            amount: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            note: Option<&'a str>,
        }
        self.post("/v1/economy/transfer", &Req { from, to, amount, note }).await
    }

    /// Unstake tokens (`POST /v1/economy/unstake`). #190 P2.
    /// Mirrors the ZeusWeb `economy_unstake` wrapper. The gateway's
    /// `UnstakeRequest` requires `agent_id` and `stake_id`.
    pub async fn economy_unstake(
        &self,
        agent_id: &str,
        stake_id: &str,
    ) -> Result<serde_json::Value, String> {
        #[derive(Serialize)]
        struct Req<'a> {
            agent_id: &'a str,
            stake_id: &'a str,
        }
        self.post("/v1/economy/unstake", &Req { agent_id, stake_id }).await
    }

    /// Installed skills (`/v1/skills`) — name/description/enabled per entry (#185).
    pub async fn skills(&self) -> Result<Vec<SkillResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] skills: Vec<SkillResponse> }
        let resp: Resp = self.get("/v1/skills").await?;
        Ok(resp.skills)
    }

    /// Installed extensions (`/v1/extensions`) — Advanced→Extensions subview
    /// (#185 wiring). The store flattens `Extension` (name/version/status) with
    /// API metadata (`extension_type`); we normalize `status` to a display
    /// string since it serializes as an enum (`"Running"` / `{"Error": "..."}`).
    pub async fn extensions(&self) -> Result<Vec<ExtensionResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] extensions: Vec<ExtensionResponse> }
        let resp: Resp = self.get("/v1/extensions").await?;
        Ok(resp.extensions)
    }

    /// Configured projects (`/v1/projects`) — Advanced→Projects subview.
    pub async fn projects(&self) -> Result<Vec<ProjectResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] projects: Vec<ProjectResponse> }
        let resp: Resp = self.get("/v1/projects").await?;
        Ok(resp.projects)
    }

    /// Configured MCP servers (`/v1/mcp/servers`) from ~/.zeus/mcp.json (#185).
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] servers: Vec<McpServerResponse> }
        let resp: Resp = self.get("/v1/mcp/servers").await?;
        Ok(resp.servers)
    }

    /// Available TTS providers (`/v1/tts/providers`) for the Voice subview (#185).
    pub async fn tts_providers(&self) -> Result<Vec<TtsProviderResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] providers: Vec<TtsProviderResponse> }
        let resp: Resp = self.get("/v1/tts/providers").await?;
        Ok(resp.providers)
    }

    /// Available TTS voices (`/v1/tts/voices`) for the Voice subview (#185).
    pub async fn tts_voices(&self) -> Result<Vec<TtsVoiceResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] voices: Vec<TtsVoiceResponse> }
        let resp: Resp = self.get("/v1/tts/voices").await?;
        Ok(resp.voices)
    }

    pub async fn channels(&self) -> Result<Vec<ChannelResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] channels: Vec<ChannelResponse> }
        let resp: Resp = self.get("/v1/channels").await?;
        Ok(resp.channels)
    }

    pub async fn tools(&self) -> Result<Vec<ToolInfo>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] tools: Vec<ToolInfo> }
        let resp: Resp = self.get("/v1/tools").await?;
        Ok(resp.tools)
    }

    pub async fn chat(&self, message: &str, session_id: Option<&str>) -> Result<ChatResponse, String> {
        let req = ChatRequest {
            message: message.to_string(),
            session_id: session_id.map(|s| s.to_string()),
        };
        // Non-streaming chat runs the full cooking loop server-side and returns a
        // SINGLE JSON response only when the cook completes — it sends NO
        // intermediate bytes. The shared `self.client` carries a 120s read_timeout
        // (an idle-between-CHUNKS cap meant for the SSE stream path, which emits a
        // keep-alive every 15s). On this NON-streaming path there are no chunks, so
        // any cook longer than 120s trips that read_timeout and drops the request,
        // surfacing as the misleading "error sending request" (#278). Use a
        // dedicated client with NO read_timeout here — only the 1800s total cap
        // applies, matching the gateway's cook budget.
        let chat_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let mut attempt = 0usize;
        loop {
            let resp = chat_client
                .post(format!("{}/v1/chat", self.base_url))
                .timeout(Duration::from_secs(1800))
                .json(&req)
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            // The gateway's `chat()` returns `Result<Json<ChatResponse>, (StatusCode,
            // String)>` — on error that's a non-2xx status + a PLAIN-TEXT body. Without
            // this status check, `.json::<ChatResponse>()` tries to JSON-decode that
            // text → "error decoding response body" → the misleading "Parse failed: …"
            // that masks the real gateway error (#282). Surface the body verbatim.
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                let retry = parse_rate_limit_response(resp).await;
                if attempt < CHAT_RATE_LIMIT_MAX_RETRIES {
                    attempt += 1;
                    sleep_rate_limit(retry.delay).await;
                    continue;
                }
                return Err(rate_limit_error("[gateway 429]", &retry));
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("[gateway {}] {}", status.as_u16(), body));
            }
            return resp.json::<ChatResponse>()
                .await
                .map_err(|e| format!("Parse failed: {}", e));
        }
    }

    /// Stream a chat response token-by-token via OpenAI-compatible SSE.
    ///
    /// Uses `bytes_stream()` for true incremental delivery — each chunk is
    /// processed as it arrives from the gateway, so `on_token` fires in
    /// real-time rather than after the full response has buffered.
    ///
    /// The `on_event` callback receives typed `SseEvent` values so callers can
    /// display tool calls, iteration counts, and thinking snippets in real-time.
    pub async fn chat_stream<F>(&self, message: &str, mut on_event: F) -> Result<String, String>
    where
        F: FnMut(SseEvent),
    {
        use futures_util::StreamExt;

        const MAX_STREAM_HISTORY_MESSAGES: usize = 24;

        #[derive(serde::Serialize)]
        struct OaiReq {
            model: &'static str,
            messages: Vec<OaiMsg>,
            stream: bool,
            session_id: String,
        }
        #[derive(serde::Serialize)]
        struct OaiMsg { role: String, content: String }

        let session_id = TUI_CHAT_SESSION_ID.to_string();
        let mut messages: Vec<OaiMsg> = self
            .session_messages(&session_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| {
                let role = normalize_session_chat_role(&m.role)?;
                if m.content.trim().is_empty() {
                    return None;
                }
                Some(OaiMsg { role: role.to_string(), content: m.content })
            })
            .rev()
            .take(MAX_STREAM_HISTORY_MESSAGES)
            .collect::<Vec<_>>();
        messages.reverse();
        messages.push(OaiMsg { role: "user".to_string(), content: message.to_string() });

        let req = OaiReq {
            model: "default",
            messages,
            stream: true,
            session_id,
        };

        let mut attempt = 0usize;
        let resp = loop {
            let resp = self.client
                .post(format!("{}/v1/chat/completions", self.base_url))
                .json(&req)
                .send()
                .await
                .map_err(|e| format!("Stream request failed: {e}"))?;

            // Check HTTP status before streaming — return error body for 4xx/5xx
            // instead of streaming garbage that produces blank messages in the TUI.
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                let retry = parse_rate_limit_response(resp).await;
                if attempt < CHAT_RATE_LIMIT_MAX_RETRIES {
                    attempt += 1;
                    sleep_rate_limit(retry.delay).await;
                    continue;
                }
                return Err(rate_limit_error("HTTP 429", &retry));
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_else(|_| "<no body>".to_string());
                return Err(format!("HTTP {}: {}", status, body.trim()));
            }
            break resp;
        };

        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        // SSE lines can span multiple HTTP chunks; we keep a partial-line buffer.
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream read failed: {e}"))?;
            // Append raw bytes as UTF-8 (gateway always sends UTF-8)
            let text = String::from_utf8_lossy(&chunk);
            buf.push_str(&text);

            // Process all complete lines in the buffer
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim_end_matches('\r').to_string();
                buf = buf[newline_pos + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(full);
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        // ── Standard OpenAI token delta ──────────────────────
                        if let Some(token) = v["choices"][0]["delta"]["content"].as_str() {
                            full.push_str(token);
                            on_event(SseEvent::Token(token.to_string()));
                            continue;
                        }

                        // ── Zeus-specific extensions ─────────────────────────
                        // Tool call start: {"event":"tool_start","tool":"Bash","input":"ls"}
                        if v["event"] == "tool_start" {
                            let name = v["tool"].as_str().unwrap_or("tool").to_string();
                            let input = v["input"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::ToolStart { name, input });
                            continue;
                        }

                        // Tool call result: {"event":"tool_end","tool":"Bash","output":"..."}
                        if v["event"] == "tool_end" {
                            let name = v["tool"].as_str().unwrap_or("tool").to_string();
                            let output = v["output"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::ToolEnd { name, output });
                            continue;
                        }

                        // Iteration boundary: {"event":"iter","n":3}
                        if v["event"] == "iter" {
                            let n = v["n"].as_u64().unwrap_or(0) as u32;
                            on_event(SseEvent::Iter(n));
                            continue;
                        }

                        // Thinking delta: {"event":"thinking","text":"..."}
                        if v["event"] == "thinking" {
                            let text = v["text"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::Thinking(text));
                            continue;
                        }

                        // Anthropic-style thinking in delta: {"choices":[{"delta":{"thinking":"..."}}]}
                        if let Some(thinking) = v["choices"][0]["delta"]["thinking"].as_str() {
                            on_event(SseEvent::Thinking(thinking.to_string()));
                        }

                        // Usage event: {"event":"usage","input_tokens":N,"output_tokens":N}
                        if v["event"] == "usage" {
                            let input = v["input_tokens"].as_u64().unwrap_or(0) as usize;
                            let output = v["output_tokens"].as_u64().unwrap_or(0) as usize;
                            on_event(SseEvent::Usage { input, output });
                            continue;
                        }
                    }
                }
            }
        }

        Ok(full)
    }

    /// Fetch session messages (for loading history on TUI startup)
    pub async fn session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            messages: Vec<SessionMessage>,
        }
        let resp: Resp = self.get(&format!("/v1/sessions/{}", session_id)).await?;
        Ok(resp.messages)
    }

    /// List workspace memory files (`GET /v1/memory/files`).
    pub async fn memory_files(&self) -> Result<Vec<MemoryFileEntry>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            files: Vec<MemoryFileEntry>,
        }
        let resp: Resp = self.get("/v1/memory/files").await?;
        Ok(resp.files)
    }

    /// List session summaries (`GET /v1/sessions`).
    pub async fn sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            sessions: Vec<SessionSummary>,
        }
        let resp: Resp = self.get(&format!("/v1/sessions?limit={}", limit)).await?;
        Ok(resp.sessions)
    }

    /// Search memory via Mnemosyne hybrid (or file fallback) (`POST /v1/memory/search`).
    pub async fn memory_search(&self, query: &str, limit: usize) -> Result<Vec<MemorySearchHit>, String> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            query: &'a str,
            limit: usize,
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            results: Vec<MemorySearchHit>,
        }
        let resp: Resp = self.post("/v1/memory/search", &Req { query, limit }).await?;
        Ok(resp.results)
    }

    /// Fetch the default session ID from gateway status
    pub async fn default_session_id(&self) -> Result<String, String> {
        #[derive(serde::Deserialize)]
        struct Sessions {
            #[serde(default)]
            sessions: Vec<SessionEntry>,
        }
        #[derive(serde::Deserialize)]
        struct SessionEntry {
            id: String,
        }
        let resp: Sessions = self.get("/v1/sessions?limit=1").await?;
        resp.sessions.first()
            .map(|s| s.id.clone())
            .ok_or_else(|| "No sessions found".to_string())
    }

    /// Fetch Pantheon war rooms
    pub async fn pantheon_rooms(&self) -> Result<Vec<PantheonRoomResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            rooms: Vec<PantheonRoomResponse>,
        }
        let resp: Resp = self.get("/v1/pantheon/rooms").await?;
        Ok(resp.rooms)
    }

    /// Fetch Pantheon missions
    pub async fn pantheon_missions(&self) -> Result<Vec<PantheonMissionResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            missions: Vec<PantheonMissionResponse>,
        }
        let resp: Resp = self.get("/v1/pantheon/missions").await?;
        Ok(resp.missions)
    }

    /// Fetch the message stream for a Pantheon room (drill-in view, #104).
    pub async fn pantheon_messages(&self, room_id: &str) -> Result<Vec<PantheonMessageResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            messages: Vec<PantheonMessageResponse>,
        }
        let resp: Resp = self
            .get(&format!("/v1/pantheon/rooms/{}/messages", room_id))
            .await?;
        Ok(resp.messages)
    }

    /// Fetch a workspace memory file by path (e.g. "daily/2026-03-27.md")
    pub async fn memory_file(&self, path: &str) -> Result<String, String> {
        let resp = self.client
            .get(format!("{}/v1/memory/files/{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Not found: {}", resp.status()));
        }
        // Response may be JSON with content field or raw text
        let text = resp.text().await.map_err(|e| format!("Read failed: {e}"))?;
        // Try JSON first (gateway wraps in {"content": "..."})
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(content) = v["content"].as_str() {
                return Ok(content.to_string());
            }
        Ok(text)
    }

    /// Fetch messages from a Pantheon room
    pub async fn pantheon_room_messages(&self, room_id: &str) -> Result<Vec<PantheonMessageResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            messages: Vec<PantheonMessageResponse>,
        }
        let resp: Resp = self.get(&format!("/v1/pantheon/rooms/{}/messages?limit=50", room_id)).await?;
        Ok(resp.messages)
    }

    /// Send a message to a Pantheon room
    pub async fn pantheon_send_message(&self, room_id: &str, content: &str, sender: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Req { content: String, sender_id: String, message_type: String }
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/rooms/{}/messages", room_id),
            &Req { content: content.to_string(), sender_id: sender.to_string(), message_type: "chat".to_string() },
        ).await?;
        Ok(())
    }

    /// Create a Pantheon room
    pub async fn pantheon_create_room(&self, name: &str) -> Result<String, String> {
        #[derive(serde::Serialize)]
        struct Req { name: String, created_by: String }
        let resp: serde_json::Value = self.post("/v1/pantheon/rooms", &Req {
            name: name.to_string(),
            created_by: "tui-user".to_string(),
        }).await?;
        Ok(resp["id"].as_str().unwrap_or("").to_string())
    }

    /// Intervene in a Pantheon mission (pause/cancel/redirect)
    pub async fn pantheon_intervene(&self, mission_id: &str, action: &str, reason: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Req { action: String, reason: String }
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/missions/{}/intervene", mission_id),
            &Req { action: action.to_string(), reason: reason.to_string() },
        ).await?;
        Ok(())
    }

    /// Approve a Pantheon mission plan
    pub async fn pantheon_approve(&self, mission_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/missions/{}/approve", mission_id),
            &serde_json::json!({}),
        ).await?;
        Ok(())
    }

    /// Reject a mission (cancel with optional reason)
    pub async fn pantheon_reject_mission(&self, mission_id: &str, reason: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/missions/{}/reject", mission_id),
            &serde_json::json!({"reason": reason, "rejector_id": "tui", "rejector_name": "TUI"}),
        ).await?;
        Ok(())
    }

    /// Approve a plan card (plan-level approval, not mission-level)
    pub async fn pantheon_approve_plan(&self, plan_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/plans/{}/approve", plan_id),
            &serde_json::json!({"approver_id": "tui", "approver_name": "TUI"}),
        ).await?;
        Ok(())
    }

    /// Reject a plan card with an optional reason
    pub async fn pantheon_reject_plan(&self, plan_id: &str, reason: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/plans/{}/reject", plan_id),
            &serde_json::json!({"reason": reason, "approver_id": "tui", "approver_name": "TUI"}),
        ).await?;
        Ok(())
    }

    /// Clear a session (remove all messages, keep file)
    pub async fn session_clear(&self, session_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/sessions/{}/clear", session_id),
            &serde_json::json!({}),
        ).await?;
        Ok(())
    }

    /// Compact a session (strip tool outputs from older messages)
    pub async fn session_compact(&self, session_id: &str) -> Result<String, String> {
        let resp: serde_json::Value = self.post(
            &format!("/v1/sessions/{}/compact", session_id),
            &serde_json::json!({}),
        ).await?;
        Ok(resp["message"].as_str().unwrap_or("Compacted").to_string())
    }

    /// Fetch the full config from the gateway (sanitized — no secrets).
    pub async fn config(&self) -> Result<serde_json::Value, String> {
        self.get("/v1/config").await
    }

    /// Update config fields via PUT /v1/config.
    pub async fn update_config(&self, updates: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.client
            .put(format!("{}/v1/config", self.base_url))
            .json(updates)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }

    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Pending tool approvals (`GET /v1/approvals`) — Approvals tab (#235).
    /// The gateway returns a JSON array of `PendingApproval` objects.
    pub async fn approvals(&self) -> Result<Vec<ApprovalResponse>, String> {
        self.get("/v1/approvals").await
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .timeout(NON_STREAMING_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<T>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }

    async fn post<T: serde::de::DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T, String> {
        self.client
            .post(format!("{}{}", self.base_url, path))
            .timeout(NON_STREAMING_TIMEOUT)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<T>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }
}

#[cfg(test)]
mod chat_error_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    /// Spawn a one-shot HTTP server that replies with the given status line and
    /// plain-text body, then returns its base URL. No mock-HTTP dependency — a
    /// raw TCP listener is enough to reproduce the gateway's 500 + text body.
    async fn spawn_once(status_line: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let status_line = status_line.to_string();
        let body = body.to_string();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Drain the request (best-effort) so the client's write completes.
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        format!("http://{}", addr)
    }

    fn raw_response(status_line: &str, content_type: &str, headers: &[(&str, &str)], body: &str) -> String {
        let mut resp = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            resp.push_str(name);
            resp.push_str(": ");
            resp.push_str(value);
            resp.push_str("\r\n");
        }
        resp.push_str("\r\n");
        resp.push_str(body);
        resp
    }

    async fn spawn_response_sequence(responses: Vec<String>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_task = Arc::clone(&count);
        tokio::spawn(async move {
            for resp in responses {
                if let Ok((mut sock, _)) = listener.accept().await {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    count_for_task.fetch_add(1, Ordering::SeqCst);
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.flush().await;
                }
            }
        });
        (format!("http://{}", addr), count)
    }


    async fn read_http_request(sock: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let mut header_end = None;
        loop {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = Some(pos + 4);
                break;
            }
        }

        if let Some(end) = header_end {
            let headers = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let content_len = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let have_body = buf.len().saturating_sub(end);
            while have_body + (buf.len().saturating_sub(end + have_body)) < content_len {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.len().saturating_sub(end) >= content_len {
                    break;
                }
            }
        }

        String::from_utf8_lossy(&buf).into_owned()
    }

    async fn spawn_session_stream_server() -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let req = read_http_request(&mut sock).await;
                assert!(
                    req.starts_with("GET /v1/sessions/agent:main:main "),
                    "expected session history fetch first, got: {req}"
                );
                let body = r#"{"messages":[{"role":"User","content":"remember lightdm"},{"role":"Assistant","content":"noted"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }

            if let Ok((mut sock, _)) = listener.accept().await {
                let req = read_http_request(&mut sock).await;
                let body = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                let _ = tx.send(body);
                let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        (format!("http://{}", addr), rx)
    }


    #[tokio::test]
    async fn chat_stream_sends_stable_session_and_recent_history() {
        let (base, posted_body) = spawn_session_stream_server().await;
        let client = ApiClient::new(&base);
        let mut tokens = Vec::new();

        let reply = client
            .chat_stream("go check it", |event| {
                if let SseEvent::Token(token) = event {
                    tokens.push(token);
                }
            })
            .await
            .expect("stream succeeds");

        assert_eq!(reply, "ok");
        assert_eq!(tokens, vec!["ok".to_string()]);

        let body = posted_body.await.expect("captured POST body");
        let payload: serde_json::Value = serde_json::from_str(&body).expect("json request body");
        assert_eq!(payload["session_id"], "agent:main:main");
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(payload["messages"][0]["content"], "remember lightdm");
        assert_eq!(payload["messages"][1]["role"], "assistant");
        assert_eq!(payload["messages"][1]["content"], "noted");
        assert_eq!(payload["messages"][2]["role"], "user");
        assert_eq!(payload["messages"][2]["content"], "go check it");
    }

    #[tokio::test]
    async fn chat_retries_short_429_then_succeeds() {
        let too_many = raw_response(
            "429 Too Many Requests",
            "application/json",
            &[("Retry-After", "0")],
            r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error","retry_after":0}}"#,
        );
        let ok = raw_response(
            "200 OK",
            "application/json",
            &[],
            r#"{"response":"ok after retry","session_id":"agent:main:main"}"#,
        );
        let (base, count) = spawn_response_sequence(vec![too_many, ok]).await;
        let client = ApiClient::new(&base);

        let resp = client.chat("hi", None).await.expect("429 should retry once");

        assert_eq!(resp.response, "ok after retry");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn chat_stream_retries_short_429_then_succeeds() {
        let sessions = raw_response(
            "200 OK",
            "application/json",
            &[],
            r#"{"messages":[]}"#,
        );
        let too_many = raw_response(
            "429 Too Many Requests",
            "application/json",
            &[("Retry-After", "0")],
            r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error","retry_after":0}}"#,
        );
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
        let ok = raw_response("200 OK", "text/event-stream", &[], sse);
        let (base, count) = spawn_response_sequence(vec![sessions, too_many, ok]).await;
        let client = ApiClient::new(&base);
        let mut tokens = Vec::new();

        let reply = client
            .chat_stream("hi", |event| {
                if let SseEvent::Token(token) = event {
                    tokens.push(token);
                }
            })
            .await
            .expect("429 should retry once before streaming");

        assert_eq!(reply, "ok");
        assert_eq!(tokens, vec!["ok".to_string()]);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn chat_500_surfaces_body_text_not_parse_failed() {
        // Regression for #282: the gateway's chat() returns 500 + a PLAIN-TEXT
        // error body. The client must surface that body, not mask it behind a
        // JSON "Parse failed" (which is what decoding text as ChatResponse gives).
        let base = spawn_once("500 Internal Server Error", "cook budget exhausted").await;
        let client = ApiClient::new(&base);
        let err = client.chat("hi", None).await.expect_err("500 must be an error");
        assert!(
            err.contains("cook budget exhausted"),
            "expected the real gateway body, got: {err}"
        );
        assert!(
            err.contains("[gateway 500]"),
            "expected the status-tagged prefix, got: {err}"
        );
        assert!(
            !err.contains("Parse failed"),
            "must NOT mask the error as a parse failure, got: {err}"
        );
    }
}
