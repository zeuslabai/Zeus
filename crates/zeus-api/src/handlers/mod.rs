//! API Handlers

pub mod gateway_handlers;
pub mod history_handlers;
pub use history_handlers::*;
pub mod tts_handlers;
pub use tts_handlers::*;

pub mod analytics;
pub use analytics::*;
pub mod agent_handlers;
pub use agent_handlers::*;
pub mod channel_handlers;
pub use channel_handlers::*;
pub mod chat_handlers;
pub mod council_handlers;
pub use chat_handlers::*;
pub mod config_handlers;
pub use config_handlers::*;
pub mod memory_handlers;
pub use memory_handlers::*;
pub mod sessions;
pub use sessions::*;
pub mod mcp_handlers;
pub use mcp_handlers::*;
pub mod onboarding_handlers;
pub use onboarding_handlers::*;
pub mod project_handlers;
pub use project_handlers::*;
pub mod network_handlers;
pub use network_handlers::*;
pub mod schedule_handlers;
pub use schedule_handlers::*;
pub mod security;
pub use security::*;
pub mod security_handlers;
pub use security_handlers::*;
pub mod agent_spawner;
pub mod blog;
pub mod blog_auth;
pub mod cron;
pub mod deploy_pipeline;
pub mod deploy_store;
pub mod discord_history;
pub mod slack_history;
pub mod fleet;
pub mod fleet_provisioner;
pub mod marketplace_dto;
pub mod marketplace_store;
pub mod observatory;
pub mod pantheon;
pub mod pantheon_store;
pub mod studio_handlers;
pub use studio_handlers::*;
pub mod economy_handlers;
pub use economy_handlers::*;
pub mod studio_store;
pub mod task_store;
pub mod totp_store;
mod webhooks;
pub use deploy_store::DeployStore;
pub use discord_history::DiscordHistoryStore;
pub use slack_history::SlackHistoryStore;
pub use task_store::TaskStore;
pub use marketplace_store::MarketplaceStore;
pub use pantheon::{
    PantheonEvent, Room, RoomMember, RoomType,
    // Reactions
    add_reaction,
    // Plan approval
    approve_mission,
    approve_plan,
    reject_mission,
    create_mission,
    // DM handlers
    find_or_create_dm,
    list_dms,
    // Room handlers
    create_room,
    // Chat ops
    delete_room_message,
    edit_room_message,
    get_identity,
    get_mission,
    get_mission_artifacts,
    download_mission_artifact,
    get_mission_feed,
    get_reactions,
    get_room,
    get_room_messages,
    intervene_mission,
    join_room,
    leave_room,
    list_identities,
    list_missions,
    list_pending_plans,
    list_room_members,
    list_rooms,
    mission_events,
    // Agora wiring
    pantheon_economy,
    reject_plan,
    remove_reaction,
    review_task,
    send_room_message,
    // File uploads
    upload_room_file,
    // Identity
    set_identity,
};
pub use pantheon_store::PantheonStore;
pub use studio_store::StudioStore;
pub use totp_store::TotpStore;
pub use vector_store::VectorStoreDb;
pub mod skills;
pub use skills::*;
pub mod sandbox_handlers;
pub use sandbox_handlers::*;
pub mod teams_handlers;
pub use teams_handlers::*;
pub mod extensions_handlers;
pub use extensions_handlers::*;
pub mod benchmark_handlers;
pub use benchmark_handlers::*;
pub mod prometheus_handlers;
pub use prometheus_handlers::*;
pub mod workflow_handlers;
pub use workflow_handlers::*;
pub mod review_handlers;
pub use review_handlers::*;
pub mod agora;
pub use agora::{
    agora_listings, agora_list_skill, agora_agent_listings, agora_delist_skill,
    agora_search, agora_wallet, agora_register_wallet, agora_buy, agora_transactions,
    agora_reputation,
};
pub mod batch;
pub mod canvas;
pub mod intelligence;
pub mod responses;
pub mod templates;
pub mod vector_store;
pub use webhooks::{
    create_trigger, delete_trigger, disable_trigger, enable_trigger, list_triggers,
    receive_recording_status, receive_voice_inbound, receive_webhook, receive_webhook_source,
    receive_whatsapp_webhook, voice_inbound_health, webhook_health, whatsapp_webhook_health,
};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, info};
use zeus_llm::LlmClient;
use zeus_session::Session;

use crate::SharedState;

use std::path::PathBuf;

/// Pagination query parameters for list endpoints.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// ============================================================================
// Economy helpers
// ============================================================================

