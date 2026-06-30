//! Fleet agent registration and heartbeat handlers
//!
//! Bridges file-based agent definitions with GlobalStateManager for Pantheon
//! team assembly. Provides:
//! - `boot_fleet_agents()` — registers known fleet agents on server startup
//! - `POST /v1/fleet/register` — self-registration for remote agents
//! - `POST /v1/fleet/:id/heartbeat` — keep-alive heartbeat
//! - `GET /v1/fleet` — list registered fleet agents with runtime state
//! - `DELETE /v1/fleet/:id` — deregister an agent

use axum::extract::{Json, Path, Query, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use zeus_orchestra::state::{AgentState, AgentStatus};

use crate::api_key::constant_time_eq;
use crate::SharedState;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterAgentRequest {
    /// Unique agent ID (e.g. "zeus-112", "zeus-100")
    pub id: String,
    /// Human-readable name (e.g. "Zeus112 — MacBook Pro")
    pub name: String,
    /// Agent capabilities for task matching
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional metadata (IP, machine, role, etc.)
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    /// Optional status update
    #[serde(default)]
    pub status: Option<String>,
    /// Optional health score (0.0 - 1.0)
    #[serde(default)]
    pub health: Option<f32>,
    /// Optional load percentage (0.0 - 1.0)
    #[serde(default)]
    pub load: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct FleetAgentResponse {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub health_score: f32,
    pub load_pct: f32,
    pub last_heartbeat: String,
    pub registered_at: String,
    pub metadata: std::collections::HashMap<String, String>,
    /// S66-P4C: Current task description (from AgentStatus::Busy or current_task field)
    pub current_task: Option<String>,
}

impl From<AgentState> for FleetAgentResponse {
    fn from(a: AgentState) -> Self {
        // Extract current task from Busy status or the explicit field
        let current_task = a.current_task.clone().or_else(|| {
            match &a.status {
                AgentStatus::Busy(task) if !task.is_empty() => Some(task.clone()),
                _ => None,
            }
        });
        Self {
            id: a.id,
            name: a.name,
            capabilities: a.capabilities,
            status: a.status.to_string(),
            health_score: a.health_score,
            load_pct: a.load_pct,
            last_heartbeat: a.last_heartbeat.to_rfc3339(),
            registered_at: a.registered_at.to_rfc3339(),
            metadata: a.metadata,
            current_task,
        }
    }
}

// ---------------------------------------------------------------------------
// Fleet bootstrap — registers known agents on server startup
// ---------------------------------------------------------------------------

/// Known fleet agent definitions.
/// These are registered automatically when the API server starts.
/// Agent IPs are read from environment variables (ZEUS_FLEET_<ID>_IP) to avoid
/// hardcoding network-specific values. Falls back to "unknown" if not set.
type FleetDef = (String, String, Vec<&'static str>, Vec<(String, String)>);

fn fleet_agent_ip(agent_suffix: &str) -> String {
    let env_key = format!("ZEUS_FLEET_{}_IP", agent_suffix.to_uppercase());
    std::env::var(&env_key).unwrap_or_else(|_| "unknown".to_string())
}

/// Fleet subnet prefix for federation URL validation.
/// Reads from `ZEUS_FLEET_SUBNET` env var, defaults to `192.168.1.`.
fn fleet_subnet() -> String {
    std::env::var("ZEUS_FLEET_SUBNET").unwrap_or_else(|_| "192.168.1.".to_string())
}

fn fleet_definitions() -> Vec<FleetDef> {
    vec![
        (
            "zeus-112".to_string(),
            "Zeus112 — MacBook Pro".to_string(),
            vec![
                "coordinate",
                "code",
                "review",
                "plan",
                "deploy",
                "backend",
                "shell",
                "read_file",
                "write_file",
                "edit_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("112")),
                ("role".to_string(), "coordinator".to_string()),
                ("machine".to_string(), "MacBook Pro".to_string()),
            ],
        ),
        (
            "zeus-100".to_string(),
            "Zeus100 — Mac Mini M5".to_string(),
            vec![
                "code",
                "frontend",
                "ios",
                "macos",
                "swift",
                "design",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("100")),
                ("role".to_string(), "frontend-coordinator".to_string()),
                ("machine".to_string(), "Mac Mini M5".to_string()),
            ],
        ),
        (
            "zeus-102".to_string(),
            "ZeusMarketing — Mac Mini M2".to_string(),
            vec![
                "marketing",
                "social",
                "blog",
                "content",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("102")),
                ("role".to_string(), "marketing".to_string()),
                ("machine".to_string(), "Mac Mini M2".to_string()),
            ],
        ),
        (
            "zeus-106".to_string(),
            "Z (zeusmolty) — Mac Studio M1 Ultra".to_string(),
            vec![
                "code",
                "api",
                "backend",
                "websocket",
                "database",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("106")),
                ("role".to_string(), "backend".to_string()),
                ("machine".to_string(), "Mac Studio M1 Ultra".to_string()),
            ],
        ),
        (
            "zeus-107".to_string(),
            "Zeus107 — Mac Mini M4 Pro".to_string(),
            vec![
                "code",
                "security",
                "tui",
                "test",
                "review",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("107")),
                ("role".to_string(), "security".to_string()),
                ("machine".to_string(), "Mac Mini M4 Pro".to_string()),
            ],
        ),
        (
            "fbsd1".to_string(),
            "fbsd1 — FreeBSD 15.0".to_string(),
            vec![
                "provision",
                "deploy",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("224")),
                ("role".to_string(), "provisioner".to_string()),
                ("machine".to_string(), "FreeBSD 15.0".to_string()),
            ],
        ),
        (
            "fbsd2".to_string(),
            "fbsd2 — FreeBSD 15.0".to_string(),
            vec![
                "web",
                "platform",
                "gateway",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("226")),
                ("role".to_string(), "web-platform".to_string()),
                ("machine".to_string(), "FreeBSD 15.0".to_string()),
            ],
        ),
        (
            "fbsd3".to_string(),
            "fbsd3 — FreeBSD 15.0".to_string(),
            vec![
                "monitor",
                "guard",
                "shell",
                "read_file",
                "write_file",
            ],
            vec![
                ("ip".to_string(), fleet_agent_ip("225")),
                ("role".to_string(), "loop-guard".to_string()),
                ("machine".to_string(), "FreeBSD 15.0".to_string()),
            ],
        ),
    ]
}

/// Register all known fleet agents in GlobalStateManager.
/// Called on API server startup. Idempotent — skips already-registered agents.
pub async fn boot_fleet_agents(state: &SharedState) {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    for (id, name, caps, meta) in fleet_definitions() {
        let agent_id = id.clone();
        let mut agent = AgentState::new(id, name)
            .with_capabilities(caps.into_iter().map(String::from).collect());
        for (k, v) in meta {
            agent.metadata.insert(k, v);
        }

        match gsm.register_agent(agent).await {
            Ok(()) => info!(agent_id = %agent_id, "Fleet agent registered"),
            Err(_) => {
                // Already registered — update heartbeat
                let _ = gsm.heartbeat(&agent_id).await;
            }
        }
    }

    let count = gsm.agent_count().await;
    info!("Fleet bootstrap complete: {} agents registered", count);
}

// ---------------------------------------------------------------------------
// Stale agent cleanup — mark agents with expired heartbeats as Offline
// ---------------------------------------------------------------------------