/// Map a model string to a token cost tier.
pub fn model_tier_cost(model: &str) -> u64 {
    let model_lower = model.to_lowercase();
    if model_lower.contains("opus") {
        50
    } else if model_lower.contains("sonnet")
        || model_lower.contains("gpt-4o")
        || model_lower.contains("gemini-1.5-pro")
        || model_lower.contains("llama-3.1-405b")
    {
        25
    } else if model_lower.contains("haiku")
        || model_lower.contains("gpt-4o-mini")
        || model_lower.contains("gemini-flash")
        || model_lower.contains("llama-3")
    {
        5
    } else {
        1
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================


/// Format an error as OpenAI-compatible JSON string
pub fn openai_error(message: &str) -> String {
    json!({
        "error": {
            "message": message,
            "type": "server_error",
            "code": null,
        }
    })
    .to_string()
}

pub mod tools_handlers;
pub use tools_handlers::*;
pub mod credentials_handlers;
pub use credentials_handlers::*;
pub mod polls_handlers;
pub use polls_handlers::*;
pub mod webhooks_handlers;
pub use webhooks_handlers::*;
pub mod personas_handlers;
pub use personas_handlers::*;
pub mod stats_handlers;
pub mod auth_handlers;
pub use auth_handlers::*;
pub mod device_code_handlers;
pub use device_code_handlers::*;
pub mod task_handlers;
pub use task_handlers::*;
pub mod marketplace_handlers;
pub use marketplace_handlers::*;
pub mod deploy_handlers;
pub use deploy_handlers::*;
pub mod orchestrate_handlers;
pub use orchestrate_handlers::*;
pub mod goals_handlers;
pub use goals_handlers::*;
pub mod bounty_handlers;
pub use bounty_handlers::*;
pub use stats_handlers::*;

// ============================================================================
// Token Counting
// ============================================================================

// Memory handlers moved to memory_handlers.rs
// Config handlers moved to config_handlers.rs

// ============================================================================
// Activity Feed
// ============================================================================


// ============================================================================
// Stats
// ============================================================================


// ============================================================================
// Doctor / Diagnostics
// ============================================================================


/// Simple check if Ollama is reachable (TCP connect)

// ============================================================================
// Credential Vault Endpoints
// ============================================================================

// ============================================================================
// Phase 4 Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub autonomy: Option<String>,
    #[serde(default)]
    pub autonomy_level: Option<String>,
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub heartbeat: Option<Value>,
    #[serde(default)]
    pub bindings: Option<Vec<zeus_core::BindingRule>>,
    #[serde(default)]
    pub tool_policy: Option<zeus_core::AgentToolPolicy>,
    #[serde(default)]
    pub priority: Option<i32>,
    /// Auto-create a supervisor agent (default: true). Set to false for standalone agents.
    #[serde(default = "default_auto_supervisor")]
    pub auto_supervisor: bool,
}

fn default_auto_supervisor() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub status: Option<String>,
    pub autonomy: Option<String>,
    pub autonomy_level: Option<String>,
    pub model: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub persona: Option<String>,
    pub soul: Option<String>,
    pub tools: Option<Vec<String>>,
    pub heartbeat: Option<Value>,
    pub bindings: Option<Vec<zeus_core::BindingRule>>,
    pub tool_policy: Option<zeus_core::AgentToolPolicy>,
    pub priority: Option<i32>,
}

// ============================================================================
// Phase 3: Pipeline Stats
// ============================================================================

/// GET /v1/pipeline/stats — Pipeline stage metrics

// ============================================================================
// Phase 4: Agent CRUD Endpoints
// ============================================================================

pub fn agents_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.parent().unwrap_or(workspace).join("agents")
}

/// Scan the agents directory for an existing agent with the given name.
/// Returns (id, path) if found, so the caller can reuse the ID for upsert.
pub async fn find_agent_by_name(
    dir: &std::path::Path,
    name: &str,
) -> Option<(String, std::path::PathBuf)> {
    let mut rd = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && let Ok(content) = tokio::fs::read_to_string(&path).await
            && let Ok(agent) = serde_json::from_str::<Value>(&content)
            && agent["name"].as_str() == Some(name)
            && let Some(id) = agent["id"].as_str()
        {
            return Some((id.to_string(), path));
        }
    }
    None
}

/// GET /v1/agents — List agents

/// POST /v1/agents — Create agent

/// GET /v1/agents/:id — Get agent

/// PUT /v1/agents/:id — Update agent

/// DELETE /v1/agents/:id — Delete agent

// ============================================================================
// Agent Persona Templates
// ============================================================================

/// POST /v1/agents/from-persona/:name — Create an agent from a persona template

// ============================================================================
// Agent Routing: Spawn, Send, Status
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SpawnAgentRequest {
    pub agent_id: String,
}

/// POST /v1/agents/run-task — Run a task via a local subagent.
///
/// Accepts the same payload that `spawn_remote()` sends, creating and running
/// a local subagent with the gateway's configured LLM (or an explicit model
/// override). When `wait` is true (default) the response is returned
/// synchronously; otherwise a background task ID is returned.
#[derive(Debug, Deserialize)]
pub struct RunTaskRequest {
    /// Natural-language task description.
    pub task: String,
    /// Additional context for the subagent.
    #[serde(default)]
    pub context: String,
    /// Maximum LLM iterations (default 15).
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Block until the subagent finishes (default true).
    #[serde(default = "default_wait")]
    pub wait: bool,
    /// Optional model override (e.g. "anthropic/claude-sonnet-4-20250514").
    #[serde(default)]
    pub model: Option<String>,
}

fn default_max_iterations() -> usize {
    15
}
fn default_wait() -> bool {
    true
}

pub async fn run_task(
    State(state): State<SharedState>,
    Json(req): Json<RunTaskRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.task.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "task is required".to_string()));
    }

    let (llm, workspace) = {
        let sg = state.read().await;
        let llm = if let Some(ref model_str) = req.model {
            // Temporarily override the config model for parsing
            let mut cfg = sg.config.clone();
            cfg.model = model_str.clone();
            LlmClient::from_config(&cfg).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("LLM init failed: {e}"),
                )
            })?
        } else {
            LlmClient::from_config(&sg.config).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("LLM init failed: {e}"),
                )
            })?
        };
        (llm, sg.workspace.clone())
    };

    let subagent_config = zeus_agent::SubagentConfig {
        max_iterations: req.max_iterations,
        can_spawn: false,
        task: req.task.clone(),
        context: req.context.clone(),
        target: zeus_agent::AgentTarget::Local,
        model: req.model.clone(),
        mission_id: None,
        parent_system_prompt: None,
        ..Default::default()
    };

    if req.wait {
        let subagent = zeus_agent::Subagent::new(subagent_config, llm, workspace, None);
        let result = subagent.run().await;
        Ok(Json(serde_json::json!({
            "success": result.success,
            "output": result.output,
            "iterations": result.iterations,
            "agent_id": result.id,
        })))
    } else {
        let handle = zeus_agent::spawn_subagent(subagent_config, llm, workspace, None);
        let task_id = uuid::Uuid::new_v4().to_string();
        // Detach — caller polls via other mechanisms
        drop(handle);
        Ok(Json(serde_json::json!({
            "status": "spawned",
            "task_id": task_id,
            "message": "Task running in background",
        })))
    }
}

#[derive(Debug, Deserialize)]
pub struct SendToAgentRequest {
    pub message: String,
}

/// POST /v1/agents/spawn — Spawn a managed agent into the registry
pub async fn spawn_agent(
    State(state): State<SharedState>,
    Json(req): Json<SpawnAgentRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut state = state.write().await;

    match state.agent_registry.spawn(&req.agent_id).await {
        Ok(()) => {
            let instance = state.agent_registry.get(&req.agent_id).ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Agent spawned but not found in registry".to_string(),
                )
            })?;
            // Snapshot fields before using the helper (releases instance borrow).
            let agent_id = instance.agent_id.clone();
            let name = instance.name.clone();
            let spawned_at = instance.spawned_at.to_rfc3339();
            let bindings_count = instance.binding.bindings.len();
            let priority = instance.binding.priority as f64;

            // Register with the compute provisioner so cooking-loop LLM and
            // tool quota checks can enforce budgets for this agent.
            state.register_agent_compute(&agent_id, priority).await;

            Ok(Json(json!({
                "agent_id": agent_id,
                "name": name,
                "spawned_at": spawned_at,
                "bindings_count": bindings_count,
                "compute_quota_registered": true
            })))
        }
        Err(e) if e.contains("already spawned") => Err((StatusCode::CONFLICT, e)),
        Err(e) if e.contains("not found") => Err((StatusCode::NOT_FOUND, e)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// POST /v1/agents/:id/send — Send a message to a spawned agent
pub async fn send_to_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<SendToAgentRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Get the agent Arc and update activity
    let agent_arc = {
        let mut state = state.write().await;
        state.agent_registry.update_activity(&id);
        match state.agent_registry.get(&id) {
            Some(instance) => instance.agent.clone(),
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("Agent '{}' is not spawned", id),
                ));
            }
        }
    };

    // Run the message outside of state lock
    let mut agent = agent_arc.write().await;
    match agent.run(&req.message).await {
        Ok(response) => Ok(Json(json!({
            "agent_id": id,
            "response": response
        }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Agent error: {}", e),
        )),
    }
}