/// Mark agents whose last heartbeat exceeds `max_age` as Offline.
/// Called on gateway startup and can be invoked periodically.
pub async fn cleanup_stale_agents(state: &SharedState, max_age: std::time::Duration) {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let stale = gsm.stale_agents(max_age).await;
    if stale.is_empty() {
        return;
    }

    for agent in &stale {
        if let Err(e) = gsm.update_status(&agent.id, AgentStatus::Offline).await {
            tracing::warn!(agent_id = %agent.id, error = %e, "Failed to mark stale agent offline");
        } else {
            tracing::info!(
                agent_id = %agent.id,
                last_heartbeat = %agent.last_heartbeat,
                "Marked stale agent as Offline"
            );
        }
    }

    tracing::info!("{} stale agents marked Offline", stale.len());
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /v1/fleet/register — Self-registration for remote agents
pub async fn register_agent(
    State(state): State<SharedState>,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let mut agent = AgentState::new(&req.id, &req.name).with_capabilities(req.capabilities);
    agent.metadata = req.metadata;

    match gsm.register_agent(agent).await {
        Ok(()) => {
            info!(agent_id = %req.id, name = %req.name, "Agent registered via API");
            Ok(Json(json!({
                "status": "registered",
                "id": req.id,
            })))
        }
        Err(e) => {
            // Already registered — update heartbeat instead
            let _ = gsm.heartbeat(&req.id).await;
            Ok(Json(json!({
                "status": "already_registered",
                "id": req.id,
                "message": e.to_string(),
            })))
        }
    }
}

/// POST /v1/fleet/:id/heartbeat — Agent heartbeat with optional status/health update
pub async fn fleet_heartbeat(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    // Update heartbeat timestamp
    gsm.heartbeat(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Apply optional updates
    if let Some(health) = req.health {
        let _ = gsm.update_health(&id, health).await;
    }
    if let Some(load) = req.load {
        let _ = gsm.update_load(&id, load).await;
    }
    if let Some(ref status) = req.status {
        let agent_status = match status.as_str() {
            "idle" => AgentStatus::Idle,
            "offline" => AgentStatus::Offline,
            s if s.starts_with("busy:") => AgentStatus::Busy(s[5..].trim().to_string()),
            s if s.starts_with("error:") => AgentStatus::Error(s[6..].trim().to_string()),
            _ => AgentStatus::Busy(status.clone()),
        };
        let _ = gsm.update_status(&id, agent_status).await;
    }

    Ok(Json(json!({
        "status": "ok",
        "id": id,
    })))
}

/// GET /v1/fleet — List all registered fleet agents with runtime state
pub async fn list_fleet_agents(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let agents: Vec<FleetAgentResponse> = gsm
        .list_agents()
        .await
        .into_iter()
        .map(FleetAgentResponse::from)
        .collect();

    let idle_count = agents.iter().filter(|a| a.status == "idle").count();
    let busy_count = agents
        .iter()
        .filter(|a| a.status.starts_with("busy"))
        .count();

    Json(json!({
        "agents": agents,
        "total": agents.len(),
        "idle": idle_count,
        "busy": busy_count,
    }))
}

/// GET /v1/agents/discover — searchable agent registry
///
/// Query params:
/// - `capability` — filter by capability substring (e.g. `code`, `review`)
/// - `status`     — filter by status (e.g. `idle`, `busy`, `online`)
/// - `q`          — free-text search across name + capabilities
#[derive(Debug, Deserialize, Default)]
pub struct DiscoverQuery {
    pub capability: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
}

pub async fn discover_agents(
    State(state): State<SharedState>,
    Query(params): Query<DiscoverQuery>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let all: Vec<FleetAgentResponse> = gsm
        .list_agents()
        .await
        .into_iter()
        .map(FleetAgentResponse::from)
        .collect();

    let cap_filter = params.capability.as_deref().unwrap_or("").to_lowercase();
    let stat_filter = params.status.as_deref().unwrap_or("").to_lowercase();
    let q_filter = params.q.as_deref().unwrap_or("").to_lowercase();

    let matched: Vec<&FleetAgentResponse> = all
        .iter()
        .filter(|a| {
            // capability filter
            if !cap_filter.is_empty() {
                let has_cap = a
                    .capabilities
                    .iter()
                    .any(|c| c.to_lowercase().contains(&cap_filter));
                if !has_cap {
                    return false;
                }
            }
            // status filter
            if !stat_filter.is_empty() && !a.status.to_lowercase().contains(&stat_filter) {
                return false;
            }
            // free-text
            if !q_filter.is_empty() {
                let name_match = a.name.to_lowercase().contains(&q_filter);
                let cap_match = a
                    .capabilities
                    .iter()
                    .any(|c| c.to_lowercase().contains(&q_filter));
                let meta_match = a
                    .metadata
                    .values()
                    .any(|v| v.to_lowercase().contains(&q_filter));
                if !name_match && !cap_match && !meta_match {
                    return false;
                }
            }
            true
        })
        .collect();

    // Aggregate all unique capabilities across fleet for the discovery UI
    let mut all_caps: Vec<String> = all
        .iter()
        .flat_map(|a| a.capabilities.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_caps.sort();

    Json(json!({
        "agents": matched,
        "total": matched.len(),
        "fleet_size": all.len(),
        "capabilities": all_caps,
        "filters": {
            "capability": params.capability,
            "status": params.status,
            "q": params.q,
        }
    }))
}

/// GET /v1/fleet/:id — Get a single fleet agent's runtime state
pub async fn get_fleet_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let agent = gsm
        .get_agent(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Agent not found: {}", id)))?;

    Ok(Json(
        serde_json::to_value(FleetAgentResponse::from(agent)).unwrap_or(json!({})),
    ))
}

/// DELETE /v1/fleet/:id — Deregister an agent
pub async fn deregister_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    gsm.deregister_agent(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    info!(agent_id = %id, "Agent deregistered");
    Ok(Json(json!({
        "status": "deregistered",
        "id": id,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/fleet/execute — receive a remote task and execute it locally
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FleetExecuteRequest {
    /// Step ID for correlation
    #[serde(default)]
    pub step_id: Option<usize>,
    /// Human-readable task description (used as the shell command or intent)
    pub description: String,
    /// Optional tool name to invoke directly
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Optional tool arguments (JSON)
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

/// POST /v1/fleet/execute — Receive a task dispatched by a remote coordinator.
///
/// Runs the described task locally and returns the result.
/// Currently echoes the task details (executor is plugged in via zeus-agent).
pub async fn fleet_execute(
    State(state): State<SharedState>,
    Json(req): Json<FleetExecuteRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    info!(
        step_id = ?req.step_id,
        description = %req.description,
        tool_name = ?req.tool_name,
        "Fleet execute: received remote task"
    );

    // If a specific tool was requested, invoke it via the local tool registry
    if let (Some(ref tool_name), Some(args)) = (req.tool_name.as_ref(), req.arguments.as_ref()) {
        match state_guard.tools.execute(tool_name, args.clone()).await {
            Ok(output) => {
                return Ok(Json(json!({
                    "success": true,
                    "step_id": req.step_id,
                    "tool_name": tool_name,
                    "output": output,
                })));
            }
            Err(e) => {
                return Ok(Json(json!({
                    "success": false,
                    "step_id": req.step_id,
                    "tool_name": tool_name,
                    "error": e.to_string(),
                })));
            }
        }
    }

    // No tool invoked — acknowledge receipt (description-only tasks are tracked)
    Ok(Json(json!({
        "success": true,
        "step_id": req.step_id,
        "output": format!("task received: {}", req.description),
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/fleet/protocol — receive an inter-agent ProtocolMessage
// ---------------------------------------------------------------------------

/// POST /v1/fleet/protocol — Receive a ProtocolMessage from a remote agent.
///
/// Deserializes the message and injects it into the local Orchestra MessageBus
/// so local subscribers (e.g. coordination loops) can process it.
pub async fn fleet_protocol(
    State(state): State<SharedState>,
    Json(msg): Json<zeus_orchestra::ProtocolMessage>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    info!(
        from = %msg.sender(),
        to = ?msg.target(),
        "Fleet protocol: received inter-agent message"
    );

    // Inject into local Orchestra MessageBus if available
    if let Some(orchestra) = state_guard.orchestra.get() {
        let bus_msg = msg.to_bus_message();
        if let Err(e) = orchestra.bus().send(bus_msg).await {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to inject into MessageBus: {}", e),
            ));
        }
    }

    Ok(Json(json!({ "status": "received" })))
}

// ---------------------------------------------------------------------------
// POST /v1/agents/:id/invoke — Remote agent skill invocation with escrow
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InvokeAgentRequest {
    /// ID of the agent requesting the invocation (the buyer)
    pub caller_id: String,
    /// Skill to invoke on the target agent
    pub skill_name: String,
    /// Input payload for the skill (JSON)
    #[serde(default)]
    pub input: serde_json::Value,
    /// Maximum credits the caller is willing to spend (0 = no limit)
    #[serde(default)]
    pub max_credits: u64,
    /// Timeout in seconds (default: 60)
    #[serde(default = "default_invoke_timeout")]
    pub timeout_secs: u64,
}

fn default_invoke_timeout() -> u64 {
    60
}

/// POST /v1/agents/:id/invoke — Invoke a skill on a remote agent.
///
/// End-to-end flow:
/// 1. Validate caller + target agent exist in fleet
/// 2. Look up skill listing in marketplace (price)
/// 3. Escrow: debit caller's wallet
/// 4. Forward invocation to target (via NodeWS or HTTP)
/// 5. On success: credit seller, complete transaction
/// 6. On failure: refund caller
pub async fn invoke_agent(
    State(state): State<SharedState>,
    Path(target_agent_id): Path<String>,
    Json(req): Json<InvokeAgentRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let start = std::time::Instant::now();
    let tx_id = uuid::Uuid::new_v4().to_string();

    info!(
        target = %target_agent_id,
        caller = %req.caller_id,
        skill = %req.skill_name,
        tx = %tx_id,
        "Agent invoke: starting"
    );

    // Step 1: Validate target agent exists
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let target = gsm.get_agent(&target_agent_id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Target agent not found: {}", target_agent_id),
        )
    })?;

    // Check target has the requested capability
    if !target.has_capability(&req.skill_name) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Agent '{}' does not have capability '{}'",
                target_agent_id, req.skill_name
            ),
        ));
    }

    // Check target is available
    if matches!(target.status, AgentStatus::Offline) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Agent '{}' is offline", target_agent_id),
        ));
    }

    // Step 2: Determine price from marketplace listing or default
    // web4 P0-1c cut-8: read listings from marketplace_store (source of truth)
    // instead of the in-memory registry. SkillListingRow has the same
    // name/price fields used here, so pricing behavior is unchanged.
    let price: u64 = {
        let listings = state_guard
            .marketplace_store
            .search_by_publisher(&target_agent_id)
            .await;
        listings
            .iter()
            .find(|l| {
                l.name
                    .to_lowercase()
                    .contains(&req.skill_name.to_lowercase())
            })
            .map(|l| l.price)
            .unwrap_or(10) // default 10 credits for unlisted skills
    };

    // Check max_credits limit
    if req.max_credits > 0 && price > req.max_credits {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            format!(
                "Skill costs {} credits, exceeds max_credits limit of {}",
                price, req.max_credits
            ),
        ));
    }

    // Step 3: Escrow — debit caller's wallet
    state_guard
        .ledger
        .spend(
            &req.caller_id,
            price,
            zeus_economy::TransactionReason::MarketplaceTrade,
            format!(
                "Escrow for {}:{} (tx:{})",
                target_agent_id, req.skill_name, tx_id
            ),
        )
        .map_err(|e| {
            (
                StatusCode::PAYMENT_REQUIRED,
                format!("Escrow failed: {}", e),
            )
        })?;

    // Step 4: Forward invocation to target agent
    let node_registry = state_guard.node_registry.clone();
    let timeout = std::time::Duration::from_secs(req.timeout_secs);
    drop(state_guard); // release read lock before async invoke

    let invoke_result = node_registry
        .invoke(
            &target_agent_id,
            &req.skill_name,
            serde_json::json!({
                "skill_name": req.skill_name,
                "input": req.input,
                "caller_id": req.caller_id,
                "transaction_id": tx_id,
            }),
            timeout,
        )
        .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match invoke_result {
        Ok(output) => {
            // Step 5a: Success — credit seller
            let state_guard = state.read().await;
            let _ = state_guard.ledger.earn(
                &target_agent_id,
                price,
                zeus_economy::TransactionReason::MarketplaceSale,
                format!(
                    "Sale: {} to {} (tx:{})",
                    req.skill_name, req.caller_id, tx_id
                ),
            );

            info!(
                tx = %tx_id,
                credits = price,
                elapsed_ms = elapsed_ms,
                "Agent invoke: completed successfully"
            );

            Ok(Json(serde_json::json!({
                "success": true,
                "transaction_id": tx_id,
                "agent_id": target_agent_id,
                "skill_name": req.skill_name,
                "output": output,
                "credits_charged": price,
                "elapsed_ms": elapsed_ms,
            })))
        }
        Err(e) => {
            // Step 5b: Failure — refund caller
            let state_guard = state.read().await;
            let _ = state_guard.ledger.earn(
                &req.caller_id,
                price,
                zeus_economy::TransactionReason::Custom("escrow_refund".into()),
                format!(
                    "Refund for failed invoke: {}:{} (tx:{})",
                    target_agent_id, req.skill_name, tx_id
                ),
            );

            tracing::warn!(
                tx = %tx_id,
                error = %e,
                "Agent invoke: failed, refunded {} credits to {}",
                price,
                req.caller_id
            );

            // Not connected via WebSocket — try HTTP fallback
            if e.contains("not connected") || e.contains("not found") {
                return invoke_agent_http(
                    &state,
                    &target_agent_id,
                    &target,
                    &req,
                    &tx_id,
                    price,
                    start,
                )
                .await;
            }

            Ok(Json(serde_json::json!({
                "success": false,
                "transaction_id": tx_id,
                "agent_id": target_agent_id,
                "skill_name": req.skill_name,
                "error": e,
                "credits_charged": 0,
                "credits_refunded": price,
                "elapsed_ms": elapsed_ms,
            })))
        }
    }
}

/// HTTP fallback for agent invocation when target is not connected via WebSocket.
/// Uses the agent's endpoint_url metadata to forward the request via HTTP.
async fn invoke_agent_http(
    state: &SharedState,
    target_agent_id: &str,
    target: &AgentState,
    req: &InvokeAgentRequest,
    tx_id: &str,
    price: u64,
    start: std::time::Instant,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let endpoint_url = target
        .metadata
        .get("endpoint_url")
        .or_else(|| target.metadata.get("ip"))
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                format!(
                    "Agent '{}' not connected via WebSocket and has no endpoint_url",
                    target_agent_id
                ),
            )
        })?;

    // Build the target URL
    let base = if endpoint_url.starts_with("http") {
        endpoint_url.clone()
    } else {
        format!("http://{}:3001", endpoint_url)
    };
    let url = format!("{}/v1/fleet/execute", base);

    info!(
        tx = %tx_id,
        url = %url,
        "Agent invoke: falling back to HTTP"
    );

    let client = reqwest::Client::new();
    let http_result = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(req.timeout_secs.min(120)))
        .json(&serde_json::json!({
            "description": format!("Invoke skill: {}", req.skill_name),
            "tool_name": req.skill_name,
            "arguments": req.input,
        }))
        .send()
        .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match http_result {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();

            // Credit seller on success
            let state_guard = state.read().await;
            let _ = state_guard.ledger.earn(
                target_agent_id,
                price,
                zeus_economy::TransactionReason::MarketplaceSale,
                format!(
                    "Sale (HTTP): {} to {} (tx:{})",
                    req.skill_name, req.caller_id, tx_id
                ),
            );

            Ok(Json(serde_json::json!({
                "success": true,
                "transaction_id": tx_id,
                "agent_id": target_agent_id,
                "skill_name": req.skill_name,
                "output": body,
                "credits_charged": price,
                "elapsed_ms": elapsed_ms,
                "transport": "http",
            })))
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err((
                StatusCode::BAD_GATEWAY,
                format!("Remote agent returned {}: {} (tx:{})", status, body, tx_id),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "HTTP invoke to {} failed: {} (tx:{})",
                target_agent_id, e, tx_id
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/economy/stake — Stake tokens as collateral
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StakeRequest {
    /// Agent staking tokens
    pub agent_id: String,
    /// Amount to stake
    pub amount: u64,
    /// Purpose of the stake (e.g. "marketplace_listing", "task_guarantee")
    #[serde(default = "default_stake_purpose")]
    pub purpose: String,
}

fn default_stake_purpose() -> String {
    "general".into()
}

#[derive(Debug, Deserialize)]
pub struct UnstakeRequest {
    /// Agent unstaking tokens
    pub agent_id: String,
    /// Amount to unstake
    pub amount: u64,
    /// Stake ID to unstake from
    pub stake_id: String,
}

/// POST /v1/economy/stake — Stake tokens as collateral for marketplace participation
pub async fn economy_stake(
    State(state): State<SharedState>,
    Json(req): Json<StakeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".to_string()));
    }

    let state_guard = state.read().await;

    // Debit the agent's wallet (hold as collateral)
    state_guard
        .ledger
        .spend(
            &req.agent_id,
            req.amount,
            zeus_economy::TransactionReason::Custom(format!("stake:{}", req.purpose)),
            format!("Staked {} tokens for {}", req.amount, req.purpose),
        )
        .map_err(|e| (StatusCode::PAYMENT_REQUIRED, format!("Stake failed: {}", e)))?;

    let stake_id = uuid::Uuid::new_v4().to_string();

    info!(
        agent = %req.agent_id,
        amount = req.amount,
        stake_id = %stake_id,
        purpose = %req.purpose,
        "Economy: tokens staked"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "stake_id": stake_id,
        "agent_id": req.agent_id,
        "amount": req.amount,
        "purpose": req.purpose,
    })))
}