/// GET /v1/agents/:id/status — Runtime status of a spawned agent

// ============================================================================
// Agent-as-API: Chat with a specific agent
// ============================================================================

// ============================================================================
// Outbound Webhooks Management
// ============================================================================


// ============================================================================
// Auth endpoints
// ============================================================================





// ============================================================================
// Anthropic OAuth (REST-driven authorization code + PKCE flow)
// ============================================================================



#[derive(Debug, Deserialize)]
pub struct OAuthCallbackParams {
    pub code: String,
    pub state: String,
}




// ============================================================================
// Agent Tasks (S52-T1 — checkpoint/resume)
// ============================================================================








// ============================================================================
// Discord History (S52-T2)
// ============================================================================

/// GET /v1/discord/history — query cached Discord messages
// Session Branching Endpoints — see branch_handlers.rs
pub mod branch_handlers;
pub use branch_handlers::*;
pub mod image_handlers;
pub use image_handlers::*;


// ============================================================================
// Prometheus Endpoints (Strategic Planning + Coordination)
// ============================================================================






// ── Marketplace sync ─────────────────────────────────────────────────────────



// ── SQLite-backed marketplace endpoints ──────────────────────────────────────







// ============================================================================
// Bounty Board (Agora social wiring)
// ============================================================================








// ============================================================================
// Reputation Badges (Agora social wiring)
// ============================================================================


// ============================================================================
// One-Click Deploy (Phase 4)
// ============================================================================
















/// Capture a screenshot of a URL via Chrome CDP.
///
/// Creates a new tab, navigates, waits for load, screenshots, closes tab.

#[cfg(test)]
mod tests;










/// Attempt LLM-based goal analysis using provider details from the request or saved config.
pub(crate) async fn try_llm_goal_analysis(state: &SharedState, body: &Value, goal: &str) -> Option<Value> {
    let system_prompt = "You are a project planning assistant for Zeus AI. Analyze the user's goal and return ONLY valid JSON (no markdown, no code fences). Tailor every field specifically to the goal described — never return generic steps.";

    let user_prompt = format!(
        r#"Analyze this project goal: "{}"

Return ONLY a JSON object with these exact fields:
{{
  "recommended_workflow": "a short descriptive name like web_app, api_backend, mobile_app, data_pipeline, devops_setup, ml_model, etc.",
  "steps": ["4-7 specific actionable steps tailored to this exact goal"],
  "agents": ["roles needed, e.g. developer, designer, devops, tester, researcher"],
  "complexity": "low or medium or high",
  "autonomy_level": "autonomous or guided or supervised",
  "estimated_cost": 0.20
}}

Be SPECIFIC. For example if the goal is 'create a website for a pet shop', steps should mention pet shop features, not generic 'implement solution'."#,
        goal
    );

    // Strategy 1: Use provider details from request body (onboarding case)
    let provider = body.get("provider").and_then(|v| v.as_str());
    let model = body.get("model").and_then(|v| v.as_str());
    let api_key = body.get("api_key").and_then(|v| v.as_str());
    let url = body.get("url").and_then(|v| v.as_str());

    if let (Some(prov), Some(mdl)) = (provider, model)
        && let Some(result) =
            call_provider_for_analysis(state, prov, mdl, api_key, url, system_prompt, &user_prompt)
                .await
    {
        return Some(result);
    }

    // Strategy 2: Use saved config (post-onboarding case)
    let state_read = state.read().await;
    let llm = LlmClient::from_config(&state_read.config).ok()?;
    let messages = vec![
        zeus_core::Message::system(system_prompt),
        zeus_core::Message::user(&user_prompt),
    ];
    let response = llm.complete(&messages, &[], None).await.ok()?;
    parse_llm_goal_json(&response.content)
}