/// POST /v1/economy/unstake — Release staked tokens back to wallet
pub async fn economy_unstake(
    State(state): State<SharedState>,
    Json(req): Json<UnstakeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".to_string()));
    }

    let state_guard = state.read().await;

    // Credit back to the agent's wallet
    state_guard
        .ledger
        .earn(
            &req.agent_id,
            req.amount,
            zeus_economy::TransactionReason::Custom(format!("unstake:{}", req.stake_id)),
            format!("Unstaked {} tokens (stake:{})", req.amount, req.stake_id),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unstake failed: {}", e),
            )
        })?;

    info!(
        agent = %req.agent_id,
        amount = req.amount,
        stake_id = %req.stake_id,
        "Economy: tokens unstaked"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "stake_id": req.stake_id,
        "agent_id": req.agent_id,
        "amount_released": req.amount,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/economy/transfer — Direct peer-to-peer token transfer
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    /// Sender agent ID
    pub from: String,
    /// Receiver agent ID
    pub to: String,
    /// Amount to transfer
    pub amount: u64,
    /// Optional note
    #[serde(default)]
    pub note: Option<String>,
}

/// POST /v1/economy/transfer — Transfer tokens between agents
///
/// Requires `Authorization: Bearer <ZEUS_TRANSFER_ADMIN_TOKEN>` header.
/// Returns 401 if the header is absent, 403 if the token is invalid.
/// If `ZEUS_TRANSFER_ADMIN_TOKEN` is not set, the endpoint is permanently
/// disabled (fails closed). This is an interim admin gate: it closes the
/// unauthenticated-transfer hole until per-caller authz (authenticated
/// session → `from`) lands in Phase 2. Uses a token distinct from the mint
/// admin token (`ZEUS_MINT_ADMIN_TOKEN`) — distinct privilege, distinct rotation.
pub async fn economy_transfer(
    headers: HeaderMap,
    State(state): State<SharedState>,
    Json(req): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // --- Auth gate (interim, #190): fails closed when unconfigured ---
    let admin_token = std::env::var("ZEUS_TRANSFER_ADMIN_TOKEN").unwrap_or_default();
    if admin_token.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Transfer endpoint disabled: ZEUS_TRANSFER_ADMIN_TOKEN not configured".to_string(),
        ));
    }
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if bearer.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "Authorization header required".to_string()));
    }
    if !constant_time_eq(bearer, &admin_token) {
        return Err((StatusCode::FORBIDDEN, "Invalid transfer admin token".to_string()));
    }

    if req.amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".to_string()));
    }
    if req.from == req.to {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot transfer to self".to_string(),
        ));
    }

    let state_guard = state.read().await;
    let note = req.note.as_deref().unwrap_or("Peer transfer");

    let (from_balance, to_balance) = state_guard
        .ledger
        .transfer(
            &req.from,
            &req.to,
            req.amount,
            zeus_economy::TransactionReason::PeerTransfer,
            note,
        )
        .map_err(|e| {
            (
                StatusCode::PAYMENT_REQUIRED,
                format!("Transfer failed: {}", e),
            )
        })?;

    info!(
        from = %req.from,
        to = %req.to,
        amount = req.amount,
        "Economy: peer transfer"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "from": { "agent_id": req.from, "balance": from_balance },
        "to": { "agent_id": req.to, "balance": to_balance },
        "amount": req.amount,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/economy/mint — Admin token minting
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MintRequest {
    /// Agent to receive minted tokens
    pub agent_id: String,
    /// Amount to mint
    pub amount: u64,
    /// Reason for minting
    #[serde(default = "default_mint_reason")]
    pub reason: String,
}

fn default_mint_reason() -> String {
    "admin_grant".into()
}



/// POST /v1/economy/mint — Mint new tokens into an agent's wallet (admin)
///
/// Requires `Authorization: Bearer <ZEUS_MINT_ADMIN_TOKEN>` header.
/// Returns 401 if the header is absent, 403 if the token is invalid.
/// If `ZEUS_MINT_ADMIN_TOKEN` is not set, the endpoint is permanently disabled.
pub async fn economy_mint(
    headers: HeaderMap,
    State(state): State<SharedState>,
    Json(req): Json<MintRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // --- Auth gate (E4) ---
    let admin_token = std::env::var("ZEUS_MINT_ADMIN_TOKEN").unwrap_or_default();
    if admin_token.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mint endpoint disabled: ZEUS_MINT_ADMIN_TOKEN not configured".to_string(),
        ));
    }
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if bearer.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "Authorization header required".to_string()));
    }
    if !constant_time_eq(bearer, &admin_token) {
        return Err((StatusCode::FORBIDDEN, "Invalid mint admin token".to_string()));
    }

    if req.amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".to_string()));
    }

    let state_guard = state.read().await;

    let reason = match req.reason.as_str() {
        "system_grant" | "admin_grant" => zeus_economy::TransactionReason::SystemGrant,
        "task_completion" => zeus_economy::TransactionReason::TaskCompletion,
        "review_reward" => zeus_economy::TransactionReason::ReviewReward,
        other => zeus_economy::TransactionReason::Custom(other.to_string()),
    };

    state_guard
        .ledger
        .mint(
            &req.agent_id,
            req.amount,
            reason,
            format!("Minted {} tokens: {}", req.amount, req.reason),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Mint failed: {}", e),
            )
        })?;

    let wallet = state_guard
        .ledger
        .wallet(&req.agent_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        agent = %req.agent_id,
        amount = req.amount,
        reason = %req.reason,
        new_balance = wallet.balance,
        "Economy: tokens minted"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "agent_id": req.agent_id,
        "amount_minted": req.amount,
        "new_balance": wallet.balance,
        "reason": req.reason,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/agents/hire — Hire an agent (discovery + invoke shorthand)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HireAgentRequest {
    /// ID of the hiring agent (buyer)
    pub caller_id: String,
    /// Task description for the hired agent
    pub task: String,
    /// Specific skill to invoke (optional — auto-matched if omitted)
    #[serde(default)]
    pub skill_name: Option<String>,
    /// Input payload for the task
    #[serde(default)]
    pub input: serde_json::Value,
    /// Maximum credits willing to spend
    #[serde(default)]
    pub max_credits: u64,
}

/// POST /v1/agents/hire — Hire the best available agent for a task.
///
/// Combines agent discovery (best_for_task) with skill invocation.
/// If no specific agent is targeted, finds the best match from the fleet.
pub async fn hire_agent(
    State(state): State<SharedState>,
    Json(req): Json<HireAgentRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    // Determine which skill to look for
    let skill = req.skill_name.as_deref().unwrap_or(&req.task);

    // Find the best agent for this skill
    let best = gsm.best_for_task(skill).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("No available agent found for task: {}", skill),
        )
    })?;

    let target_id = best.id.clone();
    drop(state_guard);

    info!(
        caller = %req.caller_id,
        target = %target_id,
        task = %req.task,
        "Hire agent: matched"
    );

    // Delegate to invoke_agent
    let invoke_req = InvokeAgentRequest {
        caller_id: req.caller_id,
        skill_name: req.skill_name.unwrap_or_else(|| req.task.clone()),
        input: req.input,
        max_credits: req.max_credits,
        timeout_secs: 60,
    };

    invoke_agent(State(state), Path(target_id), Json(invoke_req)).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn test_fleet_definitions_not_empty() {
        let defs = fleet_definitions();
        assert_eq!(defs.len(), 8);
    }

    #[test]
    fn test_fleet_definitions_have_capabilities() {
        for (id, _name, caps, _meta) in fleet_definitions() {
            assert!(!caps.is_empty(), "Agent {} has no capabilities", id);
        }
    }

    #[test]
    fn test_fleet_definitions_have_metadata() {
        for (id, _name, _caps, meta) in fleet_definitions() {
            let keys: Vec<_> = meta.iter().map(|(k, _)| k.as_str()).collect();
            assert!(keys.contains(&"ip"), "Agent {} missing ip metadata", id);
            assert!(keys.contains(&"role"), "Agent {} missing role metadata", id);
        }
    }

    #[test]
    fn test_fleet_agent_response_from_state() {
        let mut agent = AgentState::new("test-1", "Test Agent")
            .with_capabilities(vec!["code".into(), "review".into()]);
        agent.metadata.insert("ip".into(), "1.2.3.4".into());

        let resp = FleetAgentResponse::from(agent);
        assert_eq!(resp.id, "test-1");
        assert_eq!(resp.name, "Test Agent");
        assert_eq!(resp.capabilities, vec!["code", "review"]);
        assert_eq!(resp.status, "idle");
        assert_eq!(resp.metadata.get("ip").unwrap(), "1.2.3.4");
    }

    #[tokio::test]
    async fn test_boot_fleet_agents_registers_all() {
        use zeus_orchestra::GlobalStateManager;
        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        for (id, name, caps, meta) in fleet_definitions() {
            let mut agent = AgentState::new(id, name)
                .with_capabilities(caps.into_iter().map(String::from).collect());
            for (k, v) in meta {
                agent.metadata.insert(k.to_string(), v.to_string());
            }
            gsm.register_agent(agent).await.unwrap();
        }

        assert_eq!(gsm.agent_count().await, 8);

        // Verify specific agents
        let z112 = gsm.get_agent("zeus-112").await.unwrap();
        assert!(z112.has_capability("coordinate"));
        assert!(z112.has_capability("code"));

        let z107 = gsm.get_agent("zeus-107").await.unwrap();
        assert!(z107.has_capability("security"));
        assert!(z107.has_capability("test"));
    }

    #[tokio::test]
    async fn test_best_for_task_finds_coordinator() {
        use zeus_orchestra::GlobalStateManager;
        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        for (id, name, caps, meta) in fleet_definitions() {
            let mut agent = AgentState::new(id, name)
                .with_capabilities(caps.into_iter().map(String::from).collect());
            for (k, v) in meta {
                agent.metadata.insert(k.to_string(), v.to_string());
            }
            gsm.register_agent(agent).await.unwrap();
        }

        let coordinator = gsm.best_for_task("coordinate").await;
        assert!(coordinator.is_some());
        assert_eq!(coordinator.unwrap().id, "zeus-112");
    }

    #[tokio::test]
    async fn test_best_for_task_finds_security_specialist() {
        use zeus_orchestra::GlobalStateManager;
        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        for (id, name, caps, meta) in fleet_definitions() {
            let mut agent = AgentState::new(id, name)
                .with_capabilities(caps.into_iter().map(String::from).collect());
            for (k, v) in meta {
                agent.metadata.insert(k.to_string(), v.to_string());
            }
            gsm.register_agent(agent).await.unwrap();
        }

        let security = gsm.best_for_task("security").await;
        assert!(security.is_some());
        assert_eq!(security.unwrap().id, "zeus-107");
    }

    #[tokio::test]
    async fn test_best_for_task_finds_frontend() {
        use zeus_orchestra::GlobalStateManager;
        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        for (id, name, caps, meta) in fleet_definitions() {
            let mut agent = AgentState::new(id, name)
                .with_capabilities(caps.into_iter().map(String::from).collect());
            for (k, v) in meta {
                agent.metadata.insert(k.to_string(), v.to_string());
            }
            gsm.register_agent(agent).await.unwrap();
        }

        let frontend = gsm.best_for_task("frontend").await;
        assert!(frontend.is_some());
        assert_eq!(frontend.unwrap().id, "zeus-100");
    }

    #[tokio::test]
    async fn test_cleanup_stale_agents() {
        use zeus_orchestra::GlobalStateManager;

        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        // Register a fresh agent
        gsm.register_agent(AgentState::new("fresh-1", "Fresh Agent"))
            .await
            .unwrap();

        // Register a stale agent with old heartbeat
        let mut stale = AgentState::new("stale-1", "Stale Agent");
        stale.last_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(700);
        gsm.register_agent(stale).await.unwrap();

        // Find stale agents (> 600s)
        let stale_agents = gsm.stale_agents(std::time::Duration::from_secs(600)).await;
        assert_eq!(stale_agents.len(), 1);
        assert_eq!(stale_agents[0].id, "stale-1");

        // Mark stale as offline
        for a in &stale_agents {
            gsm.update_status(&a.id, AgentStatus::Offline)
                .await
                .unwrap();
        }

        let agent = gsm.get_agent("stale-1").await.unwrap();
        assert!(matches!(agent.status, AgentStatus::Offline));

        // Fresh agent should be unaffected
        let fresh = gsm.get_agent("fresh-1").await.unwrap();
        assert!(matches!(fresh.status, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_idempotent_registration() {
        use zeus_orchestra::GlobalStateManager;
        let gsm = std::sync::Arc::new(GlobalStateManager::new());

        let agent = AgentState::new("zeus-112", "Zeus112").with_capabilities(vec!["code".into()]);
        gsm.register_agent(agent).await.unwrap();

        // Second registration should fail
        let agent2 =
            AgentState::new("zeus-112", "Zeus112 duplicate").with_capabilities(vec!["code".into()]);
        assert!(gsm.register_agent(agent2).await.is_err());

        // But heartbeat should work
        gsm.heartbeat("zeus-112").await.unwrap();
        assert_eq!(gsm.agent_count().await, 1);
    }

    #[test]
    fn test_invoke_agent_request_deserialize() {
        let json = r#"{
            "caller_id": "zeus-112",
            "skill_name": "code_review",
            "input": {"file": "main.rs"},
            "max_credits": 50,
            "timeout_secs": 30
        }"#;
        let req: InvokeAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.caller_id, "zeus-112");
        assert_eq!(req.skill_name, "code_review");
        assert_eq!(req.max_credits, 50);
        assert_eq!(req.timeout_secs, 30);
    }

    #[test]
    fn test_invoke_agent_request_defaults() {
        let json = r#"{
            "caller_id": "zeus-112",
            "skill_name": "deploy"
        }"#;
        let req: InvokeAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.max_credits, 0);
        assert_eq!(req.timeout_secs, 60);
        assert_eq!(req.input, serde_json::Value::Null);
    }

    #[test]
    fn test_stake_request_deserialize() {
        let json = r#"{
            "agent_id": "zeus-100",
            "amount": 500,
            "purpose": "marketplace_listing"
        }"#;
        let req: StakeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "zeus-100");
        assert_eq!(req.amount, 500);
        assert_eq!(req.purpose, "marketplace_listing");
    }

    #[test]
    fn test_stake_request_default_purpose() {
        let json = r#"{"agent_id": "a", "amount": 100}"#;
        let req: StakeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.purpose, "general");
    }

    #[test]
    fn test_unstake_request_deserialize() {
        let json = r#"{
            "agent_id": "zeus-100",
            "amount": 200,
            "stake_id": "abc-123"
        }"#;
        let req: UnstakeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "zeus-100");
        assert_eq!(req.amount, 200);
        assert_eq!(req.stake_id, "abc-123");
    }

    #[test]
    fn test_transfer_request_deserialize() {
        let json = r#"{
            "from": "zeus-112",
            "to": "zeus-100",
            "amount": 50,
            "note": "For code review"
        }"#;
        let req: TransferRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.from, "zeus-112");
        assert_eq!(req.to, "zeus-100");
        assert_eq!(req.amount, 50);
        assert_eq!(req.note.as_deref(), Some("For code review"));
    }

    #[test]
    fn test_transfer_request_no_note() {
        let json = r#"{"from": "a", "to": "b", "amount": 10}"#;
        let req: TransferRequest = serde_json::from_str(json).unwrap();
        assert!(req.note.is_none());
    }

    #[test]
    fn test_mint_request_deserialize() {
        let json = r#"{"agent_id": "zeus-100", "amount": 1000, "reason": "system_grant"}"#;
        let req: MintRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "zeus-100");
        assert_eq!(req.amount, 1000);
        assert_eq!(req.reason, "system_grant");
    }

    #[test]
    fn test_mint_request_default_reason() {
        let json = r#"{"agent_id": "a", "amount": 50}"#;
        let req: MintRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.reason, "admin_grant");
    }

    #[test]
    fn test_hire_agent_request_deserialize() {
        let json = r#"{
            "caller_id": "zeus-112",
            "task": "review my code",
            "skill_name": "code_review",
            "max_credits": 100
        }"#;
        let req: HireAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.caller_id, "zeus-112");
        assert_eq!(req.task, "review my code");
        assert_eq!(req.skill_name.as_deref(), Some("code_review"));
        assert_eq!(req.max_credits, 100);
    }

    #[test]
    fn test_hire_agent_request_minimal() {
        let json = r#"{"caller_id": "a", "task": "deploy"}"#;
        let req: HireAgentRequest = serde_json::from_str(json).unwrap();
        assert!(req.skill_name.is_none());
        assert_eq!(req.max_credits, 0);
        assert_eq!(req.input, serde_json::Value::Null);
    }

    // ── Phase 5: Earning formula ──────────────────────────────────

    #[test]
    fn test_calculate_earning_simple() {
        let cfg = zeus_core::EconomyConfig::default();
        assert_eq!(calculate_earning(0, "simple", &cfg), 10); // base only
    }

    #[test]
    fn test_calculate_earning_with_tools() {
        let cfg = zeus_core::EconomyConfig::default();
        assert_eq!(calculate_earning(5, "simple", &cfg), 20); // 10 + 5*2
    }

    #[test]
    fn test_calculate_earning_moderate() {
        let cfg = zeus_core::EconomyConfig::default();
        assert_eq!(calculate_earning(3, "moderate", &cfg), 26); // 10 + 6 + 10
    }

    #[test]
    fn test_calculate_earning_complex() {
        let cfg = zeus_core::EconomyConfig::default();
        assert_eq!(calculate_earning(10, "complex", &cfg), 55); // 10 + 20 + 25
    }

    #[test]
    fn test_calculate_earning_unknown_complexity() {
        let cfg = zeus_core::EconomyConfig::default();
        assert_eq!(calculate_earning(0, "unknown", &cfg), 10); // falls back to simple
    }

    #[test]
    fn test_calculate_earning_custom_config() {
        let cfg = zeus_core::EconomyConfig {
            earning_base: 50,
            earning_tool_bonus: 5,
            earning_moderate_bonus: 20,
            earning_complex_bonus: 100,
        };
        assert_eq!(calculate_earning(3, "complex", &cfg), 165); // 50 + 15 + 100
    }

    // ── Phase 5: Request deserialization ──────────────────────────

    #[test]
    fn test_earn_request_deserialize() {
        let json = r#"{
            "agent_id": "zeus-100",
            "tools_used": 5,
            "complexity": "moderate",
            "reference": "session-abc",
            "note": "Built dashboard"
        }"#;
        let req: EarnRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "zeus-100");
        assert_eq!(req.tools_used, 5);
        assert_eq!(req.complexity, "moderate");
        assert_eq!(req.reference.as_deref(), Some("session-abc"));
        assert_eq!(req.note.as_deref(), Some("Built dashboard"));
    }

    #[test]
    fn test_earn_request_defaults() {
        let json = r#"{"agent_id": "zeus-112"}"#;
        let req: EarnRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tools_used, 0);
        assert_eq!(req.complexity, "simple");
        assert!(req.reference.is_none());
        assert!(req.note.is_none());
    }

    #[test]
    fn test_form_team_request_deserialize() {
        let json = r#"{
            "name": "Alpha Squad",
            "members": ["zeus-112", "zeus-100", "zeus-107"],
            "split_pct": [50, 30, 20]
        }"#;
        let req: FormTeamRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Alpha Squad");
        assert_eq!(req.members.len(), 3);
        assert_eq!(req.split_pct, vec![50, 30, 20]);
    }

    #[test]
    fn test_federation_invoke_request_deserialize() {
        let json = r#"{
            "caller_id": "zeus-112",
            "target_url": "http://192.168.1.100:3001",
            "skill_name": "code_review",
            "input": {"file": "main.rs"},
            "max_credits": 100
        }"#;
        let req: FederationInvokeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.caller_id, "zeus-112");
        assert_eq!(req.target_url, "http://192.168.1.100:3001");
        assert_eq!(req.skill_name, "code_review");
        assert_eq!(req.max_credits, 100);
        assert_eq!(req.timeout_secs, 30); // default
    }

    #[test]
    fn test_federation_invoke_request_defaults() {
        let json = r#"{
            "caller_id": "a",
            "target_url": "http://192.168.1.100:3001",
            "skill_name": "deploy"
        }"#;
        let req: FederationInvokeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.max_credits, 0);
        assert_eq!(req.timeout_secs, 30);
    }

    // ── Phase 5: Team validation ──────────────────────────────────

    #[test]
    fn test_form_team_split_validation() {
        // Valid: sums to 100
        let json = r#"{"name":"T","members":["a","b"],"split_pct":[60,40]}"#;
        let req: FormTeamRequest = serde_json::from_str(json).unwrap();
        let sum: u32 = req.split_pct.iter().sum();
        assert_eq!(sum, 100);

        // Invalid: sums to 90
        let json2 = r#"{"name":"T","members":["a","b"],"split_pct":[50,40]}"#;
        let req2: FormTeamRequest = serde_json::from_str(json2).unwrap();
        let sum2: u32 = req2.split_pct.iter().sum();
        assert_ne!(sum2, 100);
    }

    #[test]
    fn test_form_team_members_split_length_match() {
        let json = r#"{"name":"T","members":["a","b","c"],"split_pct":[33,33,34]}"#;
        let req: FormTeamRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.members.len(), req.split_pct.len());
    }

    // ── Phase 5b: Handler-level split_pct validation (S46-T2) ────

    fn test_state() -> SharedState {
        let config = zeus_core::Config::default();
        std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::AppState::new(config).unwrap(),
        ))
    }

    fn test_app(state: SharedState) -> axum::Router {
        crate::create_test_router(state)
    }

    #[tokio::test]
    async fn test_team_form_handler_rejects_split_not_100() {
        let state = test_state();
        let app = test_app(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/teams/form")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"Bad Split","members":["a","b"],"split_pct":[50,40]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("split_pct must sum to 100"),
            "Expected split_pct error, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_team_form_handler_rejects_length_mismatch() {
        let state = test_state();
        let app = test_app(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/teams/form")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"Mismatch","members":["a","b","c"],"split_pct":[50,50]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("same length"),
            "Expected length mismatch error, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_team_form_handler_rejects_empty_members() {
        let state = test_state();
        let app = test_app(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/teams/form")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"Empty","members":[],"split_pct":[]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("at least 1 member"),
            "Expected empty members error, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_team_form_handler_rejects_empty_name() {
        let state = test_state();
        let app = test_app(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/teams/form")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"","members":["a"],"split_pct":[100]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("1-100 chars"),
            "Expected name length error, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_team_form_handler_accepts_valid_request() {
        let state = test_state();
        let app = test_app(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/teams/form")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"Alpha","members":["zeus-100","zeus-112"],"split_pct":[60,40]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "Alpha");
        assert_eq!(json["members"], serde_json::json!(["zeus-100", "zeus-112"]));
        assert_eq!(json["split_pct"], serde_json::json!([60, 40]));
        assert!(json["id"].as_str().unwrap().starts_with("team-"));
        assert!(json["wallet_id"].as_str().unwrap().starts_with("wallet-"));
        assert_eq!(json["balance"], 0);
    }

    // ── Phase 5: AgentTeamRow persistence ─────────────────────────

    #[tokio::test]
    async fn test_agent_team_save_and_get() {
        use crate::handlers::marketplace_store::{AgentTeamRow, MarketplaceStore};
        let store = MarketplaceStore::in_memory().unwrap();

        let team = AgentTeamRow {
            id: "team-abc".into(),
            name: "Alpha Squad".into(),
            members_json: r#"["zeus-112","zeus-100"]"#.into(),
            split_pct_json: r#"[60,40]"#.into(),
            wallet_id: "wallet-team-abc".into(),
            created_at: "2026-02-26T00:00:00Z".into(),
            updated_at: "2026-02-26T00:00:00Z".into(),
        };

        store.save_team(&team).await;

        let fetched = store.get_team("team-abc").await;
        assert!(fetched.is_some());
        let t = fetched.unwrap();
        assert_eq!(t.name, "Alpha Squad");
        assert_eq!(t.wallet_id, "wallet-team-abc");

        let members: Vec<String> = serde_json::from_str(&t.members_json).unwrap();
        assert_eq!(members, vec!["zeus-112", "zeus-100"]);
    }

    #[tokio::test]
    async fn test_agent_team_list() {
        use crate::handlers::marketplace_store::{AgentTeamRow, MarketplaceStore};
        let store = MarketplaceStore::in_memory().unwrap();

        for i in 0..3 {
            let team = AgentTeamRow {
                id: format!("team-{}", i),
                name: format!("Team {}", i),
                members_json: r#"["a"]"#.into(),
                split_pct_json: r#"[100]"#.into(),
                wallet_id: format!("wallet-{}", i),
                created_at: "2026-02-26T00:00:00Z".into(),
                updated_at: "2026-02-26T00:00:00Z".into(),
            };
            store.save_team(&team).await;
        }

        let teams = store.list_teams().await;
        assert_eq!(teams.len(), 3);
    }

    #[tokio::test]
    async fn test_agent_team_not_found() {
        use crate::handlers::marketplace_store::MarketplaceStore;
        let store = MarketplaceStore::in_memory().unwrap();
        assert!(store.get_team("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_agent_team_update() {
        use crate::handlers::marketplace_store::{AgentTeamRow, MarketplaceStore};
        let store = MarketplaceStore::in_memory().unwrap();

        let team = AgentTeamRow {
            id: "team-1".into(),
            name: "Original".into(),
            members_json: r#"["a"]"#.into(),
            split_pct_json: r#"[100]"#.into(),
            wallet_id: "w-1".into(),
            created_at: "2026-02-26T00:00:00Z".into(),
            updated_at: "2026-02-26T00:00:00Z".into(),
        };
        store.save_team(&team).await;

        // Update name
        let updated = AgentTeamRow {
            name: "Renamed".into(),
            ..team
        };
        store.save_team(&updated).await;

        let t = store.get_team("team-1").await.unwrap();
        assert_eq!(t.name, "Renamed");
    }

    #[test]
    fn test_federation_url_validation() {
        // Helper matching the handler logic — uses fleet_subnet() for configurable subnet
        fn is_valid_federation_url(url: &str) -> bool {
            let subnet = super::fleet_subnet();
            let local_prefix = format!("http://{}", subnet);
            let is_local = url.starts_with(&local_prefix);
            let is_zeus = url.starts_with("https://") && {
                let host = url
                    .trim_start_matches("https://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("");
                host.ends_with(".zeuslab.ai") || host == "zeuslab.ai"
            };
            is_local || is_zeus
        }
        // Valid fleet URLs (default subnet 192.168.1.)
        assert!(is_valid_federation_url("http://192.168.1.100:3001"));
        assert!(is_valid_federation_url("https://gt.zeuslab.ai"));
        assert!(is_valid_federation_url("https://zeuslab.ai/v1/fleet"));
        // Invalid — blocks arbitrary HTTPS
        assert!(!is_valid_federation_url("https://evil.com"));
        assert!(!is_valid_federation_url("http://evil.com"));
        assert!(!is_valid_federation_url("ftp://192.168.1.100"));
    }
}

// ============================================================================
// Phase 5 — Autonomous Earning + Agent Teams + Federation
// ============================================================================

/// Credit formula: base + (tools_used × tool_bonus) + complexity_bonus
///
/// Values are configurable via `[economy]` in config.toml.
/// Defaults: base=10, tool_bonus=2, moderate=10, complex=25.
fn calculate_earning(
    tools_used: usize,
    complexity: &str,
    config: &zeus_core::EconomyConfig,
) -> u64 {
    let base = config.earning_base;
    let tool_bonus = (tools_used as u64).saturating_mul(config.earning_tool_bonus);
    let complexity_bonus: u64 = match complexity {
        "complex" => config.earning_complex_bonus,
        "moderate" => config.earning_moderate_bonus,
        _ => 0,
    };
    base.saturating_add(tool_bonus)
        .saturating_add(complexity_bonus)
}

// ---------------------------------------------------------------------------
// POST /v1/economy/earn — Record autonomous earning event
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EarnRequest {
    /// Agent that completed the task
    pub agent_id: String,
    /// Number of tools used during task
    #[serde(default)]
    pub tools_used: usize,
    /// Task complexity: "simple", "moderate", "complex"
    #[serde(default = "default_complexity")]
    pub complexity: String,
    /// Optional reference (session ID, mission ID)
    #[serde(default)]
    pub reference: Option<String>,
    /// Optional human-readable note
    #[serde(default)]
    pub note: Option<String>,
}

fn default_complexity() -> String {
    "simple".into()
}

/// POST /v1/economy/earn — Record an autonomous earning event.
///
/// Credits are computed server-side from tool count + complexity.
/// No client influence on amounts (Zeus107 security requirement).
pub async fn economy_earn(
    State(state): State<SharedState>,
    Json(req): Json<EarnRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let econ_config = state_guard.config.economy.clone().unwrap_or_default();
    let credits = calculate_earning(req.tools_used, &req.complexity, &econ_config);
    let note = req.note.as_deref().unwrap_or("Autonomous task completion");

    state_guard
        .ledger
        .earn(
            &req.agent_id,
            credits,
            zeus_economy::TransactionReason::TaskCompletion,
            format!(
                "{} (tools:{}, complexity:{}, credits:{})",
                note, req.tools_used, req.complexity, credits
            ),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Earn failed: {}", e),
            )
        })?;

    let balance = state_guard.ledger.balance(&req.agent_id).unwrap_or(0);

    info!(
        agent = %req.agent_id,
        credits = credits,
        tools = req.tools_used,
        complexity = %req.complexity,
        balance = balance,
        "Economy: autonomous earning"
    );

    Ok(Json(json!({
        "agent_id": req.agent_id,
        "credits_earned": credits,
        "balance": balance,
        "formula": {
            "base": 10,
            "tool_bonus": req.tools_used * 2,
            "complexity_bonus": match req.complexity.as_str() {
                "complex" => 25,
                "moderate" => 10,
                _ => 0,
            },
        },
        "reference": req.reference,
    })))
}

// ---------------------------------------------------------------------------
// GET /v1/economy/earnings/:agent_id — Earning history + aggregates
// ---------------------------------------------------------------------------

/// GET /v1/economy/earnings/:agent_id — earning history with aggregates
pub async fn economy_earnings(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);

    let state_guard = state.read().await;
    let balance = state_guard.ledger.balance(&agent_id).unwrap_or(0);
    let transactions = state_guard
        .ledger
        .transactions_for(&agent_id, limit)
        .unwrap_or_default();

    // Filter to earning transactions only
    let earnings: Vec<&zeus_economy::Transaction> = transactions
        .iter()
        .filter(|tx| {
            matches!(
                tx.kind,
                zeus_economy::TransactionKind::Earn | zeus_economy::TransactionKind::Mint
            ) && tx.to_agent.as_deref() == Some(&agent_id)
        })
        .collect();

    let total_earned: u64 = earnings.iter().map(|tx| tx.amount).sum();
    let total_spent: u64 = transactions
        .iter()
        .filter(|tx| {
            matches!(tx.kind, zeus_economy::TransactionKind::Spend)
                && tx.from_agent.as_deref() == Some(&agent_id)
        })
        .map(|tx| tx.amount)
        .sum();

    Ok(Json(json!({
        "agent_id": agent_id,
        "balance": balance,
        "total_earned": total_earned,
        "total_spent": total_spent,
        "net": total_earned as i64 - total_spent as i64,
        "earning_count": earnings.len(),
        "recent_earnings": earnings.iter().take(20).map(|tx| json!({
            "id": tx.id,
            "amount": tx.amount,
            "reason": format!("{:?}", tx.reason),
            "note": tx.note,
            "timestamp": tx.created_at,
        })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------------------
// Agent Teams — shared wallets + revenue splitting
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FormTeamRequest {
    /// Team name
    pub name: String,
    /// Member agent IDs
    pub members: Vec<String>,
    /// Revenue split percentages (must sum to 100, same order as members)
    pub split_pct: Vec<u32>,
}

/// POST /v1/teams/form — Create a persistent agent team with shared wallet
pub async fn team_form(
    State(state): State<SharedState>,
    Json(req): Json<FormTeamRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    // Validation (Zeus107: split config must sum to 100, no negative)
    if req.members.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Team must have at least 1 member".into(),
        ));
    }
    if req.members.len() != req.split_pct.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            "members and split_pct must have same length".into(),
        ));
    }
    let sum: u32 = req.split_pct.iter().sum();
    if sum != 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("split_pct must sum to 100 (got {})", sum),
        ));
    }
    if req.name.is_empty() || req.name.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Team name must be 1-100 chars".into(),
        ));
    }

    let team_id = format!("team-{}", uuid::Uuid::new_v4());
    let wallet_id = format!("wallet-{}", team_id);

    let state_guard = state.read().await;

    // Initialize the team's shared wallet
    let _ = state_guard.ledger.balance(&wallet_id); // ensures wallet exists

    // Persist team in the marketplace store
    let team_row = crate::handlers::marketplace_store::AgentTeamRow {
        id: team_id.clone(),
        name: req.name.clone(),
        members_json: serde_json::to_string(&req.members).unwrap_or_default(),
        split_pct_json: serde_json::to_string(&req.split_pct).unwrap_or_default(),
        wallet_id: wallet_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    state_guard.marketplace_store.save_team(&team_row).await;

    info!(
        team = %team_id,
        members = ?req.members,
        "Economy: team formed"
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": team_id,
            "name": req.name,
            "members": req.members,
            "split_pct": req.split_pct,
            "wallet_id": wallet_id,
            "balance": 0,
        })),
    ))
}

/// GET /v1/teams/:id/wallet — Get team shared wallet balance
pub async fn team_wallet(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    let team = state_guard
        .marketplace_store
        .get_team(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Team {} not found", id)))?;

    let balance = state_guard.ledger.balance(&team.wallet_id).unwrap_or(0);
    let transactions = state_guard
        .ledger
        .transactions_for(&team.wallet_id, 20)
        .unwrap_or_default();

    Ok(Json(json!({
        "team_id": id,
        "wallet_id": team.wallet_id,
        "balance": balance,
        "recent_transactions": transactions.iter().take(10).map(|tx| json!({
            "id": tx.id,
            "kind": format!("{:?}", tx.kind),
            "amount": tx.amount,
            "note": tx.note,
            "timestamp": tx.created_at,
        })).collect::<Vec<_>>(),
    })))
}

/// POST /v1/teams/:id/split — Split team wallet revenue to members
pub async fn team_split(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let amount = body
        .get("amount")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'amount'".into()))?;

    if amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".into()));
    }

    let state_guard = state.read().await;

    let team = state_guard
        .marketplace_store
        .get_team(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Team {} not found", id)))?;

    let wallet_id = &team.wallet_id;
    let members: Vec<String> = serde_json::from_str(&team.members_json).unwrap_or_default();
    let split_pct: Vec<u32> = serde_json::from_str(&team.split_pct_json).unwrap_or_default();

    // Check team wallet has enough
    let balance = state_guard.ledger.balance(wallet_id).unwrap_or(0);
    if balance < amount {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            format!("Team wallet has {} but split requires {}", balance, amount),
        ));
    }

    // Spend from team wallet
    state_guard
        .ledger
        .spend(
            wallet_id,
            amount,
            zeus_economy::TransactionReason::Custom("team_split".into()),
            format!(
                "Revenue split: {} tokens to {} members",
                amount,
                members.len()
            ),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Spend failed: {}", e),
            )
        })?;

    // Distribute to each member (remainder goes to last member)
    let mut payouts = Vec::new();
    let mut distributed: u64 = 0;
    for (i, member) in members.iter().enumerate() {
        let pct = split_pct.get(i).copied().unwrap_or(0);
        let share = if i == members.len() - 1 {
            // Last member gets remainder to prevent rounding loss
            amount.saturating_sub(distributed)
        } else {
            (amount * pct as u64) / 100
        };
        distributed += share;
        if share > 0 {
            let _ = state_guard.ledger.earn(
                member,
                share,
                zeus_economy::TransactionReason::Custom("team_revenue".into()),
                format!("Team {} revenue split ({}%)", id, pct),
            );
            payouts.push(json!({
                "agent_id": member,
                "pct": pct,
                "amount": share,
            }));
        }
    }

    info!(team = %id, amount = amount, "Economy: team revenue split");

    Ok(Json(json!({
        "team_id": id,
        "amount_split": amount,
        "payouts": payouts,
        "remaining_balance": state_guard.ledger.balance(wallet_id).unwrap_or(0),
    })))
}