/// Make a direct HTTP call to a provider for goal analysis (used during onboarding).
async fn call_provider_for_analysis(
    state: &SharedState,
    provider: &str,
    model: &str,
    api_key: Option<&str>,
    url: Option<&str>,
    system_prompt: &str,
    user_prompt: &str,
) -> Option<Value> {
    let client = { state.read().await.http_client.clone() };

    let (endpoint, auth_header) = match provider {
        "ollama" => {
            let base = url.unwrap_or("http://localhost:11434");
            (
                format!("{}/v1/chat/completions", base.trim_end_matches('/')),
                None,
            )
        }
        "openai" => (
            "https://api.openai.com/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "groq" => (
            "https://api.groq.com/openai/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "together" => (
            "https://api.together.xyz/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "fireworks" => (
            "https://api.fireworks.ai/inference/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "mistral" => (
            "https://api.mistral.ai/v1/chat/completions".to_string(),
            api_key.map(|k| format!("Bearer {}", k)),
        ),
        "anthropic" => {
            // Anthropic uses Messages API format
            let payload = json!({
                "model": model,
                "max_tokens": 1024,
                "system": system_prompt,
                "messages": [{"role": "user", "content": user_prompt}]
            });
            let key = api_key?;
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&payload)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .ok()?;
            let body: Value = resp.json().await.ok()?;
            let content = body
                .get("content")?
                .as_array()?
                .first()?
                .get("text")?
                .as_str()?;
            return parse_llm_goal_json(content);
        }
        "google" => {
            // Google Gemini uses a different format
            let key = api_key?;
            let endpoint = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                model, key
            );
            let payload = json!({
                "contents": [{"parts": [{"text": format!("{}\n\n{}", system_prompt, user_prompt)}]}]
            });
            let resp = client
                .post(&endpoint)
                .header("content-type", "application/json")
                .json(&payload)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .ok()?;
            let body: Value = resp.json().await.ok()?;
            let content = body
                .get("candidates")?
                .as_array()?
                .first()?
                .get("content")?
                .get("parts")?
                .as_array()?
                .first()?
                .get("text")?
                .as_str()?;
            return parse_llm_goal_json(content);
        }
        _ => return None,
    };

    // OpenAI-compatible call (covers most providers)
    let payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "temperature": 0.7,
        "max_tokens": 1024
    });

    let mut req = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30));

    if let Some(auth) = auth_header {
        req = req.header("authorization", auth);
    }

    let resp = req.send().await.ok()?;
    let body: Value = resp.json().await.ok()?;
    let content = body
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()?;
    parse_llm_goal_json(content)
}

/// Parse LLM response text into a goal analysis JSON Value.
fn parse_llm_goal_json(content: &str) -> Option<Value> {
    // Strip markdown code fences if present
    let cleaned = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    let parsed: Value = serde_json::from_str(cleaned).ok()?;

    // Validate required fields exist
    if parsed.get("steps").is_some() && parsed.get("recommended_workflow").is_some() {
        Some(parsed)
    } else {
        None
    }
}