/// GET /v1/teams/:id/earnings — Team earning history
pub async fn team_earnings(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    let team = state_guard
        .marketplace_store
        .get_team(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Team {} not found", id)))?;

    let wallet_id = &team.wallet_id;
    let transactions = state_guard
        .ledger
        .transactions_for(wallet_id, 50)
        .unwrap_or_default();

    let total_earned: u64 = transactions
        .iter()
        .filter(|tx| matches!(tx.kind, zeus_economy::TransactionKind::Earn))
        .map(|tx| tx.amount)
        .sum();

    let total_split: u64 = transactions
        .iter()
        .filter(|tx| {
            matches!(tx.kind, zeus_economy::TransactionKind::Spend) && tx.note.contains("split")
        })
        .map(|tx| tx.amount)
        .sum();

    Ok(Json(json!({
        "team_id": id,
        "team_name": team.name,
        "wallet_id": wallet_id,
        "total_earned": total_earned,
        "total_split": total_split,
        "balance": state_guard.ledger.balance(wallet_id).unwrap_or(0),
        "transactions": transactions.iter().take(30).map(|tx| json!({
            "id": tx.id,
            "kind": format!("{:?}", tx.kind),
            "amount": tx.amount,
            "reason": format!("{:?}", tx.reason),
            "note": tx.note,
            "timestamp": tx.created_at,
        })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------------------
// Cross-Fleet Federation — remote skill invocation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FederationInvokeRequest {
    /// Caller agent ID
    pub caller_id: String,
    /// Target remote agent URL (e.g. "http://192.168.1.100:3001")
    pub target_url: String,
    /// Skill name to invoke
    pub skill_name: String,
    /// Input parameters
    #[serde(default)]
    pub input: Value,
    /// Max credits to pay
    #[serde(default)]
    pub max_credits: u64,
    /// Timeout in seconds (default 30)
    #[serde(default = "default_federation_timeout")]
    pub timeout_secs: u64,
}

fn default_federation_timeout() -> u64 {
    30
}

/// POST /v1/federation/invoke — Invoke a skill on a remote fleet agent.
///
/// Authentication: caller must provide valid credentials.
/// Payment: credits are held in escrow, released on success.
pub async fn federation_invoke(
    State(state): State<SharedState>,
    Json(req): Json<FederationInvokeRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate target URL — allowlist: local fleet subnet + zeuslab.ai domain only
    let subnet = fleet_subnet();
    let local_prefix = format!("http://{}", subnet);
    let is_local_fleet = req.target_url.starts_with(&local_prefix);
    let is_zeus_https = req.target_url.starts_with("https://") && {
        let host = req
            .target_url
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
        host.ends_with(".zeuslab.ai") || host == "zeuslab.ai"
    };
    if !is_local_fleet && !is_zeus_https {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Federation invoke requires fleet URL ({}* or *.zeuslab.ai HTTPS)",
                subnet
            ),
        ));
    }

    let state_guard = state.read().await;

    // Hold credits in escrow
    if req.max_credits > 0 {
        state_guard
            .ledger
            .spend(
                &req.caller_id,
                req.max_credits,
                zeus_economy::TransactionReason::Custom("federation_escrow".into()),
                format!(
                    "Federation escrow: {} on {}",
                    req.skill_name, req.target_url
                ),
            )
            .map_err(|e| {
                (
                    StatusCode::PAYMENT_REQUIRED,
                    format!("Insufficient credits: {}", e),
                )
            })?;
    }

    let tx_id = uuid::Uuid::new_v4().to_string();
    let url = format!("{}/v1/fleet/execute", req.target_url.trim_end_matches('/'));

    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    let result = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(req.timeout_secs.min(120)))
        .json(&json!({
            "description": format!("Federation invoke: {}", req.skill_name),
            "tool_name": req.skill_name,
            "arguments": req.input,
            "federation_tx": tx_id,
            "caller": req.caller_id,
        }))
        .send()
        .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(resp) if resp.status().is_success() => {
            let body: Value = resp.json().await.unwrap_or_default();

            info!(
                tx = %tx_id,
                caller = %req.caller_id,
                target = %req.target_url,
                skill = %req.skill_name,
                elapsed = elapsed_ms,
                "Federation invoke: success"
            );

            Ok(Json(json!({
                "success": true,
                "transaction_id": tx_id,
                "skill_name": req.skill_name,
                "target_url": req.target_url,
                "output": body,
                "credits_charged": req.max_credits,
                "elapsed_ms": elapsed_ms,
            })))
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            // Refund escrow on failure
            if req.max_credits > 0 {
                let _ = state_guard.ledger.earn(
                    &req.caller_id,
                    req.max_credits,
                    zeus_economy::TransactionReason::Custom("federation_refund".into()),
                    format!("Federation refund: {} failed ({})", req.skill_name, status),
                );
            }

            Err((
                StatusCode::BAD_GATEWAY,
                format!("Remote agent returned {}: {} (tx:{})", status, body, tx_id),
            ))
        }
        Err(e) => {
            // Refund escrow on error
            if req.max_credits > 0 {
                let _ = state_guard.ledger.earn(
                    &req.caller_id,
                    req.max_credits,
                    zeus_economy::TransactionReason::Custom("federation_refund".into()),
                    format!("Federation refund: {} error ({})", req.skill_name, e),
                );
            }

            Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "Federation invoke to {} failed: {} (tx:{})",
                    req.target_url, e, tx_id
                ),
            ))
        }
    }
}

/// GET /v1/federation/discover — Discover remote fleet agents
pub async fn federation_discover(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();

    let all = gsm.list_agents().await;
    let agents: Vec<Value> = all
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "status": format!("{:?}", a.status),
                "capabilities": a.capabilities,
                "endpoint": a.metadata.get("endpoint").cloned().unwrap_or_default(),
                "ip": a.metadata.get("ip").cloned().unwrap_or_default(),
                "last_heartbeat": a.last_heartbeat.to_rfc3339(),
            })
        })
        .collect();

    Json(json!({
        "agents": agents,
        "total": agents.len(),
    }))
}

/// POST /v1/federation/settle — Settle a cross-fleet payment
pub async fn federation_settle(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tx_id = body
        .get("transaction_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'transaction_id'".into()))?;

    let from = body
        .get("from")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'from'".into()))?;

    let to = body
        .get("to")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'to'".into()))?;

    let amount = body
        .get("amount")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'amount'".into()))?;

    if amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".into()));
    }

    let state_guard = state.read().await;

    let (from_balance, to_balance) = state_guard
        .ledger
        .transfer(
            from,
            to,
            amount,
            zeus_economy::TransactionReason::Custom(format!("federation:{}", tx_id)),
            format!("Federation settlement (tx:{})", tx_id),
        )
        .map_err(|e| {
            (
                StatusCode::PAYMENT_REQUIRED,
                format!("Settlement failed: {}", e),
            )
        })?;

    info!(
        tx = %tx_id,
        from = %from,
        to = %to,
        amount = amount,
        "Federation: payment settled"
    );

    Ok(Json(json!({
        "transaction_id": tx_id,
        "from": from,
        "to": to,
        "amount": amount,
        "from_balance": from_balance,
        "to_balance": to_balance,
    })))
}