/// Heuristic-based goal analysis fallback (keyword matching).
pub(crate) fn heuristic_goal_analysis(goal_text: &str) -> Value {
    let lower = goal_text.to_lowercase();
    let word_count = goal_text.split_whitespace().count();

    let complexity = if word_count > 50
        || lower.contains("refactor")
        || lower.contains("migrate")
        || lower.contains("architect")
    {
        "high"
    } else if word_count > 20
        || lower.contains("implement")
        || lower.contains("build")
        || lower.contains("create")
        || lower.contains("design")
    {
        "medium"
    } else {
        "low"
    };

    let recommended_workflow = if lower.contains("deploy")
        || lower.contains("release")
        || lower.contains("ship")
    {
        "deployment"
    } else if lower.contains("fix")
        || lower.contains("bug")
        || lower.contains("debug")
        || lower.contains("error")
    {
        "bugfix"
    } else if lower.contains("test") || lower.contains("verify") || lower.contains("validate") {
        "testing"
    } else if lower.contains("research")
        || lower.contains("investigate")
        || lower.contains("analyze")
        || lower.contains("explore")
    {
        "research"
    } else if lower.contains("refactor") || lower.contains("clean") || lower.contains("optimize") {
        "refactoring"
    } else if lower.contains("write") || lower.contains("document") || lower.contains("readme") {
        "documentation"
    } else {
        "development"
    };

    let mut agents: Vec<&str> = Vec::new();
    if lower.contains("code")
        || lower.contains("implement")
        || lower.contains("build")
        || lower.contains("develop")
    {
        agents.push("developer");
    }
    if lower.contains("test") || lower.contains("verify") || lower.contains("qa") {
        agents.push("tester");
    }
    if lower.contains("review") || lower.contains("audit") || lower.contains("security") {
        agents.push("reviewer");
    }
    if lower.contains("deploy")
        || lower.contains("infra")
        || lower.contains("server")
        || lower.contains("ci")
    {
        agents.push("devops");
    }
    if lower.contains("research") || lower.contains("investigate") || lower.contains("analyze") {
        agents.push("researcher");
    }
    if lower.contains("design")
        || lower.contains("ui")
        || lower.contains("ux")
        || lower.contains("frontend")
    {
        agents.push("designer");
    }
    if agents.is_empty() {
        agents.push("developer");
    }

    let autonomy_level = if lower.contains("deploy")
        || lower.contains("delete")
        || lower.contains("production")
        || lower.contains("release")
    {
        "supervised"
    } else if complexity == "high" {
        "guided"
    } else {
        "autonomous"
    };

    let estimated_cost = match complexity {
        "high" => 0.50,
        "medium" => 0.20,
        _ => 0.05,
    };

    // Generate steps based on workflow type AND goal keywords for better specificity
    let steps: Vec<&str> = match recommended_workflow {
        "deployment" => vec![
            "Review changes",
            "Run test suite",
            "Build release",
            "Deploy to staging",
            "Verify staging",
            "Deploy to production",
        ],
        "bugfix" => vec![
            "Reproduce the issue",
            "Identify root cause",
            "Implement fix",
            "Write regression test",
            "Verify fix",
        ],
        "testing" => vec![
            "Identify test scope",
            "Write test cases",
            "Run tests",
            "Review coverage",
            "Report results",
        ],
        "research" => vec![
            "Define research scope",
            "Gather information",
            "Analyze findings",
            "Document conclusions",
            "Present recommendations",
        ],
        "refactoring" => vec![
            "Analyze current code",
            "Identify improvement areas",
            "Plan refactoring steps",
            "Implement changes",
            "Run tests",
            "Review results",
        ],
        "documentation" => vec![
            "Identify documentation needs",
            "Outline structure",
            "Write content",
            "Review and revise",
            "Publish",
        ],
        _ if lower.contains("website")
            || lower.contains("web app")
            || lower.contains("landing page") =>
        {
            agents = vec!["developer", "designer"];
            vec![
                "Design page layout and navigation",
                "Set up project structure and tooling",
                "Build responsive UI components",
                "Implement content and media sections",
                "Add interactivity and forms",
                "Test across browsers and devices",
                "Deploy to hosting",
            ]
        }
        _ if lower.contains("api") || lower.contains("backend") || lower.contains("server") => {
            vec![
                "Define API endpoints and data models",
                "Set up project and dependencies",
                "Implement core business logic",
                "Add authentication and validation",
                "Write integration tests",
                "Document API endpoints",
                "Deploy and monitor",
            ]
        }
        _ if lower.contains("mobile")
            || lower.contains("ios")
            || lower.contains("android")
            || lower.contains("app") =>
        {
            agents = vec!["developer", "designer"];
            vec![
                "Design app screens and navigation flow",
                "Set up project and dependencies",
                "Build core UI screens",
                "Implement data layer and networking",
                "Add user authentication",
                "Test on target devices",
                "Prepare for store submission",
            ]
        }
        _ if lower.contains("bot") || lower.contains("automat") || lower.contains("script") => {
            vec![
                "Define automation triggers and actions",
                "Set up runtime environment",
                "Implement core logic",
                "Add error handling and retries",
                "Test with sample data",
                "Deploy and schedule",
            ]
        }
        _ if lower.contains("dashboard")
            || lower.contains("analytics")
            || lower.contains("report") =>
        {
            agents = vec!["developer", "designer"];
            vec![
                "Identify key metrics and data sources",
                "Design dashboard layout",
                "Set up data pipeline",
                "Build visualization components",
                "Add filtering and interactivity",
                "Test with real data",
            ]
        }
        _ => vec![
            "Analyze requirements",
            "Plan implementation",
            "Implement solution",
            "Test changes",
            "Review and finalize",
        ],
    };

    json!({
        "recommended_workflow": recommended_workflow,
        "agents": agents,
        "autonomy_level": autonomy_level,
        "estimated_cost": estimated_cost,
        "steps": steps,
        "complexity": complexity,
        "analysis_method": "heuristic",
    })
}

/// GET /v1/workflows/:id/artifacts — List artifacts for an orchestration session.
pub async fn workflow_artifacts(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .orchestration()
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    drop(state_guard);

    let artifacts: Vec<Value> = session
        .artifacts
        .iter()
        .filter_map(|a| serde_json::to_value(a).ok())
        .collect();

    Ok(Json(json!({
        "session_id": id,
        "artifacts": artifacts,
        "total": session.artifacts.len(),
    })))
}