// ============================================================================
// GitHub Webhook — Fleet Auto-Sync
// ============================================================================

/// POST /v1/fleet/sync
///
/// Receives a GitHub push webhook. On a push to `refs/heads/main`:
/// 1. Verifies HMAC-SHA256 signature against `GITHUB_WEBHOOK_SECRET` env var.
/// 2. Spawns a background task that runs `scripts/fleet-sync.sh`.
/// 3. Returns 202 Accepted immediately (non-blocking).
///
/// If `GITHUB_WEBHOOK_SECRET` is unset, signature verification is skipped
/// (acceptable for private self-hosted deployments with network-level security).
pub async fn github_webhook_sync(
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::response::IntoResponse;

    // --- 1. Verify GitHub push event header ---
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event != "push" {
        // Not a push event — ACK silently (GitHub sends pings too)
        return Ok((StatusCode::OK, axum::Json(serde_json::json!({"status": "ignored", "reason": "not a push event"}))).into_response());
    }

    // --- 2. HMAC-SHA256 signature verification ---
    if let Ok(secret) = std::env::var("GITHUB_WEBHOOK_SECRET") {
        let sig = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_github_signature(&body, sig, &secret) {
            tracing::warn!("Fleet sync: webhook signature verification failed");
            return Err((StatusCode::UNAUTHORIZED, "Invalid webhook signature".to_string()));
        }
    }

    // --- 3. Parse payload, check branch ---
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {e}")))?;

    let git_ref = payload.get("ref").and_then(|v| v.as_str()).unwrap_or("");
    if git_ref != "refs/heads/main" {
        return Ok((StatusCode::OK, axum::Json(serde_json::json!({"status": "ignored", "reason": "not main branch"}))).into_response());
    }

    let commit_sha = payload
        .pointer("/after")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let repo = payload
        .pointer("/repository/full_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    tracing::info!(sha = %commit_sha, repo = %repo, "Fleet sync triggered via GitHub webhook");

    // --- 4. Spawn background sync (non-blocking) ---
    tokio::spawn(async move {
        run_fleet_sync(commit_sha, repo).await;
    });

    Ok((StatusCode::ACCEPTED, axum::Json(serde_json::json!({"status": "accepted", "message": "fleet sync queued"}))).into_response())
}

/// Verify HMAC-SHA256 signature from GitHub webhook.
fn verify_github_signature(body: &[u8], signature_header: &str, secret: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let Ok(mut mac) = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let expected_hex: String = expected.iter().map(|b| format!("{:02x}", b)).collect();

    let Some(provided) = signature_header.strip_prefix("sha256=") else {
        return false;
    };
    if provided.len() != expected_hex.len() {
        return false;
    }
    provided
        .as_bytes()
        .iter()
        .zip(expected_hex.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Run the fleet sync script and post Discord notification on completion.
async fn run_fleet_sync(commit_sha: String, repo: String) {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fleet-sync.sh");

    let result = tokio::process::Command::new("bash")
        .arg(&script)
        .env("ZEUS_SYNC_COMMIT", &commit_sha)
        .env("ZEUS_SYNC_REPO", &repo)
        .output()
        .await;

    match result {
        Ok(out) if out.status.success() => {
            tracing::info!(sha = %commit_sha, "Fleet sync completed successfully");
            notify_discord(
                &format!(
                    "**fleet-auto-sync** ✅ `{}` → `{}` deployed and restarted.",
                    repo,
                    &commit_sha[..8.min(commit_sha.len())]
                ),
            )
            .await;
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::error!(sha = %commit_sha, stderr = %stderr, "Fleet sync script failed");
            notify_discord(
                &format!(
                    "**fleet-auto-sync** ❌ sync failed for `{}` @ `{}`: {}",
                    repo,
                    &commit_sha[..8.min(commit_sha.len())],
                    stderr.chars().take(200).collect::<String>()
                ),
            )
            .await;
        }
        Err(e) => {
            tracing::error!(sha = %commit_sha, err = %e, "Failed to spawn fleet-sync.sh");
            notify_discord(&format!(
                "**fleet-auto-sync** ❌ failed to spawn sync script: {e}"
            ))
            .await;
        }
    }
}

/// Post a message to the Zeus fleet Discord channel.
async fn notify_discord(message: &str) {
    let token = match zeus_core::resolve_discord_token() {
        Some(t) => t,
        None => return, // No token configured — skip silently
    };
    let channel_id = std::env::var("ZEUS_DISCORD_FLEET_CHANNEL")
        .unwrap_or_else(|_| "1475583517156180018".to_string());

    let client = reqwest::Client::new();
    let _ = client
        .post(format!(
            "https://discord.com/api/v10/channels/{}/messages",
            channel_id
        ))
        .header("Authorization", format!("Bot {}", token))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"content": message}))
        .send()
        .await;
}