/// GET /v1/workflows/:id/download — Download deliverable ZIP for an orchestration.
pub async fn workflow_download(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response<axum::body::Body>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .orchestration()
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    drop(state_guard);

    if let zeus_prometheus::orchestrate::OrchestrationPhase::Delivered {
        ref artifact_path, ..
    } = session.phase
        && !artifact_path.is_empty()
    {
        let path = std::path::Path::new(artifact_path);
        if path.exists() {
            let bytes = tokio::fs::read(path).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read artifact: {e}"),
                )
            })?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("deliverable.zip");
            return Ok(axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/zip")
                .header(
                    "Content-Disposition",
                    format!("attachment; filename=\"{filename}\""),
                )
                .body(axum::body::Body::from(bytes))
                .unwrap());
        }
    }

    Ok(axum::response::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            json!({
                "error": "No deliverable available yet",
                "session_id": id,
                "phase": session.phase.label(),
            })
            .to_string(),
        ))
        .unwrap())
}

// ============================================================================
// Orchestration Helpers
// ============================================================================

/// Parse LLM response into a GoalAnalysis, with fallback for malformed JSON.
pub(crate) fn parse_goal_analysis(content: &str) -> zeus_prometheus::orchestrate::GoalAnalysis {
    use zeus_prometheus::orchestrate::{GoalAnalysis, OnboardingQuestion};

    // Strip markdown code blocks if present
    let json_str = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim())
        .trim()
        .strip_suffix("```")
        .unwrap_or(content.trim())
        .trim();

    if let Ok(parsed) = serde_json::from_str::<Value>(json_str) {
        let questions: Vec<OnboardingQuestion> = parsed
            .get("clarification_questions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| {
                        Some(OnboardingQuestion {
                            question: q.get("question")?.as_str()?.to_string(),
                            answer: None,
                            purpose: q
                                .get("purpose")
                                .and_then(|v| v.as_str())
                                .unwrap_or("clarification")
                                .to_string(),
                        })
                    })
                    .take(5)
                    .collect()
            })
            .unwrap_or_default();

        let needs_clarification = parsed
            .get("needs_clarification")
            .and_then(|v| v.as_bool())
            .unwrap_or(!questions.is_empty());

        GoalAnalysis {
            summary: parsed
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Project analysis")
                .to_string(),
            scope: parsed
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_string(),
            complexity: parsed
                .get("complexity")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string(),
            suggested_approach: parsed
                .get("suggested_approach")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            needs_clarification,
            clarification_questions: questions,
        }
    } else {
        // Fallback: treat response as summary, skip onboarding
        GoalAnalysis {
            summary: content.chars().take(200).collect(),
            scope: "general".to_string(),
            complexity: "medium".to_string(),
            suggested_approach: String::new(),
            needs_clarification: false,
            clarification_questions: vec![],
        }
    }
}

/// Generate a team recommendation based on goal analysis.
pub(crate) fn generate_team_recommendation(
    analysis: &zeus_prometheus::orchestrate::GoalAnalysis,
    _goal: &str,
) -> zeus_prometheus::orchestrate::TeamRecommendation {
    use zeus_prometheus::orchestrate::{AgentSuggestion, TeamRecommendation};

    let (coordinators, workers, estimated_steps) = match analysis.complexity.as_str() {
        "low" => (
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec!["planning".to_string(), "code-review".to_string()],
                model_tier: "sonnet".to_string(),
            }],
            vec![AgentSuggestion {
                role: "developer".to_string(),
                capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                model_tier: "sonnet".to_string(),
            }],
            3,
        ),
        "high" | "very_high" => (
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec![
                    "planning".to_string(),
                    "architecture".to_string(),
                    "code-review".to_string(),
                ],
                model_tier: "opus".to_string(),
            }],
            vec![
                AgentSuggestion {
                    role: "senior-developer".to_string(),
                    capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "developer".to_string(),
                    capabilities: vec!["implementation".to_string(), "testing".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "qa-engineer".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: "haiku".to_string(),
                },
            ],
            10,
        ),
        _ => (
            // medium complexity
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec!["planning".to_string(), "code-review".to_string()],
                model_tier: "opus".to_string(),
            }],
            vec![
                AgentSuggestion {
                    role: "developer".to_string(),
                    capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "tester".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: "haiku".to_string(),
                },
            ],
            6,
        ),
    };

    TeamRecommendation {
        team_name: format!("{}-team", analysis.scope),
        coordinators,
        workers,
        rationale: format!(
            "{} complexity {} project. {}",
            analysis.complexity, analysis.scope, analysis.suggested_approach
        ),
        estimated_complexity: analysis.complexity.clone(),
        estimated_steps,
    }
}

// ============================================================================
// WAVE 1 — Missing Endpoints (Audit Fix)
// ============================================================================

/// POST /v1/agents/team — create a team with agents assigned
///
/// Accepts the body shape that ZeusWeb teams.rs sends:
/// `{ "name", "description", "supervisor_id", "agents": [...], "routing_strategy" }`
pub async fn create_agent_team(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let mut team = zeus_orchestra::AgentTeam::new(name);

    if let Some(supervisor) = body.get("supervisor_id").and_then(|v| v.as_str())
        && !supervisor.is_empty()
    {
        team = team.with_supervisor(supervisor.to_string());
    }

    // Accept both "agents" (ZeusWeb) and "agent_ids" (existing API)
    let agent_ids = body
        .get("agents")
        .or_else(|| body.get("agent_ids"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if !agent_ids.is_empty() {
        team = team.with_agents(agent_ids);
    }

    let state_guard = state.read().await;
    let created = state_guard
        .orchestra()
        .create_team(team)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let mut val = serde_json::to_value(&created).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(
            "message".to_string(),
            serde_json::Value::String("Team created".to_string()),
        );
    }

    info!("Created agent team: {}", name);
    Ok((StatusCode::CREATED, Json(val)))
}

/// POST /v1/channels/:id/connect — connect (start) a channel adapter

/// POST /v1/channels/:id/disconnect — disconnect (stop) a channel adapter

pub async fn studio_chat(
    State(state): State<SharedState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let message = req
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if message.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing 'message' field".to_string(),
        ));
    }

    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let custom_system_prompt = req
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let state = state.read().await;

    // Create or load session
    let mut session = if let Some(id) = &session_id {
        Session::load(&state.config.sessions, id)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?
    } else {
        let s = Session::new(&state.config.sessions);
        s.init()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        s
    };

    let sid = session.id.clone();

    // Create LLM client
    let llm = LlmClient::from_config(&state.config)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Add user message
    let user_msg = zeus_core::Message::user(&message);
    session
        .add(user_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Build system prompt: use custom if provided, else workspace default
    let system_prompt = if let Some(custom) = custom_system_prompt {
        custom
    } else {
        state
            .workspace
            .get_context()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let tool_schemas = state.tools.schemas();

    // Call LLM
    let response = llm
        .complete(&session.messages, &tool_schemas, Some(&system_prompt))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Record LLM spend
    let cost = model_tier_cost(&state.config.model);
    if let Err(e) = state.ledger.spend(
        "default",
        cost,
        zeus_economy::TransactionReason::LlmCall,
        format!("studio: {}", &state.config.model),
    ) {
        debug!("Economy spend failed (non-fatal): {e}");
    }

    // Save assistant response
    let assistant_text = if response.content.is_empty() {
        "[no response]".to_string()
    } else {
        response.content.clone()
    };
    let assistant_msg = zeus_core::Message::assistant(&assistant_text);
    let _ = session.add(assistant_msg).await;

    Ok(Json(json!({
        "response": assistant_text,
        "session_id": sid,
    })))
}

// ---------------------------------------------------------------------------
// DM Pairing / Channel Auth handlers
// ---------------------------------------------------------------------------

/// POST /v1/channels/:id/pair — Generate a 6-digit pairing code

/// POST /v1/channels/:id/verify — Verify a pairing code

/// GET /v1/channels/:id/pairings — List verified pairings for a channel

// ═══════════════════════════════════════════════════
// S63: Office Message Stream (SSE)
// ═══════════════════════════════════════════════════

/// GET /v1/office/stream — SSE stream of all channel messages for Office visualization
pub async fn office_message_stream(
    State(state): State<SharedState>,
) -> axum::response::sse::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let state = state.read().await;
    let mut rx = state.office_broadcast.subscribe();
    drop(state);

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        yield Ok(axum::response::sse::Event::default().data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("Office stream lagged {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    axum::response::sse::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
}


// ═══════════════════════════════════════════════════
// S86: Office State (Star Office game)
// ═══════════════════════════════════════════════════

// S86: office_state is in agent_handlers.rs — this duplicate removed to avoid conflicts.
