#![allow(dead_code)]

// Sub-modules for logical grouping (S61 refactor — phase 1)
// These contain extracted functions. Original functions still in this file
// for backward compatibility. Will be migrated incrementally.
pub mod agents;
pub mod analytics;
pub mod auth;
pub mod channels;
pub mod config;
pub mod economy;
pub mod media;
pub mod memory;
pub mod pantheon;
pub mod projects;
pub mod sessions;
pub mod tools;

use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

/// Safely truncate a string to at most `max_chars` characters,
/// avoiding panics on multi-byte UTF-8 boundaries.
pub fn truncate_str(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }
    // Find the last char boundary at or before max_chars bytes
    let mut end = max_chars;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── LocalStorage auth token helpers ──

pub fn get_auth_token() -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("zeus_auth_token").ok().flatten())
        .filter(|t| !t.is_empty())
}

pub fn set_auth_token(token: &str) {
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
    {
        let _ = storage.set_item("zeus_auth_token", token);
    }
}

pub fn clear_auth_token() {
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
    {
        let _ = storage.remove_item("zeus_auth_token");
    }
}

pub fn get_auth_provider() -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("zeus_auth_provider").ok().flatten())
        .filter(|t| !t.is_empty())
}

pub fn set_auth_provider(provider: &str) {
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
    {
        let _ = storage.set_item("zeus_auth_provider", provider);
    }
}

fn auth_bearer() -> Option<String> {
    get_auth_token().map(|t| format!("Bearer {}", t))
}

fn string_or_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => s.parse().map_err(serde::de::Error::custom),
        _ => Ok(0.0),
    }
}

pub async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let mut req = Request::get(url);
    if let Some(auth) = auth_bearer() { req = req.header("Authorization", &auth); }
    let resp = req.send().await.map_err(|e| format!("fetch error: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(if body.is_empty() { format!("HTTP {}", status) } else { format!("HTTP {}: {}", status, body) });
    }
    resp.json::<T>().await.map_err(|e| format!("parse error: {}", e))
}

pub async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
    url: &str,
    body: &B,
) -> Result<T, String> {
    let mut req = Request::post(url);
    if let Some(auth) = auth_bearer() { req = req.header("Authorization", &auth); }
    let resp = req
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(body).map_err(|e| format!("serialize: {}", e))?)
        .map_err(|e| format!("request: {}", e))?
        .send()
        .await
        .map_err(|e| format!("fetch error: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(if body.is_empty() { format!("HTTP {}", status) } else { format!("HTTP {}: {}", status, body) });
    }
    resp.json::<T>().await.map_err(|e| format!("parse error: {}", e))
}

pub async fn put_json<B: Serialize, T: for<'de> Deserialize<'de>>(
    url: &str,
    body: &B,
) -> Result<T, String> {
    let mut req = Request::put(url);
    if let Some(auth) = auth_bearer() { req = req.header("Authorization", &auth); }
    let resp = req
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(body).map_err(|e| format!("serialize: {}", e))?)
        .map_err(|e| format!("request: {}", e))?
        .send()
        .await
        .map_err(|e| format!("fetch error: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(if body.is_empty() { format!("HTTP {}", status) } else { format!("HTTP {}: {}", status, body) });
    }
    resp.json::<T>().await.map_err(|e| format!("parse error: {}", e))
}

pub async fn delete_endpoint(url: &str) -> Result<(), String> {
    let mut req = Request::delete(url);
    if let Some(auth) = auth_bearer() { req = req.header("Authorization", &auth); }
    let resp = req.send().await.map_err(|e| format!("fetch error: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(if body.is_empty() { format!("HTTP {}", status) } else { format!("HTTP {}: {}", status, body) });
    }
    Ok(())
}

pub async fn delete_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let mut req = Request::delete(url);
    if let Some(auth) = auth_bearer() { req = req.header("Authorization", &auth); }
    let resp = req.send().await.map_err(|e| format!("fetch error: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(if body.is_empty() { format!("HTTP {}", status) } else { format!("HTTP {}: {}", status, body) });
    }
    resp.json().await.map_err(|e| format!("JSON parse error: {}", e))
}

// Generic message response for mutations
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MsgResponse {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub id: String,
}

// ── Response types ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StatusResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub tools: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub auth_method: String,
    #[serde(default)]
    pub sessions_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionStats {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub active: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolStats {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub custom: u32,
    #[serde(default)]
    pub categories: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryStats {
    #[serde(default)]
    pub workspace_files: u32,
    #[serde(default)]
    pub memory_size_bytes: u64,
    #[serde(default)]
    pub total_entries: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StatsResponse {
    #[serde(default)]
    pub sessions: SessionStats,
    #[serde(default)]
    pub tools: ToolStats,
    #[serde(default)]
    pub memory: MemoryStats,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Channel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub message_count: u64,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_message_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChannelsResponse {
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Skill {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub audit_status: String,
    // Enriched fields (OpenClaw metadata)
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub primary_env: Option<String>,
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub requires: Option<SkillRequirements>,
    #[serde(default)]
    pub install_specs: Option<Vec<SkillInstallSpec>>,
    #[serde(default)]
    pub tools_count: usize,
    #[serde(default)]
    pub command_dispatch: Option<SkillDispatch>,
    // Full detail fields (only from GET /v1/skills/:id)
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub frontmatter: Option<std::collections::HashMap<String, String>>,
}

fn default_true() -> bool { true }

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillRequirements {
    #[serde(default)]
    pub bins: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
    #[serde(default)]
    pub config: Option<Vec<String>>,
    #[serde(default)]
    pub satisfied: bool,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillInstallSpec {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillDispatch {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub arg_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillsResponse {
    #[serde(default)]
    pub skills: Vec<Skill>,
    #[serde(default)]
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillCategoriesResponse {
    #[serde(default)]
    pub categories: Vec<SkillCategory>,
    #[serde(default)]
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillCategory {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct McpServer {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tools_count: u32,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub error_rate: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct McpServersResponse {
    #[serde(default)]
    pub servers: Vec<McpServer>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Session {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub duration_seconds: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionsResponse {
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub total: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ConfigResponse {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub sessions: String,
    #[serde(default)]
    pub max_iterations: u32,
    #[serde(default)]
    pub max_subagent_iterations: u32,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub mnemosyne: Option<serde_json::Value>,
    #[serde(default)]
    pub athena: Option<serde_json::Value>,
    #[serde(default)]
    pub aegis: Option<serde_json::Value>,
    #[serde(default)]
    pub hermes: Option<serde_json::Value>,
    #[serde(default)]
    pub prometheus: Option<serde_json::Value>,
    #[serde(default)]
    pub nous: Option<serde_json::Value>,
    #[serde(default)]
    pub talos: Option<serde_json::Value>,
    #[serde(default)]
    pub channels: Option<serde_json::Value>,
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
    #[serde(default)]
    pub search: Option<serde_json::Value>,
    #[serde(default)]
    pub gateway: Option<serde_json::Value>,
    #[serde(default)]
    pub session_compaction: Option<serde_json::Value>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub obsidian_vault: String,
    #[serde(default)]
    pub mnemosyne_db: String,
}

impl ConfigResponse {
    pub fn gateway(&self) -> String {
        self.gateway.as_ref()
            .map(|g| {
                let host = g.get("host").and_then(|v| v.as_str()).unwrap_or("127.0.0.1");
                let port = g.get("port").and_then(|v| v.as_u64()).unwrap_or(8080);
                format!("{}:{}", host, port)
            })
            .unwrap_or_else(|| "127.0.0.1:8080".to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TuiConfig {
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub vim_mode: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OllamaConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub preferred_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default)]
    pub use_oauth: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryResponse {
    #[serde(default)]
    pub context_length: u64,
    #[serde(default)]
    pub memory: String,
    #[serde(default)]
    pub daily: String,
    #[serde(default)]
    pub files_indexed: u32,
    #[serde(default)]
    pub total_chunks: u32,
    #[serde(default)]
    pub embedding_model: String,
    #[serde(default)]
    pub last_reindex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified: String,
    #[serde(default)]
    pub chunk_count: u32,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub last_indexed: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryFilesResponse {
    #[serde(default)]
    pub files: Vec<MemoryFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryFileContent {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CostsResponse {
    #[serde(default)]
    pub today: f64,
    #[serde(default)]
    pub this_week: f64,
    #[serde(default)]
    pub this_month: f64,
    #[serde(default)]
    pub projected_monthly: f64,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub budget_limit: f64,
    #[serde(default)]
    pub session_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TokensResponse {
    #[serde(default)]
    pub total_input_tokens: u64,
    #[serde(default)]
    pub total_output_tokens: u64,
    #[serde(default)]
    pub by_model: serde_json::Value,
    #[serde(default)]
    pub by_session: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Threat {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub threat_type: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ThreatsResponse {
    #[serde(default)]
    pub threats: Vec<Threat>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ApiKey {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub env_var: String,
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct KeysResponse {
    #[serde(default)]
    pub keys: Vec<ApiKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GlobalPerms {
    #[serde(default)]
    pub shell_access: bool,
    #[serde(default)]
    pub file_write: bool,
    #[serde(default)]
    pub web_access: bool,
    #[serde(default)]
    pub level: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PermissionsResponse {
    #[serde(default)]
    pub global: GlobalPerms,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PipelineStage {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub messages_processed: u64,
    #[serde(default)]
    pub avg_latency_ms: u64,
    #[serde(default)]
    pub error_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PipelineStatsResponse {
    #[serde(default)]
    pub stages: Vec<PipelineStage>,
    #[serde(default)]
    pub total_messages: u64,
    #[serde(default)]
    pub uptime_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ActivityEvent {
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ActivityResponse {
    #[serde(default)]
    pub events: Vec<ActivityEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Project {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub budget: f64,
    #[serde(default)]
    pub spent: f64,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub mission_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProjectsResponse {
    #[serde(default)]
    pub projects: Vec<Project>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkAgent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub address: String,
    #[serde(default, rename = "type")]
    pub agent_type: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub autonomy: String,
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    pub soul: String,
    #[serde(default)]
    pub tasks: u32,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub created: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkAgentsResponse {
    #[serde(default)]
    pub agents: Vec<NetworkAgent>,
}

// ── Typed fetch functions ──

pub async fn fetch_status() -> Result<StatusResponse, String> {
    fetch_json("/v1/status").await
}

pub async fn fetch_stats() -> Result<StatsResponse, String> {
    fetch_json("/v1/stats").await
}

pub async fn fetch_channels() -> Result<ChannelsResponse, String> {
    fetch_json("/v1/channels").await
}

pub async fn fetch_skills() -> Result<SkillsResponse, String> {
    fetch_json("/v1/skills").await
}

pub async fn fetch_mcp_servers() -> Result<McpServersResponse, String> {
    fetch_json("/v1/mcp/servers").await
}

pub async fn fetch_sessions() -> Result<SessionsResponse, String> {
    fetch_json("/v1/sessions").await
}

pub async fn fetch_config() -> Result<ConfigResponse, String> {
    fetch_json("/v1/config").await
}

pub async fn fetch_memory() -> Result<MemoryResponse, String> {
    fetch_json("/v1/memory").await
}

pub async fn fetch_memory_files() -> Result<MemoryFilesResponse, String> {
    fetch_json("/v1/memory/files").await
}

pub async fn fetch_memory_file(path: &str) -> Result<MemoryFileContent, String> {
    fetch_json(&format!("/v1/memory/files/{}", path)).await
}

pub async fn fetch_costs() -> Result<CostsResponse, String> {
    fetch_json("/v1/analytics/costs").await
}

pub async fn fetch_tokens() -> Result<TokensResponse, String> {
    fetch_json("/v1/analytics/tokens").await
}

pub async fn fetch_threats() -> Result<ThreatsResponse, String> {
    fetch_json("/v1/security/threats").await
}

pub async fn fetch_keys() -> Result<KeysResponse, String> {
    fetch_json("/v1/security/keys").await
}

pub async fn store_credential(name: &str, value: &str) -> Result<MsgResponse, String> {
    post_json("/v1/credentials", &serde_json::json!({ "name": name, "value": value })).await
}

pub async fn fetch_permissions() -> Result<PermissionsResponse, String> {
    fetch_json("/v1/security/permissions").await
}

pub async fn fetch_pipeline_stats() -> Result<PipelineStatsResponse, String> {
    fetch_json("/v1/pipeline/stats").await
}

pub async fn fetch_activity() -> Result<ActivityResponse, String> {
    fetch_json("/v1/activity").await
}

pub async fn fetch_projects() -> Result<ProjectsResponse, String> {
    fetch_json("/v1/projects").await
}

pub async fn fetch_network_agents() -> Result<NetworkAgentsResponse, String> {
    fetch_json("/v1/network/agents").await
}

pub async fn fetch_agents() -> Result<NetworkAgentsResponse, String> {
    fetch_json("/v1/agents").await
}

pub async fn fetch_agent(id: &str) -> Result<NetworkAgent, String> {
    fetch_json(&format!("/v1/agents/{}", id)).await
}

// ── Session detail ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub tool_calls: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub messages: Vec<SessionMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionStatsDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub user_messages: u32,
    #[serde(default)]
    pub assistant_messages: u32,
    #[serde(default)]
    pub tool_calls: u32,
    #[serde(default)]
    pub duration_seconds: u64,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub last_activity: String,
}

pub async fn fetch_session(id: &str) -> Result<SessionDetail, String> {
    fetch_json(&format!("/v1/sessions/{}", id)).await
}

pub async fn fetch_session_stats(id: &str) -> Result<SessionStatsDetail, String> {
    fetch_json(&format!("/v1/sessions/{}/stats", id)).await
}

// ── Session replay ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReplayTurn {
    #[serde(default)] pub index: u32,
    #[serde(default)] pub timestamp: String,
    #[serde(default)] pub role: String,
    #[serde(default)] pub content: String,
    #[serde(default)] pub tool_calls: Vec<serde_json::Value>,
    #[serde(default)] pub tool_name: String,
    #[serde(default)] pub tool_results: String,
    #[serde(default)] pub thinking: String,
    #[serde(default)] pub token_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReplayStats {
    #[serde(default)] pub total_turns: u32,
    #[serde(default)] pub total_tokens: u64,
    #[serde(default)] pub duration_ms: u64,
    #[serde(default)] pub tools_used: Vec<String>,
    #[serde(default)] pub model_used: String,
    #[serde(default, deserialize_with = "string_or_f64")] pub cost_estimate: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ReplayResponse {
    #[serde(default)]
    pub entries: Vec<ReplayTurn>,
}

pub async fn fetch_session_replay(id: &str) -> Result<Vec<ReplayTurn>, String> {
    let resp: ReplayResponse = fetch_json(&format!("/v1/sessions/{}/replay", id)).await?;
    Ok(resp.entries)
}

pub async fn fetch_replay_stats(id: &str) -> Result<ReplayStats, String> {
    fetch_json(&format!("/v1/sessions/{}/stats", id)).await
}

// ── Mutation endpoints ──

#[derive(Serialize)]
pub struct CreateAgentReq {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomy_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

pub async fn create_agent(req: &CreateAgentReq) -> Result<MsgResponse, String> {
    post_json("/v1/agents", req).await
}

pub async fn delete_agent(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/agents/{}", id)).await
}

#[derive(Serialize)]
pub struct UpdateAgentReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soul: Option<String>,
}

pub async fn update_agent(id: &str, req: &UpdateAgentReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/agents/{}", id), req).await
}

#[derive(Serialize)]
pub struct DispatchMissionReq {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

pub async fn dispatch_mission(req: &DispatchMissionReq) -> Result<ChatResponse, String> {
    post_json("/v1/chat", req).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChatResponse {
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub session_id: String,
}

#[derive(Serialize)]
pub struct InstallSkillReq {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

pub async fn install_skill(req: &InstallSkillReq) -> Result<MsgResponse, String> {
    post_json("/v1/skills", req).await
}

pub async fn toggle_skill(id: &str, enable: bool) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/skills/{}", id), &serde_json::json!({ "enabled": enable })).await
}

pub async fn delete_skill(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/skills/{}", id)).await
}

pub async fn fetch_skill(id: &str) -> Result<Skill, String> {
    fetch_json(&format!("/v1/skills/{}", id)).await
}

pub async fn search_skills(query: Option<&str>, category: Option<&str>) -> Result<SkillsResponse, String> {
    let mut params = Vec::new();
    if let Some(q) = query
        && !q.is_empty()
    {
        params.push(format!("q={}", q));
    }
    if let Some(cat) = category
        && !cat.is_empty()
    {
        params.push(format!("category={}", cat));
    }
    let url = if params.is_empty() {
        "/v1/skills/search".to_string()
    } else {
        format!("/v1/skills/search?{}", params.join("&"))
    };
    fetch_json(&url).await
}

pub async fn fetch_skill_categories() -> Result<SkillCategoriesResponse, String> {
    fetch_json("/v1/skills/categories").await
}

#[derive(Serialize)]
pub struct ConnectMcpReq {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: String,
}

pub async fn connect_mcp(req: &ConnectMcpReq) -> Result<MsgResponse, String> {
    post_json("/v1/mcp/servers", req).await
}

pub async fn disconnect_mcp(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/mcp/servers/{}", id)).await
}

#[derive(Serialize)]
pub struct CreateChannelReq {
    pub channel_type: String,
    pub name: String,
    pub config: serde_json::Value,
}

#[derive(Serialize)]
pub struct UpdateChannelReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TestChannelResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChannelStatusResponse {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub uptime_seconds: u64,
    #[serde(default)]
    pub last_message_at: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

pub async fn create_channel(req: &CreateChannelReq) -> Result<MsgResponse, String> {
    post_json("/v1/channels", req).await
}

pub async fn update_channel(id: &str, req: &UpdateChannelReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/channels/{}", id), req).await
}

pub async fn delete_channel(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/channels/{}", id)).await
}

pub async fn test_channel(id: &str) -> Result<TestChannelResponse, String> {
    post_json(&format!("/v1/channels/{}/test", id), &serde_json::json!({})).await
}

#[derive(Serialize)]
pub struct CreateProjectReq {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub budget: f64,
}

pub async fn create_project(req: &CreateProjectReq) -> Result<MsgResponse, String> {
    post_json("/v1/projects", req).await
}

pub async fn save_config(config: &serde_json::Value) -> Result<MsgResponse, String> {
    put_json("/v1/config", config).await
}

// ── Provider configuration ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProvidersResponse {
    #[serde(default)]
    pub providers: std::collections::HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub default_provider: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TestResult {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub models: Vec<String>,
}

pub async fn fetch_providers() -> Result<ProvidersResponse, String> {
    fetch_json("/v1/config/providers").await
}

pub async fn test_provider_connection(provider: &str, api_key: Option<&str>, url: Option<&str>) -> Result<TestResult, String> {
    let mut body = serde_json::json!({ "provider": provider });
    if let Some(k) = api_key { body["api_key"] = serde_json::Value::String(k.to_string()); }
    if let Some(u) = url { body["url"] = serde_json::Value::String(u.to_string()); }
    post_json("/v1/config/test", &body).await
}

// ── Analytics provider costs ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProviderCost {
    #[serde(default, alias = "name")]
    pub provider: String,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub requests: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProviderCostsResponse {
    #[serde(default)]
    pub providers: Vec<ProviderCost>,
    #[serde(default)]
    pub budget_limit: f64,
}

pub async fn fetch_provider_costs() -> Result<ProviderCostsResponse, String> {
    fetch_json("/v1/analytics/providers").await
}

// ── Security allowlist ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AllowlistResponse {
    #[serde(default)]
    pub allowlist: Vec<String>,
}

pub async fn fetch_allowlist() -> Result<AllowlistResponse, String> {
    fetch_json("/v1/security/allowlist").await
}

pub async fn update_allowlist(commands: &[String]) -> Result<MsgResponse, String> {
    put_json("/v1/security/allowlist", &serde_json::json!({ "allowlist": commands })).await
}

// ── Auth ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuthStatusResponse {
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuthLoginResponse {
    #[serde(default)]
    pub authorize_url: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuthTokenResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub message: String,
}

pub async fn fetch_auth_status() -> Result<AuthStatusResponse, String> {
    fetch_json("/v1/auth/status").await
}

pub async fn auth_login() -> Result<AuthLoginResponse, String> {
    post_json("/v1/auth/login", &serde_json::json!({})).await
}

pub async fn auth_login_provider(provider: &str, redirect_uri: &str, state: &str, code_verifier: &str) -> Result<AuthLoginResponse, String> {
    post_json("/v1/auth/login", &serde_json::json!({
        "provider": provider,
        "redirect_uri": redirect_uri,
        "state": state,
        "code_verifier": code_verifier,
    })).await
}

pub async fn auth_token(token: &str) -> Result<AuthTokenResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({ "token": token })).await
}

pub async fn auth_logout() -> Result<AuthTokenResponse, String> {
    post_json("/v1/auth/logout", &serde_json::json!({})).await
}

/// Map a provider name to its canonical environment variable name.
/// Used when routing API keys to CredentialVault via POST /v1/credentials.
fn provider_to_key_name(provider: &str) -> &str {
    match provider {
        "anthropic"  => "ANTHROPIC_API_KEY",
        "openai"     => "OPENAI_API_KEY",
        "google"     => "GOOGLE_API_KEY",
        "groq"       => "GROQ_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "mistral"    => "MISTRAL_API_KEY",
        "together"   => "TOGETHER_API_KEY",
        "fireworks"  => "FIREWORKS_API_KEY",
        "azure"      => "AZURE_OPENAI_API_KEY",
        "bedrock"    => "AWS_ACCESS_KEY_ID",
        // Fallback: log a warning but return a deterministic name so
        // credential save doesn't silently fail (S86 P0 fix).
        // Callers still get an Err from auth_save_credentials for truly
        // unknown providers, but at least provider_to_key_name itself
        // never returns a useless sentinel.
        _other => "UNKNOWN_API_KEY"
    }
}

/// Store an API key in CredentialVault via POST /v1/credentials (S54 Track A).
/// Previously routed to POST /v1/auth/token → OAuthManager; now goes to
/// CredentialVault → keychain / config.credentials fallback.
pub async fn auth_save_credentials(provider: &str, api_key: &str) -> Result<AuthCallbackResponse, String> {
    let key_name = provider_to_key_name(provider);
    if key_name == "UNKNOWN_API_KEY" {
        // S86 P0: Instead of hard-failing, derive a reasonable env var name
        // from the provider id (e.g. "deepseek" → "DEEPSEEK_API_KEY").
        // This lets new providers work without code changes.
        let derived = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
        return post_json("/v1/credentials", &serde_json::json!({
            "name": derived,
            "value": api_key,
        })).await;
    }
    post_json("/v1/credentials", &serde_json::json!({
        "name": key_name,
        "value": api_key,
    })).await
}

/// Store an OAuth setup token (sk-ant-oat01-...) via /v1/auth/token
pub async fn auth_store_oauth_token(token: &str) -> Result<AuthCallbackResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({
        "token": token,
    })).await
}

pub async fn auth_oauth_callback(code: &str, code_verifier: &str, provider: &str, redirect_uri: &str) -> Result<AuthCallbackResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({
        "code": code,
        "code_verifier": code_verifier,
        "provider": provider,
        "redirect_uri": redirect_uri,
    })).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuthCallbackResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub message: String,
}

pub async fn update_permissions(perms: &GlobalPerms) -> Result<MsgResponse, String> {
    put_json("/v1/security/permissions", perms).await
}

// ── MCP tools & logs ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct McpTool {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub server_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct McpToolsResponse {
    #[serde(default)]
    pub tools: Vec<McpTool>,
}

pub async fn fetch_mcp_tools(server_id: &str) -> Result<McpToolsResponse, String> {
    fetch_json(&format!("/v1/mcp/servers/{}/tools", server_id)).await
}

pub async fn test_mcp_tool(tool_name: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/mcp/tools/{}/test", tool_name), &serde_json::json!({})).await
}

// ── Memory search & management ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemorySearchResult {
    // File search fields
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub snippet: String,
    // Hybrid search fields
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub citation: Option<String>,
    // Common
    #[serde(default)]
    pub score: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemorySearchResponse {
    #[serde(default)]
    pub results: Vec<MemorySearchResult>,
    #[serde(default)]
    pub search_method: String,
}

pub async fn search_memory(query: &str) -> Result<MemorySearchResponse, String> {
    post_json("/v1/memory/search", &serde_json::json!({ "query": query })).await
}

pub async fn remember(fact: &str) -> Result<MsgResponse, String> {
    post_json("/v1/memory/remember", &serde_json::json!({ "fact": fact })).await
}

pub async fn add_note(content: &str) -> Result<MsgResponse, String> {
    post_json("/v1/memory/note", &serde_json::json!({ "content": content })).await
}

// ── Memory sync ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemorySyncResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub files_scanned: u32,
    #[serde(default)]
    pub files_changed: u32,
    #[serde(default)]
    pub files_unchanged: u32,
    #[serde(default)]
    pub chunks_embedded: u32,
    #[serde(default)]
    pub cache_hits: u32,
    #[serde(default)]
    pub cache_misses: u32,
    #[serde(default)]
    pub sessions_indexed: u32,
    #[serde(default)]
    pub errors: Vec<String>,
}

pub async fn fetch_reindex() -> Result<MemorySyncResponse, String> {
    post_json("/v1/memory/sync", &serde_json::json!({})).await
}

// ── Memory tracked files ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TrackedFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub last_indexed: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TrackedFilesResponse {
    #[serde(default)]
    pub files: Vec<TrackedFile>,
    #[serde(default)]
    pub available: bool,
}

pub async fn fetch_tracked_files() -> Result<TrackedFilesResponse, String> {
    fetch_json("/v1/memory/files").await
}

// ── Session / Project delete ──

pub async fn delete_session(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/sessions/{}", id)).await
}

pub async fn delete_project(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/projects/{}", id)).await
}

// ── Doctor ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DoctorCheck {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DoctorResponse {
    #[serde(default)]
    pub checks: Vec<DoctorCheck>,
    #[serde(default)]
    pub overall: String,
    #[serde(default)]
    pub healthy: bool,
}

pub async fn fetch_doctor() -> Result<DoctorResponse, String> {
    fetch_json("/v1/doctor").await
}

// ── Session raw / audit / tools ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionRawResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub data: String,
}

pub async fn fetch_session_raw(id: &str) -> Result<SessionRawResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/raw", id)).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AuditEntry {
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub tool: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionAuditResponse {
    #[serde(default)]
    pub entries: Vec<AuditEntry>,
}

pub async fn fetch_session_audit(id: &str) -> Result<SessionAuditResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/audit", id)).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolExecution {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SessionToolsResponse {
    #[serde(default)]
    pub tools: Vec<ToolExecution>,
}

pub async fn fetch_session_tools(id: &str) -> Result<SessionToolsResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/tools", id)).await
}

// ── Tools listing + execution ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolDef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub calls: u64,
}

impl ToolDef {
    /// Derive category from tool name prefix when backend doesn't supply one.
    pub fn with_derived_category(mut self) -> Self {
        if !self.category.is_empty() {
            return self;
        }
        self.category = match self.name.as_str() {
            "read_file" | "write_file" | "edit_file" | "list_dir" => "filesystem",
            "shell" => "shell",
            "web_fetch" => "web",
            "spawn" => "agent",
            "message" => "messaging",
            n if n.starts_with("browser_") => "browser",
            n if n.starts_with("git_") => "git",
            n if n.starts_with("calendar_") => "calendar",
            n if n.starts_with("notes_") || n.starts_with("note_") => "notes",
            n if n.starts_with("reminder_") => "reminders",
            n if n.starts_with("contacts_") || n.starts_with("contact_") => "contacts",
            n if n.starts_with("safari_") => "safari",
            n if n.starts_with("mail_") || n.starts_with("email_") => "mail",
            n if n.starts_with("imessage_") => "imessage",
            n if n.starts_with("music_") => "music",
            n if n.starts_with("ui_") => "ui",
            n if n.starts_with("pdf_") => "pdf",
            n if n.starts_with("bluetooth_") || n.starts_with("bt_") => "bluetooth",
            n if n.starts_with("defaults_") || n.starts_with("config_") => "defaults",
            n if n.starts_with("network_") || n == "ping" || n == "port_check" => "network",
            n if n.starts_with("telegram_") => "telegram",
            n if n.starts_with("homebrew_") || n.starts_with("brew_") => "homebrew",
            n if n.starts_with("voice_") || n == "speak_text" || n == "stt" => "voice",
            n if n.starts_with("file_") || n == "find_files" => "files",
            n if n.starts_with("system_") || n == "process_list" || n == "clipboard"
                || n == "screenshot" || n == "volume" || n == "wifi"
                || n == "focus" || n == "spotlight_search" => "system",
            _ => "other",
        }
        .to_string();
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolsResponse {
    #[serde(default)]
    pub tools: Vec<ToolDef>,
}

/// Fetch all available tools from the API and derive categories.
pub async fn get_tools() -> Result<ToolsResponse, String> {
    let mut resp: ToolsResponse = fetch_json("/v1/tools").await?;
    resp.tools = resp.tools.into_iter().map(|t| t.with_derived_category()).collect();
    Ok(resp)
}

/// Get a single tool by name (filters from the full list).
pub async fn get_tool(name: &str) -> Result<Option<ToolDef>, String> {
    let resp = get_tools().await?;
    Ok(resp.tools.into_iter().find(|t| t.name == name))
}

/// Backwards-compatible alias.
pub async fn fetch_tools() -> Result<ToolsResponse, String> {
    get_tools().await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolExecResponse {
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub success: bool,
}

pub async fn execute_tool(name: &str, args: &serde_json::Value) -> Result<ToolExecResponse, String> {
    post_json(&format!("/v1/tools/{}", name), &serde_json::json!({ "arguments": args })).await
}

// ── Analytics budgets ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Budget {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub limit: f64,
    #[serde(default)]
    pub spent: f64,
    #[serde(default)]
    pub period: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BudgetsResponse {
    #[serde(default)]
    pub budgets: Vec<Budget>,
}

pub async fn fetch_budgets() -> Result<BudgetsResponse, String> {
    fetch_json("/v1/analytics/budgets").await
}

// ── Network discover / messages ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DiscoveryResult {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkDiscoverResponse {
    #[serde(default)]
    pub mdns: Vec<DiscoveryResult>,
    #[serde(default)]
    pub tailscale: Vec<DiscoveryResult>,
    #[serde(default)]
    pub manual: Vec<DiscoveryResult>,
}

pub async fn fetch_network_discover() -> Result<NetworkDiscoverResponse, String> {
    fetch_json("/v1/network/discover").await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkMessage {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkMessagesResponse {
    #[serde(default)]
    pub messages: Vec<NetworkMessage>,
}

pub async fn fetch_network_messages() -> Result<NetworkMessagesResponse, String> {
    fetch_json("/v1/network/messages").await
}

pub async fn network_send(host: &str, port: Option<u16>, from_agent: &str, to_agent: Option<&str>, content: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/network/send", &serde_json::json!({
        "host": host, "port": port, "from_agent": from_agent, "to_agent": to_agent, "content": content,
    })).await
}

pub async fn network_broadcast(from_agent: &str, content: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/network/broadcast", &serde_json::json!({ "from_agent": from_agent, "content": content })).await
}

pub async fn economy_earn(agent_id: &str, tools_used: usize, complexity: &str, note: Option<&str>) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/earn", &serde_json::json!({
        "agent_id": agent_id, "tools_used": tools_used, "complexity": complexity, "note": note,
    })).await
}

pub async fn economy_mint(agent_id: &str, amount: u64, reason: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/mint", &serde_json::json!({ "agent_id": agent_id, "amount": amount, "reason": reason })).await
}

pub async fn economy_stake(agent_id: &str, amount: u64, purpose: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/stake", &serde_json::json!({ "agent_id": agent_id, "amount": amount, "purpose": purpose })).await
}

pub async fn economy_unstake(agent_id: &str, stake_id: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/unstake", &serde_json::json!({ "agent_id": agent_id, "stake_id": stake_id })).await
}

pub async fn economy_transfer(from: &str, to: &str, amount: u64, note: Option<&str>) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/transfer", &serde_json::json!({
        "from": from, "to": to, "amount": amount, "note": note,
    })).await
}

pub async fn hire_agent(caller_id: &str, task: &str, skill_name: Option<&str>, max_credits: u64) -> Result<serde_json::Value, String> {
    post_json("/v1/agents/hire", &serde_json::json!({
        "caller_id": caller_id, "task": task, "skill_name": skill_name, "input": {}, "max_credits": max_credits,
    })).await
}

pub async fn run_agent_task(task: &str, context: Option<&str>, model: Option<&str>, wait: bool) -> Result<serde_json::Value, String> {
    post_json("/v1/agents/run-task", &serde_json::json!({
        "task": task, "context": context, "model": model, "wait": wait, "max_iterations": 10,
    })).await
}

// ── Projects: single fetch, update, agent assignment ──

pub async fn fetch_project(id: &str) -> Result<Project, String> {
    fetch_json(&format!("/v1/projects/{}", id)).await
}

#[derive(Serialize)]
pub struct UpdateProjectReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
}

pub async fn update_project(id: &str, req: &UpdateProjectReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/projects/{}", id), req).await
}

#[derive(Serialize)]
pub struct AssignAgentsReq {
    pub agents: Vec<String>,
}

pub async fn assign_project_agents(id: &str, req: &AssignAgentsReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/projects/{}/agents", id), req).await
}

// ── Schedules ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Schedule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cron: String,
    #[serde(default)]
    pub task_type: String,
    #[serde(default)]
    pub task_payload: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: String,
    #[serde(default)]
    pub last_status: String,
    #[serde(default)]
    pub next_run: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SchedulesResponse {
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    #[serde(default)]
    pub total: u32,
}

#[derive(Serialize)]
pub struct CreateScheduleReq {
    pub name: String,
    pub cron: String,
    pub task_type: String,
    pub task_payload: String,
}

pub async fn fetch_schedules() -> Result<SchedulesResponse, String> {
    fetch_json("/v1/schedules").await
}

pub async fn create_schedule(req: &CreateScheduleReq) -> Result<MsgResponse, String> {
    post_json("/v1/schedules", req).await
}

pub async fn update_schedule(id: &str, body: &serde_json::Value) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/schedules/{}", id), body).await
}

pub async fn delete_schedule(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/schedules/{}", id)).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ScheduleHistoryResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub history: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

pub async fn fetch_schedule_history(id: &str) -> Result<ScheduleHistoryResponse, String> {
    fetch_json(&format!("/v1/schedules/{}/history", id)).await
}

// ── Approvals ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PendingApproval {
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
    #[serde(default)]
    pub status: serde_json::Value,
    /// Risk level computed client-side from tool_name + args via Aegis policy mapping.
    /// Populated after deserialization via `compute_risk()`.
    #[serde(skip)]
    pub risk: RiskLabel,
}

/// Serialization-friendly risk label (mirrors zeus_aegis::RiskLevel).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RiskLabel {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

impl RiskLabel {
    pub fn label(&self) -> &'static str {
        match self {
            RiskLabel::Low => "Low",
            RiskLabel::Medium => "Medium",
            RiskLabel::High => "High",
            RiskLabel::Critical => "Critical",
        }
    }

    pub fn color(&self) -> &'static str {
        match self {
            RiskLabel::Low => "rgba(34,197,94,0.9)",      // green
            RiskLabel::Medium => "rgba(251,191,36,0.9)",  // amber
            RiskLabel::High => "rgba(249,115,22,0.9)",    // orange
            RiskLabel::Critical => "rgba(239,68,68,0.9)", // red
        }
    }

    /// Compute from tool_name + args using same heuristics as zeus-aegis::tool_risk.
    pub fn from_tool(tool_name: &str, args: &serde_json::Value) -> Self {
        let tool = tool_name.to_lowercase();

        // Shell / bash — pattern-match command arg
        if tool == "shell" || tool == "bash" || tool == "run_command" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let c = cmd.to_lowercase();
                if c.contains("rm -rf") || c.contains("rm -fr") || c.contains("mkfs")
                    || c.contains("dd if=") || c.contains("wipefs") || c.contains("> /dev/")
                {
                    return RiskLabel::Critical;
                }
                if c.contains("rm ") || c.contains("sudo ") || c.contains("chmod ")
                    || c.contains("chown ") || c.contains("kill ") || c.contains("pkill")
                    || c.contains("systemctl") || c.contains("apt ") || c.contains("yum ")
                {
                    return RiskLabel::High;
                }
                if c.contains("curl ") || c.contains("wget ") || c.contains("git push")
                    || c.contains("git commit") || c.contains("> ") || c.contains(">>")
                    || c.contains("cp ") || c.contains("mv ") || c.contains("mkdir ")
                {
                    return RiskLabel::Medium;
                }
                return RiskLabel::Low;
            }
            return RiskLabel::Medium;
        }

        // File system
        match tool.as_str() {
            "write_file" | "create_file" | "edit_file" | "apply_patch" => RiskLabel::Medium,
            "delete_file" | "remove_file" | "trash" => RiskLabel::High,
            "read_file" | "list_dir" | "glob" | "search_files" => RiskLabel::Low,
            "web_fetch" | "web_search" | "deep_research" => RiskLabel::Low,
            "send_message" | "discord_send_message" | "telegram_send_message" => RiskLabel::Medium,
            "discord_delete_message" | "telegram_delete_message" => RiskLabel::High,
            "execute_code" | "run_python" | "run_script" => RiskLabel::High,
            "spawn" | "spawn_agent" => RiskLabel::Medium,
            "read_secret" | "get_secret" | "vault_read" => RiskLabel::High,
            "write_secret" | "set_secret" | "vault_write" => RiskLabel::Critical,
            _ => RiskLabel::Medium,
        }
    }
}

impl PendingApproval {
    /// Compute and cache the risk label from tool_name + args.
    pub fn with_risk(mut self) -> Self {
        self.risk = RiskLabel::from_tool(&self.tool_name, &self.args);
        self
    }
}

pub async fn fetch_approvals() -> Result<Vec<PendingApproval>, String> {
    let raw: Vec<PendingApproval> = fetch_json("/v1/approvals").await?;
    Ok(raw.into_iter().map(|a| a.with_risk()).collect())
}

pub async fn approve_execution(id: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/approvals/{}/approve", id), &serde_json::json!({})).await
}

pub async fn deny_execution(id: &str, reason: Option<&str>) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/approvals/{}/deny", id), &serde_json::json!({ "reason": reason })).await
}

// ── Agent Spawn ──

#[derive(Serialize)]
pub struct SpawnAgentReq {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soul: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

pub async fn spawn_agent(req: &SpawnAgentReq) -> Result<MsgResponse, String> {
    post_json("/v1/agents/spawn", req).await
}

// ── TTS (provider-agnostic, proxied through nginx) ──
//
// Piper TTS — proxied via nginx at /tts/ → configured Piper backend
// Returns JSON with audio_base64 field containing base64-encoded WAV.

pub struct TtsConfig {
    pub base_path: &'static str,
    pub synthesize_path: &'static str,
}

pub static TTS_CONFIG: TtsConfig = TtsConfig {
    base_path: "/tts",
    synthesize_path: "/synthesize",
};

fn decode_base64_audio(b64: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let raw = window.atob(b64).map_err(|e| format!("atob failed: {:?}", e))?;
    Ok(raw.chars().map(|c| c as u8).collect())
}

// ── Whisper STT (Speech-to-Text) ──

#[derive(Clone, Debug, Deserialize, Default)]
pub struct SttResponse {
    #[serde(default)]
    pub text: String,
}

/// Transcribe audio via Whisper STT (POST /stt/inference, multipart form).
/// Takes raw audio bytes (WebM/Opus format from MediaRecorder). Returns transcribed text.
pub async fn stt_transcribe(audio: &[u8]) -> Result<String, String> {
    stt_transcribe_with_mime(audio, "audio/webm").await
}

pub async fn stt_transcribe_with_mime(audio: &[u8], mime_type: &str) -> Result<String, String> {
    // Build multipart form data via JS FormData + Blob
    let form = web_sys::FormData::new().map_err(|e| format!("FormData: {:?}", e))?;

    // Create a Blob from audio bytes with the actual recorded MIME type
    let uint8 = js_sys::Uint8Array::new_with_length(audio.len() as u32);
    uint8.copy_from(audio);
    let parts = js_sys::Array::new();
    parts.push(&uint8.buffer());
    let blob_opts = web_sys::BlobPropertyBag::new();
    blob_opts.set_type(mime_type);
    let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &blob_opts)
        .map_err(|e| format!("Blob: {:?}", e))?;

    // Determine file extension from MIME type
    let ext = if mime_type.contains("ogg") { "ogg" }
        else if mime_type.contains("mp4") { "mp4" }
        else { "webm" };
    form.append_with_blob_and_filename("file", &blob, &format!("audio.{}", ext))
        .map_err(|e| format!("FormData append: {:?}", e))?;
    form.append_with_str("temperature", "0.0")
        .map_err(|e| format!("FormData append: {:?}", e))?;
    form.append_with_str("response_format", "json")
        .map_err(|e| format!("FormData append: {:?}", e))?;

    // Use raw fetch (gloo-net doesn't support FormData bodies directly)
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form.into());

    let request = web_sys::Request::new_with_str_and_init("/stt/inference", &opts)
        .map_err(|e| format!("Request: {:?}", e))?;

    let window = web_sys::window().ok_or("No window")?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch: {:?}", e))?;

    let resp: web_sys::Response = resp_val.dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;

    if !resp.ok() {
        let status = resp.status();
        return Err(format!("STT HTTP {}", status));
    }

    let text_promise = resp.text().map_err(|e| format!("STT text: {:?}", e))?;
    let text_val = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("STT text read: {:?}", e))?;

    let text = text_val.as_string().unwrap_or_default();
    let parsed: SttResponse = serde_json::from_str(&text)
        .map_err(|e| format!("STT parse: {}", e))?;

    Ok(parsed.text.trim().to_string())
}

// ── Teams ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Team {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub supervisor_id: String,
    #[serde(default)]
    pub routing_strategy: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TeamsResponse {
    #[serde(default)]
    pub teams: Vec<Team>,
}

pub async fn fetch_teams() -> Result<TeamsResponse, String> {
    fetch_json("/v1/teams").await
}

pub async fn create_team(name: &str, description: &str, routing_strategy: &str) -> Result<MsgResponse, String> {
    let body = serde_json::json!({
        "name": name,
        "description": description,
        "routing_strategy": routing_strategy,
    });
    post_json("/v1/teams", &body).await
}

pub async fn create_agent_team(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/agents/team", body).await
}

// ── Extensions ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Extension {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub audit_status: String,
    #[serde(default)]
    pub install_date: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub log_entries: Vec<String>,
    #[serde(default)]
    pub tools_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ExtensionsResponse {
    #[serde(default)]
    pub extensions: Vec<Extension>,
}

pub async fn fetch_extensions() -> Result<ExtensionsResponse, String> {
    fetch_json("/v1/extensions").await
}

pub async fn install_extension(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/extensions", body).await
}

pub async fn toggle_extension(id: &str, enabled: bool) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/extensions/{}", id), &serde_json::json!({"enabled": enabled})).await
}

pub async fn delete_extension(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/extensions/{}", id)).await
}

// ── Sandbox ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SandboxPolicy {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sandbox_level: String,
    #[serde(default)]
    pub shell_allowlist: Vec<String>,
    #[serde(default)]
    pub filesystem_boundaries: Vec<String>,
    #[serde(default)]
    pub network_access: String,
    #[serde(default)]
    pub network_allowlist: Vec<String>,
    #[serde(default)]
    pub network_blocklist: Vec<String>,
    #[serde(default)]
    pub approval_patterns: Vec<String>,
    #[serde(default)]
    pub max_memory_mb: u32,
    #[serde(default)]
    pub max_cpu_percent: u32,
    #[serde(default)]
    pub max_execution_seconds: u32,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SandboxPoliciesResponse {
    #[serde(default)]
    pub policies: Vec<SandboxPolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SandboxExecution {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub policy_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub memory_used_mb: u32,
    #[serde(default)]
    pub cpu_percent: f64,
    #[serde(default)]
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SandboxResourceUsage {
    #[serde(default)]
    pub cpu_current: f64,
    #[serde(default)]
    pub cpu_limit: f64,
    #[serde(default)]
    pub memory_current_mb: f64,
    #[serde(default)]
    pub memory_limit_mb: f64,
    #[serde(default)]
    pub disk_current_mb: f64,
    #[serde(default)]
    pub disk_limit_mb: f64,
    #[serde(default)]
    pub network_bytes_in: u64,
    #[serde(default)]
    pub network_bytes_out: u64,
}

pub async fn fetch_sandbox_policies() -> Result<SandboxPoliciesResponse, String> {
    fetch_json("/v1/sandbox/policies").await
}

pub async fn create_sandbox_policy(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/sandbox/policies", body).await
}

pub async fn delete_sandbox_policy(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/sandbox/policies/{}", id)).await
}

pub async fn run_sandbox_command(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/sandbox/execute", body).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TaskCost {
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub task_name: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default, deserialize_with = "string_or_f64")]
    pub cost: f64,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FallbackChainEntry {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub enabled: bool,
}

pub async fn fetch_task_costs() -> Result<Vec<TaskCost>, String> {
    // Task-level costs come from the sessions endpoint with per-session cost breakdowns
    let resp: serde_json::Value = fetch_json("/v1/analytics/sessions").await?;
    let sessions = resp.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    Ok(sessions.iter().filter_map(|s| {
        Some(TaskCost {
            task_id: s.get("session_id")?.as_str()?.to_string(),
            task_name: String::new(),
            agent_id: String::new(),
            model: s.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string(),
            tokens: s.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
            cost: s.get("estimated_cost").and_then(|c| c.as_f64()).unwrap_or(0.0),
            timestamp: s.get("created").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        })
    }).collect())
}

pub async fn fetch_fallback_chain() -> Result<Vec<FallbackChainEntry>, String> {
    // Fallback chain is derived from provider data
    let resp: ProviderCostsResponse = fetch_json("/v1/analytics/providers").await?;
    Ok(resp.providers.iter().enumerate().map(|(i, p)| FallbackChainEntry {
        provider: p.provider.clone(),
        model: String::new(),
        priority: i as u32,
        enabled: p.tokens > 0 || p.requests > 0,
    }).collect())
}

// ── Daily analytics (server-side aggregation) ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DailyAnalytics {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub estimated_cost: f64,
    #[serde(default)]
    pub session_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DailyAnalyticsResponse {
    #[serde(default)]
    pub days: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub daily: Vec<DailyAnalytics>,
}

pub async fn fetch_daily_analytics(days: u32) -> Result<DailyAnalyticsResponse, String> {
    fetch_json(&format!("/v1/analytics/daily?days={}", days)).await
}

// ── Per-model analytics ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModelAnalytics {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub requests: u32,
    #[serde(default)]
    pub estimated_cost: f64,
    #[serde(default)]
    pub avg_tokens_per_request: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModelsAnalyticsResponse {
    #[serde(default)]
    pub models: Vec<ModelAnalytics>,
    #[serde(default)]
    pub total_models: u32,
}

pub async fn fetch_model_analytics() -> Result<ModelsAnalyticsResponse, String> {
    fetch_json("/v1/analytics/models").await
}

// ── Security audit log ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SecurityAuditEntry {
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SecurityAuditResponse {
    #[serde(default)]
    pub entries: Vec<SecurityAuditEntry>,
    #[serde(default)]
    pub total: u32,
}

pub async fn fetch_audit_log() -> Result<SecurityAuditResponse, String> {
    fetch_json("/v1/security/audit").await
}

// ── Channel health ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChannelHealthResponse {
    #[serde(default)]
    pub channels: Vec<ChannelHealthEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChannelHealthEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub last_message_at: String,
    #[serde(default)]
    pub error: String,
}

pub async fn fetch_channel_health() -> Result<ChannelHealthResponse, String> {
    fetch_json("/v1/channels/health").await
}

// ── Conversation branching ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ConversationBranch {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub parent_session_id: String,
    #[serde(default)]
    pub branch_point: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub message_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BranchesResponse {
    #[serde(default)]
    pub branches: Vec<ConversationBranch>,
}

pub async fn fetch_branches(session_id: &str) -> Result<BranchesResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/branches", session_id)).await
}

pub async fn create_branch(session_id: &str, turn_index: u32, label: &str) -> Result<MsgResponse, String> {
    post_json(
        &format!("/v1/sessions/{}/branch", session_id),
        &serde_json::json!({"branch_point": turn_index, "label": label}),
    ).await
}

// ── Channel messages ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChannelMessage {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub is_bot: bool,
}



pub async fn fetch_channel_status(channel_id: &str) -> Result<ChannelStatusResponse, String> {
    fetch_json(&format!("/v1/channels/{}/status", channel_id)).await
}

pub async fn connect_channel(channel_id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/connect", channel_id), &serde_json::json!({})).await
}

pub async fn disconnect_channel(channel_id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/disconnect", channel_id), &serde_json::json!({})).await
}

// tts_synthesize delegates to media.rs (with auth headers)
pub async fn tts_synthesize(text: &str) -> Result<Vec<u8>, String> {
    media::tts_synthesize(text).await
}

// ── Onboarding ──

#[derive(Clone, Debug, Deserialize, Default)]
pub struct OnboardingStatus {
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub auth_method: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ProviderModel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tier: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ProviderInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub auth_methods: Vec<String>,
    #[serde(default)]
    pub models: Vec<ProviderModel>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub requires_url: bool,
    #[serde(default)]
    pub default_url: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ProvidersListResponse {
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
}

pub async fn fetch_onboarding_status() -> Result<OnboardingStatus, String> {
    fetch_json("/v1/onboarding/status").await
}

/// Send feature toggles to /v1/onboarding/setup, which persists disabled
/// features into tui.disabled_tools (the consumer the agent loop reads).
/// NOTE: PUT /v1/config silently DROPS a "features" key — ConfigUpdateRequest
/// has no such field — so this is the only path that actually applies toggles.
pub async fn save_feature_toggles(features: &std::collections::HashMap<String, bool>) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({ "features": features });
    post_json("/v1/onboarding/setup", &body).await
}

/// #220: Single consolidated onboarding save — POST /v1/onboarding/setup with
/// the full wizard payload. Replaces the old multi-call sequence
/// (save_feature_toggles + per-provider /v1/credentials + /v1/onboarding/complete
/// + trailing PUT /v1/config), all of which the setup handler persists in one
/// atomic config write: api_keys → [credentials]+vault+env, model, security
/// level → aegis.sandbox_level, features → tui.disabled_tools, name, ollama
/// URL, and (complete=true) onboarding_complete + workspace file generation.
pub async fn onboarding_setup(
    provider: &str,
    model: &str,
    api_keys: &std::collections::HashMap<String, String>,
    security_level: &str,
    features: &std::collections::HashMap<String, bool>,
    name: &str,
    ollama_url: Option<&str>,
    complete: bool,
    use_oauth: bool,
    voice_provider: Option<&str>,
    image_gen_provider: Option<&str>,
    embedding_provider: Option<&str>,
    workspace_path: Option<&str>,
    persona: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut body = serde_json::json!({
        "provider": provider,
        "model": model,
        "api_keys": api_keys,
        "security_level": security_level,
        "features": features,
        "name": name,
        "complete": complete,
        "use_oauth": use_oauth,
    });
    if let Some(u) = ollama_url {
        body["ollama_url"] = serde_json::Value::String(u.to_string());
    }
    if let Some(v) = voice_provider {
        body["voice_provider"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = image_gen_provider {
        body["image_gen_provider"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = embedding_provider {
        body["embedding_provider"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = workspace_path {
        body["workspace_path"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = persona {
        body["persona"] = serde_json::Value::String(v.to_string());
    }
    post_json("/v1/onboarding/setup", &body).await
}

pub async fn complete_onboarding(
    mode: &str,
    provider: &str,
    auth_method: &str,
    model: Option<&str>,
    url: Option<&str>,
) -> Result<MsgResponse, String> {
    let mut body = serde_json::json!({
        "mode": mode,
        "provider": provider,
        "auth_method": auth_method,
    });
    if let Some(m) = model {
        body["model"] = serde_json::Value::String(m.to_string());
    }
    if let Some(u) = url {
        body["url"] = serde_json::Value::String(u.to_string());
    }
    post_json("/v1/onboarding/complete", &body).await
}

pub async fn fetch_providers_list() -> Result<ProvidersListResponse, String> {
    fetch_json("/v1/providers").await
}

#[derive(Deserialize, Clone, Debug)]
pub struct GatewayStartResponse {
    #[serde(default)]
    pub started: bool,
    #[serde(default)]
    pub already_running: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub error: String,
}

/// Request the gateway to (re)start and return its URL.
/// POST /v1/gateway/restart — triggers daemon restart, returns bound URL.
pub async fn start_gateway(port: u16, bind: &str) -> Result<GatewayStartResponse, String> {
    let body = serde_json::json!({
        "port": port,
        "bind": bind,
    });
    post_json("/v1/gateway/restart", &body).await
}

// ── Web search via backend web_fetch tool ──

#[derive(Deserialize)]
struct ToolExecResp {
    #[serde(default)]
    output: String,
    #[serde(default)]
    success: bool,
}

#[derive(Deserialize)]
struct WikiSearchResp {
    #[serde(default)]
    query: WikiQuery,
}
#[derive(Deserialize, Default)]
struct WikiQuery {
    #[serde(default)]
    search: Vec<WikiResult>,
}
#[derive(Deserialize)]
struct WikiResult {
    title: String,
    #[serde(default)]
    snippet: String,
}

#[derive(Clone)]
pub struct SearchResult {
    pub title: String,
    pub snippet: String,
    pub url: String,
}

/// Perform a web search using Wikipedia API + DuckDuckGo instant answer.
/// Returns formatted search results as a string.
pub async fn web_search(query: &str) -> Result<Vec<SearchResult>, String> {
    let encoded = js_sys::encode_uri_component(query);
    let mut results = Vec::new();

    // 1) Wikipedia search
    let wiki_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=5",
        encoded
    );
    let wiki_tool_body = serde_json::json!({
        "arguments": { "url": wiki_url }
    });
    match post_json::<serde_json::Value, ToolExecResp>("/v1/tools/web_fetch", &wiki_tool_body).await {
        Ok(resp) if resp.success => {
            if let Ok(wiki) = serde_json::from_str::<WikiSearchResp>(&resp.output) {
                for r in wiki.query.search {
                    // Strip HTML tags from snippet
                    let snippet = r.snippet
                        .replace("<span class=\"searchmatch\">", "")
                        .replace("</span>", "")
                        .replace("&quot;", "\"")
                        .replace("&amp;", "&");
                    let url = format!(
                        "https://en.wikipedia.org/wiki/{}",
                        r.title.replace(' ', "_")
                    );
                    results.push(SearchResult {
                        title: r.title,
                        snippet,
                        url,
                    });
                }
            }
        }
        _ => {}
    }

    // 2) DuckDuckGo instant answer API (no CAPTCHA, returns abstracts)
    let ddg_url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_redirect=1&no_html=1",
        encoded
    );
    let ddg_tool_body = serde_json::json!({
        "arguments": { "url": ddg_url }
    });
    match post_json::<serde_json::Value, ToolExecResp>("/v1/tools/web_fetch", &ddg_tool_body).await {
        Ok(resp) if resp.success => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&resp.output) {
                // Abstract
                if let Some(abs) = val.get("AbstractText").and_then(|v| v.as_str())
                    && !abs.is_empty() {
                        let url = val.get("AbstractURL").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let src = val.get("AbstractSource").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        results.insert(0, SearchResult {
                            title: format!("{} ({})", val.get("Heading").and_then(|v| v.as_str()).unwrap_or(""), src),
                            snippet: abs.to_string(),
                            url,
                        });
                    }
                // Related topics
                if let Some(topics) = val.get("RelatedTopics").and_then(|v| v.as_array()) {
                    for topic in topics.iter().take(3) {
                        if let (Some(text), Some(url)) = (
                            topic.get("Text").and_then(|v| v.as_str()),
                            topic.get("FirstURL").and_then(|v| v.as_str()),
                        ) {
                            results.push(SearchResult {
                                title: text.split(" - ").next().unwrap_or(text).to_string(),
                                snippet: text.to_string(),
                                url: url.to_string(),
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(results)
}

// ── Image generation via Fooocus API ──

#[derive(Serialize)]
struct FooocusRequest {
    prompt: String,
    negative_prompt: String,
    style_selections: Vec<String>,
    performance_selection: String,
    aspect_ratios_selection: String,
    image_number: u32,
    output_format: String,
    image_seed: i64,
}

/// Generate an image using Fooocus API (proxied through nginx at /imggen/).
/// Returns base64-encoded PNG on success.
pub async fn generate_image(prompt: &str, style: Option<&str>, size: Option<&str>) -> Result<String, String> {
    let style_selection = style.unwrap_or("Fooocus V2").to_string();
    let aspect = size.unwrap_or("1024\u{00d7}1024").to_string();

    let body = FooocusRequest {
        prompt: prompt.to_string(),
        negative_prompt: String::new(),
        style_selections: vec![style_selection],
        performance_selection: "Speed".to_string(),
        aspect_ratios_selection: aspect,
        image_number: 1,
        output_format: "png".to_string(),
        image_seed: -1,
    };

    let resp = gloo_net::http::Request::post("/imggen/v1/generation/text-to-image-with-ip")
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).map_err(|e| format!("serialize: {}", e))?)
        .map_err(|e| format!("request: {}", e))?
        .send()
        .await
        .map_err(|e| {
            format!("Image generation server (Fooocus) is not currently running or unreachable: {}", e)
        })?;

    if !resp.ok() {
        let status = resp.status();
        if status == 502 || status == 503 {
            return Err("Image generation server (Fooocus) is not currently running. Start it and try again.".to_string());
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Image generation failed (HTTP {}): {}", status, body));
    }

    let text = resp.text().await.map_err(|e| format!("read response: {}", e))?;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| format!("parse response: {}", e))?;

    if let Some(first) = arr.first() {
        if let Some(b64) = first.get("base64").and_then(|v| v.as_str()) {
            return Ok(b64.to_string());
        }
        if let Some(url) = first.get("url").and_then(|v| v.as_str()) {
            return Ok(url.to_string());
        }
    }

    Err("No image data in response".to_string())
}

// ── Peer Review ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReviewListResponse {
    #[serde(default)]
    pub reviews: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReviewDetailResponse {
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub entries: Vec<serde_json::Value>,
    #[serde(default)]
    pub reviews: Vec<ReviewItem>,
    #[serde(default)]
    pub review_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReviewItem {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub reviewer_id: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub dimensions: std::collections::HashMap<String, f64>,
    #[serde(default)]
    pub comments: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SubmitReviewResponse {
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub reviewers_assigned: Vec<String>,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReviewVerdictResponse {
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub consensus: Option<serde_json::Value>,
}

pub async fn list_reviews(limit: usize) -> Result<ReviewListResponse, String> {
    fetch_json(&format!("/v1/reviews?limit={}", limit)).await
}

pub async fn get_review(id: &str) -> Result<ReviewDetailResponse, String> {
    fetch_json(&format!("/v1/reviews/{}", id)).await
}

pub async fn submit_review(task_id: &str, agent_id: &str, output: &str) -> Result<SubmitReviewResponse, String> {
    post_json("/v1/reviews", &serde_json::json!({
        "task_id": task_id,
        "agent_id": agent_id,
        "output": output,
    })).await
}

pub async fn approve_review(id: &str, reviewer_id: &str, score: Option<f64>, comments: Option<&str>) -> Result<ReviewVerdictResponse, String> {
    let mut body = serde_json::json!({ "reviewer_id": reviewer_id });
    if let Some(s) = score { body["score"] = serde_json::json!(s); }
    if let Some(c) = comments { body["comments"] = serde_json::Value::String(c.to_string()); }
    post_json(&format!("/v1/reviews/{}/approve", id), &body).await
}

pub async fn reject_review(id: &str, reviewer_id: &str, score: Option<f64>, comments: Option<&str>) -> Result<ReviewVerdictResponse, String> {
    let mut body = serde_json::json!({ "reviewer_id": reviewer_id });
    if let Some(s) = score { body["score"] = serde_json::json!(s); }
    if let Some(c) = comments { body["comments"] = serde_json::Value::String(c.to_string()); }
    post_json(&format!("/v1/reviews/{}/reject", id), &body).await
}

// ── Prometheus plan/execute ──

#[derive(Clone, Debug, Deserialize, Default)]
pub struct PrometheusPlanResponse {
    #[serde(default)]
    pub plan_id: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub nodes: usize,
    #[serde(default)]
    pub parallel_groups: Vec<Vec<usize>>,
    #[serde(default)]
    pub critical_path: Vec<usize>,
    #[serde(default)]
    pub estimated_total_ms: u64,
    #[serde(default)]
    pub topological_order: Vec<usize>,
    #[serde(default)]
    pub dag: serde_json::Value,
}

pub async fn prometheus_plan(goal: &str, steps: Option<&serde_json::Value>) -> Result<PrometheusPlanResponse, String> {
    let mut body = serde_json::json!({ "goal": goal });
    if let Some(s) = steps {
        body["steps"] = s.clone();
    }
    post_json("/v1/prometheus/plan", &body).await
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct PrometheusExecuteResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub total_steps: usize,
    #[serde(default)]
    pub parallel_groups: usize,
    #[serde(default)]
    pub next_ready: Vec<usize>,
    #[serde(default)]
    pub estimated_total_ms: u64,
    #[serde(default)]
    pub is_finished: bool,
    #[serde(default)]
    pub dag: serde_json::Value,
}

pub async fn prometheus_execute(goal: &str, steps: Option<&serde_json::Value>) -> Result<PrometheusExecuteResponse, String> {
    let mut body = serde_json::json!({ "goal": goal });
    if let Some(s) = steps {
        body["steps"] = s.clone();
    }
    post_json("/v1/prometheus/execute", &body).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PrometheusAgentInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "agent_id")]
    pub agent_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub current_task: String,
    #[serde(default)]
    pub iterations: u64,
    #[serde(default)]
    pub uptime_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PrometheusStateResponse {
    #[serde(default)]
    pub total_agents: usize,
    #[serde(default)]
    pub idle_agents: usize,
    #[serde(default)]
    pub agents: Vec<PrometheusAgentInfo>,
}

pub async fn fetch_prometheus_state() -> Result<PrometheusStateResponse, String> {
    fetch_json("/v1/prometheus/state").await
}

// ── Economy / Wallets ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Wallet {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub balance: f64,
    #[serde(default)]
    pub total_earned: f64,
    #[serde(default)]
    pub total_spent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WalletsResponse {
    #[serde(default)]
    pub wallets: Vec<Wallet>,
    #[serde(default)]
    pub total_supply: f64,
    #[serde(default)]
    pub circulating: f64,
}

pub async fn fetch_wallets() -> Result<WalletsResponse, String> {
    fetch_json("/v1/economy/wallets").await
}

pub async fn fetch_wallet(agent_id: &str) -> Result<Wallet, String> {
    fetch_json(&format!("/v1/economy/wallets/{}", agent_id)).await
}

pub async fn fetch_transactions(limit: Option<usize>) -> Result<Vec<Transaction>, String> {
    let url = if let Some(lim) = limit {
        format!("/v1/economy/transactions?limit={}", lim)
    } else {
        "/v1/economy/transactions".to_string()
    };
    fetch_json(&url).await
}

pub async fn fetch_stakes(agent_id: Option<&str>) -> Result<Vec<Stake>, String> {
    let url = if let Some(aid) = agent_id {
        format!("/v1/economy/stakes?agent_id={}", aid)
    } else {
        "/v1/economy/stakes".to_string()
    };
    #[derive(Deserialize)]
    struct StakesResponse { stakes: Vec<Stake> }
    let resp: StakesResponse = fetch_json(&url).await?;
    Ok(resp.stakes)
}

// ── Marketplace / Agora ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceListing {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, alias = "publisher_id")]
    pub author_agent_id: String,
    #[serde(default, alias = "price")]
    pub price_tokens: u64,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata_json: String,
}

impl MarketplaceListing {
    /// Trust level: 2=trusted (builtin), 1=basic (local), 0=restricted (clawhub), -1=unknown
    pub fn trust_level(&self) -> i8 {
        // Try metadata_json first
        if !self.metadata_json.is_empty()
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&self.metadata_json)
                && let Some(tl) = v.get("trust_level").and_then(|t| t.as_i64()) {
                    return tl as i8;
                }
        // Infer from source
        match self.source.as_str() {
            "builtin" => 2,
            "local" => 1,
            "clawhub" => 0,
            _ => -1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceListingsResponse {
    #[serde(default)]
    pub listings: Vec<MarketplaceListing>,
    #[serde(default)]
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PublishListingRequest {
    pub name: String,
    pub description: String,
    pub publisher_id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub price: u64,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PublishListingResponse {
    #[serde(default)]
    pub skill_id: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TradeRequest {
    pub buyer_id: String,
    pub skill_id: String,
    pub offered_price: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TradeResponse {
    #[serde(default)]
    pub trade_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub buyer_id: String,
    #[serde(default)]
    pub seller_id: String,
    #[serde(default)]
    pub skill_id: String,
    #[serde(default)]
    pub price: u64,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Transaction {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub amount: u64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Stake {
    #[serde(default)]
    pub stake_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub amount: u64,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub released_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceStats {
    #[serde(default)]
    pub total_listings: u64,
    #[serde(default)]
    pub total_trades: u64,
    #[serde(default)]
    pub active_listings: u64,
    #[serde(default)]
    pub total_downloads: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceCategoriesResponse {
    #[serde(default)]
    pub categories: Vec<CategoryCount>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CategoryCount {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillRating {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkillRatingsResponse {
    #[serde(default)]
    pub skill_id: String,
    #[serde(default)]
    pub ratings: Vec<SkillRating>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LedgerResponse {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub balance: u64,
    #[serde(default)]
    pub transactions: Vec<Transaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReputationResponse {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub total_trades: u64,
    #[serde(default)]
    pub successful_trades: u64,
    #[serde(default)]
    pub ratings: f64,
    #[serde(default)]
    pub trade_success_rate: f64,
}

/// GET /v1/marketplace/listings - Fetch marketplace skill listings
pub async fn fetch_marketplace_listings(
    capability: Option<&str>,
    tag: Option<&str>,
    query: Option<&str>,
    publisher: Option<&str>,
) -> Result<MarketplaceListingsResponse, String> {
    let mut url = String::from("/v1/marketplace/listings");
    let mut params = Vec::new();

    if let Some(cap) = capability {
        params.push(format!("capability={}", cap));
    }
    if let Some(t) = tag {
        params.push(format!("tag={}", t));
    }
    if let Some(q) = query {
        params.push(format!("q={}", q));
    }
    if let Some(p) = publisher {
        params.push(format!("publisher={}", p));
    }

    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }

    fetch_json(&url).await
}

/// POST /v1/marketplace/listings - Publish a new skill listing
pub async fn publish_marketplace_listing(
    request: &PublishListingRequest,
) -> Result<PublishListingResponse, String> {
    post_json("/v1/marketplace/listings", request).await
}

/// POST /v1/marketplace/trade - Initiate a trade
pub async fn marketplace_trade(request: &TradeRequest) -> Result<TradeResponse, String> {
    post_json("/v1/marketplace/trade", request).await
}

/// GET /v1/marketplace/featured - Get featured listings (server-side)
pub async fn fetch_marketplace_featured(limit: usize) -> Result<MarketplaceListingsResponse, String> {
    fetch_json(&format!("/v1/marketplace/featured?limit={}", limit)).await
}

/// GET /v1/marketplace/stats - Marketplace overview
pub async fn fetch_marketplace_stats() -> Result<MarketplaceStats, String> {
    fetch_json("/v1/marketplace/stats").await
}

/// GET /v1/marketplace/categories - Category counts
pub async fn fetch_marketplace_categories() -> Result<MarketplaceCategoriesResponse, String> {
    fetch_json("/v1/marketplace/categories").await
}

/// GET /v1/marketplace/ratings/:skill_id - Get ratings for a skill
pub async fn fetch_skill_ratings(skill_id: &str) -> Result<SkillRatingsResponse, String> {
    fetch_json(&format!("/v1/marketplace/ratings/{}", skill_id)).await
}

/// POST /v1/marketplace/ratings/:skill_id - Submit a rating
pub async fn submit_skill_rating(skill_id: &str, agent_id: &str, score: f64, comment: Option<&str>) -> Result<MsgResponse, String> {
    let mut body = serde_json::json!({ "agent_id": agent_id, "score": score });
    if let Some(c) = comment { body["comment"] = serde_json::json!(c); }
    post_json(&format!("/v1/marketplace/ratings/{}", skill_id), &body).await
}

/// GET /v1/marketplace/ledger/:agent_id - Get token balance and transaction history
pub async fn fetch_marketplace_ledger(agent_id: &str) -> Result<LedgerResponse, String> {
    fetch_json(&format!("/v1/marketplace/ledger/{}", agent_id)).await
}

/// GET /v1/marketplace/reputation/:agent_id - Get reputation score
pub async fn fetch_marketplace_reputation(agent_id: &str) -> Result<ReputationResponse, String> {
    fetch_json(&format!("/v1/marketplace/reputation/{}", agent_id)).await
}

// ── Anthropic OAuth ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AnthropicOAuthStatus {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub provider: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AnthropicOAuthLoginResponse {
    #[serde(default)]
    pub authorize_url: String,
    #[serde(default)]
    pub status: String,
}

pub async fn fetch_anthropic_oauth_status() -> Result<AnthropicOAuthStatus, String> {
    fetch_json("/v1/auth/anthropic/status").await
}

/// #216b: matches the actual GET /v1/auth/anthropic/status response shape
/// (authenticated/method/expires_at) — `AnthropicOAuthStatus` above predates
/// the handler and carries stale fields.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AnthropicAuthStatus {
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

pub async fn fetch_anthropic_auth_status() -> Result<AnthropicAuthStatus, String> {
    fetch_json("/v1/auth/anthropic/status").await
}



// ── File Uploads ──

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadedFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, rename = "type")]
    pub mime_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub extracted_text: Option<String>,
    #[serde(default)]
    pub extension: String,
    #[serde(default)]
    pub uploaded_at: String,
}

pub async fn upload_file(file: web_sys::File, _on_progress: impl Fn(f64) + 'static) -> Result<UploadedFile, String> {
    // Use raw web_sys fetch (not gloo_net) so the browser auto-sets the
    // correct multipart/form-data Content-Type with boundary — gloo_net
    // can interfere with this header, causing the server to reject the upload.
    let form_data = web_sys::FormData::new().map_err(|e| format!("FormData error: {:?}", e))?;
    form_data
        .append_with_blob("file", &file)
        .map_err(|e| format!("Append blob error: {:?}", e))?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form_data);

    let request = web_sys::Request::new_with_str_and_init("/v1/uploads", &opts)
        .map_err(|e| format!("Request: {:?}", e))?;

    // Add auth header if configured
    if let Some(auth) = auth_bearer() {
        request.headers()
            .set("Authorization", &auth)
            .map_err(|e| format!("Header: {:?}", e))?;
    }

    let window = web_sys::window().ok_or("No window")?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch: {:?}", e))?;

    let resp: web_sys::Response = resp_val.dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;

    if !resp.ok() {
        let status = resp.status();
        let body_promise = resp.text().map_err(|e| format!("Text: {:?}", e))?;
        let body = wasm_bindgen_futures::JsFuture::from(body_promise)
            .await
            .map_err(|e| format!("Text read: {:?}", e))?
            .as_string()
            .unwrap_or_default();
        return Err(if body.is_empty() {
            format!("HTTP {}", status)
        } else {
            format!("HTTP {}: {}", status, body)
        });
    }

    let text_promise = resp.text().map_err(|e| format!("Text: {:?}", e))?;
    let text = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("Text read: {:?}", e))?
        .as_string()
        .unwrap_or_default();

    serde_json::from_str::<UploadedFile>(&text)
        .map_err(|e| format!("Parse error: {} (body: {})", e, &text[..text.len().min(200)]))
}

pub async fn list_uploads() -> Result<Vec<UploadedFile>, String> {
    fetch_json("/v1/uploads").await
}

pub async fn get_upload_metadata(id: &str) -> Result<UploadedFile, String> {
    fetch_json(&format!("/v1/uploads/{}", id)).await
}

pub async fn delete_upload(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/uploads/{}", id)).await
}

// ── Goals ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GoalResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub deadline: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GoalsResponse {
    #[serde(default)]
    pub goals: Vec<GoalResponse>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GoalAnalysisResponse {
    #[serde(default)]
    pub recommended_workflow: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub autonomy_level: String,
    #[serde(default)]
    pub estimated_cost: f64,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub complexity: String,
    #[serde(default)]
    pub analysis_method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CreateGoalRequest {
    pub description: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub source: String,
}

pub async fn fetch_goals() -> Result<GoalsResponse, String> {
    fetch_json("/v1/goals").await
}

pub async fn get_goal(id: &str) -> Result<GoalResponse, String> {
    fetch_json(&format!("/v1/goals/{}", id)).await
}

pub async fn create_goal(req: &CreateGoalRequest) -> Result<GoalResponse, String> {
    post_json("/v1/goals", req).await
}

pub async fn analyze_goal(goal: &str) -> Result<GoalAnalysisResponse, String> {
    let body = serde_json::json!({ "goal": goal });
    post_json("/v1/goals/analyze", &body).await
}

/// Analyze a goal with provider context (used during onboarding when config isn't saved yet).
pub async fn analyze_goal_with_provider(
    goal: &str,
    provider: &str,
    model: &str,
    api_key: &str,
    url: &str,
) -> Result<GoalAnalysisResponse, String> {
    let mut body = serde_json::json!({ "goal": goal, "provider": provider, "model": model });
    if !api_key.is_empty() {
        body["api_key"] = serde_json::json!(api_key);
    }
    if !url.is_empty() {
        body["url"] = serde_json::json!(url);
    }
    post_json("/v1/goals/analyze", &body).await
}

pub async fn update_goal_status(id: &str, status: &str) -> Result<GoalResponse, String> {
    let body = serde_json::json!({ "status": status });
    put_json(&format!("/v1/goals/{}/status", id), &body).await
}


// ══════════════════════════════════════════════════════════════
// OBSERVATORY — /v1/observatory/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryActiveTasks {
    #[serde(default)]
    pub cron_tasks: Vec<ObservatoryCronTask>,
    #[serde(default)]
    pub workflows: Vec<ObservatoryWorkflow>,
    #[serde(default)]
    pub pending_approvals: Vec<ObservatoryApproval>,
    #[serde(default)]
    pub summary: ObservatoryTaskSummary,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryCronTask {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cron_expr: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<String>,
    #[serde(default)]
    pub next_run: Option<String>,
    #[serde(default, rename = "type")]
    pub task_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryWorkflow {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub progress_pct: f64,
    #[serde(default)]
    pub total_nodes: u32,
    #[serde(default)]
    pub completed_nodes: u32,
    #[serde(default)]
    pub failed_nodes: u32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryApproval {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub requested_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryTaskSummary {
    #[serde(default)]
    pub cron_total: u32,
    #[serde(default)]
    pub cron_enabled: u32,
    #[serde(default)]
    pub workflows_active: u32,
    #[serde(default)]
    pub workflows_total: u32,
    #[serde(default)]
    pub approvals_pending: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryAgentStats {
    #[serde(default)]
    pub agents: Vec<ObservatoryAgent>,
    #[serde(default)]
    pub summary: ObservatoryAgentSummary,
    #[serde(default)]
    pub teams: Vec<ObservatoryTeamInfo>,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryAgent {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub spawned_at: String,
    #[serde(default)]
    pub last_active: String,
    #[serde(default)]
    pub message_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryAgentSummary {
    #[serde(default)]
    pub total_agents: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryTeamInfo {
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub supervisor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryChannelHealth {
    #[serde(default)]
    pub channels: Vec<ObservatoryChannel>,
    #[serde(default)]
    pub summary: ObservatoryChannelSummary,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryChannel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_message_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub uptime_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryChannelSummary {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub connected: u32,
    #[serde(default)]
    pub disconnected: u32,
    #[serde(default)]
    pub overall_healthy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryCostLive {
    #[serde(default)]
    pub cost_summary: ObservatoryCostSummary,
    #[serde(default)]
    pub economy: ObservatoryEconomy,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryCostSummary {
    #[serde(default)]
    pub total_cost: f64,
    #[serde(default)]
    pub budget_remaining: f64,
    #[serde(default)]
    pub period_start: String,
    #[serde(default)]
    pub top_models: Vec<ObservatoryModelCost>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryModelCost {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObservatoryEconomy {
    #[serde(default)]
    pub token_balance: u64,
}

pub async fn fetch_observatory_active_tasks() -> Result<ObservatoryActiveTasks, String> {
    fetch_json("/v1/observatory/active-tasks").await
}

pub async fn fetch_observatory_agent_stats() -> Result<ObservatoryAgentStats, String> {
    fetch_json("/v1/observatory/agent-stats").await
}

pub async fn fetch_observatory_channel_health() -> Result<ObservatoryChannelHealth, String> {
    fetch_json("/v1/observatory/channel-health").await
}

pub async fn fetch_observatory_cost_live() -> Result<ObservatoryCostLive, String> {
    fetch_json("/v1/observatory/cost-live").await
}

// ══════════════════════════════════════════════════════════════
// CRON — /v1/cron/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CronJobsResponse {
    #[serde(default)]
    pub jobs: Vec<serde_json::Value>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CronTemplatesResponse {
    #[serde(default)]
    pub templates: Vec<CronTemplate>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CronTemplate {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cron: String,
    #[serde(default)]
    pub task_type: serde_json::Value,
}

pub async fn fetch_cron_jobs() -> Result<CronJobsResponse, String> {
    fetch_json("/v1/cron/jobs").await
}

pub async fn create_cron_job(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/cron/jobs", body).await
}

pub async fn delete_cron_job(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/cron/jobs/{}", id)).await
}

pub async fn fetch_cron_job_history(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/cron/jobs/{}/history", id)).await
}

pub async fn fetch_cron_templates() -> Result<CronTemplatesResponse, String> {
    fetch_json("/v1/cron/templates").await
}

// ══════════════════════════════════════════════════════════════
// WEBHOOKS — /v1/webhooks/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WebhookHealthResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub accepts: Vec<String>,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub signature_verification: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WebhookTriggersResponse {
    #[serde(default)]
    pub triggers: Vec<serde_json::Value>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OutboundWebhooksResponse {
    #[serde(default)]
    pub webhooks: Vec<OutboundWebhook>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OutboundWebhook {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_triggered_at: Option<String>,
    #[serde(default)]
    pub failure_count: u32,
}

pub async fn fetch_webhook_health() -> Result<WebhookHealthResponse, String> {
    fetch_json("/v1/webhooks").await
}

pub async fn fetch_webhook_triggers() -> Result<WebhookTriggersResponse, String> {
    fetch_json("/v1/webhooks/triggers").await
}

pub async fn create_webhook_trigger(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/webhooks/triggers", body).await
}

pub async fn delete_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/webhooks/triggers/{}", id)).await
}

pub async fn enable_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/webhooks/triggers/{}/enable", id), &serde_json::json!({})).await
}

pub async fn disable_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/webhooks/triggers/{}/disable", id), &serde_json::json!({})).await
}

pub async fn fetch_outbound_webhooks() -> Result<OutboundWebhooksResponse, String> {
    fetch_json("/v1/webhooks/outbound").await
}

pub async fn register_outbound_webhook(body: &serde_json::Value) -> Result<OutboundWebhook, String> {
    post_json("/v1/webhooks/outbound", body).await
}

pub async fn delete_outbound_webhook(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/webhooks/outbound/{}", id)).await
}

// ══════════════════════════════════════════════════════════════
// WORKFLOWS — /v1/workflows/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowsListResponse {
    #[serde(default)]
    pub workflows: Vec<WorkflowSummary>,
    #[serde(default)]
    pub total: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowSummary {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub progress_percentage: f64,
    #[serde(default)]
    pub total_nodes: u32,
    #[serde(default)]
    pub completed_nodes: u32,
    #[serde(default)]
    pub failed_nodes: u32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowDetail {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub progress_percentage: f64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub total_nodes: u32,
    #[serde(default)]
    pub completed_nodes: u32,
    #[serde(default)]
    pub failed_nodes: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowNode {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowCreateResponse {
    #[serde(default)]
    pub workflow_id: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub steps: Vec<serde_json::Value>,
    #[serde(default)]
    pub total_steps: u32,
    #[serde(default)]
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkflowArtifacts {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub artifacts: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

pub async fn fetch_workflows() -> Result<WorkflowsListResponse, String> {
    fetch_json("/v1/workflows").await
}

pub async fn create_workflow(body: &serde_json::Value) -> Result<WorkflowCreateResponse, String> {
    post_json("/v1/workflows", body).await
}

pub async fn fetch_workflow(id: &str) -> Result<WorkflowDetail, String> {
    fetch_json(&format!("/v1/workflows/{}", id)).await
}

pub async fn fetch_workflow_artifacts(id: &str) -> Result<WorkflowArtifacts, String> {
    fetch_json(&format!("/v1/workflows/{}/artifacts", id)).await
}

// ══════════════════════════════════════════════════════════════
// VECTOR STORES — /v1/vector_stores/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorStoresListResponse {
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub data: Vec<VectorStore>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorStore {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub file_counts: VectorStoreFileCounts,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorStoreFileCounts {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub completed: u32,
    #[serde(default)]
    pub in_progress: u32,
    #[serde(default)]
    pub failed: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorSearchResponse {
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub data: Vec<VectorSearchResult>,
    #[serde(default)]
    pub search_query: String,
    #[serde(default)]
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorSearchResult {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub memory_type: String,
    #[serde(default)]
    pub importance: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorStoreFileResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub vector_store_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VectorStoreFilesListResponse {
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub data: Vec<serde_json::Value>,
    #[serde(default)]
    pub vector_store_id: String,
    #[serde(default)]
    pub file_count: u32,
}

pub async fn fetch_vector_stores() -> Result<VectorStoresListResponse, String> {
    fetch_json("/v1/vector_stores").await
}

pub async fn create_vector_store(body: &serde_json::Value) -> Result<VectorStore, String> {
    post_json("/v1/vector_stores", body).await
}

pub async fn fetch_vector_store(id: &str) -> Result<VectorStore, String> {
    fetch_json(&format!("/v1/vector_stores/{}", id)).await
}

pub async fn delete_vector_store(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/vector_stores/{}", id)).await
}

pub async fn search_vector_store(id: &str, body: &serde_json::Value) -> Result<VectorSearchResponse, String> {
    post_json(&format!("/v1/vector_stores/{}/search", id), body).await
}

pub async fn add_file_to_vector_store(id: &str, body: &serde_json::Value) -> Result<VectorStoreFileResponse, String> {
    post_json(&format!("/v1/vector_stores/{}/files", id), body).await
}

pub async fn fetch_vector_store_files(id: &str) -> Result<VectorStoreFilesListResponse, String> {
    fetch_json(&format!("/v1/vector_stores/{}/files", id)).await
}

// ══════════════════════════════════════════════════════════════
// CANVAS — /v1/canvas/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CanvasComponentsResponse {
    #[serde(default)]
    pub component_types: Vec<String>,
    #[serde(default)]
    pub action_prefixes: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CanvasRenderResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub components: Vec<CanvasComponent>,
    #[serde(default)]
    pub action: serde_json::Value,
    #[serde(default)]
    pub layout: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CanvasComponent {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub component_type: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub props: serde_json::Value,
    #[serde(default)]
    pub children: Vec<CanvasComponent>,
}

pub async fn fetch_canvas_components() -> Result<CanvasComponentsResponse, String> {
    fetch_json("/v1/canvas/components").await
}

pub async fn render_canvas(body: &serde_json::Value) -> Result<CanvasRenderResponse, String> {
    post_json("/v1/canvas/render", body).await
}

// ══════════════════════════════════════════════════════════════
// BATCHES — /v1/batches/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BatchResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub completed_at: Option<u64>,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub completed: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BatchResultsResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub results: Vec<BatchResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BatchResult {
    #[serde(default)]
    pub custom_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub response: Option<BatchResultContent>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BatchResultContent {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub finish_reason: String,
}

pub async fn create_batch(body: &serde_json::Value) -> Result<BatchResponse, String> {
    post_json("/v1/batches", body).await
}

pub async fn fetch_batch(id: &str) -> Result<BatchResponse, String> {
    fetch_json(&format!("/v1/batches/{}", id)).await
}

pub async fn fetch_batch_results(id: &str) -> Result<BatchResultsResponse, String> {
    fetch_json(&format!("/v1/batches/{}/results", id)).await
}

// ══════════════════════════════════════════════════════════════
// ECONOMY & MARKETPLACE — /v1/economy/*, /v1/marketplace/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct EconomyWalletsResponse {
    #[serde(default)]
    pub wallets: Vec<serde_json::Value>,
    #[serde(default)]
    pub total_supply: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct EconomyWalletDetail {
    #[serde(default)]
    pub wallet: serde_json::Value,
    #[serde(default)]
    pub transactions: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceLedger {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub balance: i64,
    #[serde(default)]
    pub transactions: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceReputation {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub total_trades: u32,
    #[serde(default)]
    pub successful_trades: u32,
    #[serde(default)]
    pub ratings: f64,
    #[serde(default)]
    pub trade_success_rate: f64,
}

pub async fn fetch_economy_wallets() -> Result<EconomyWalletsResponse, String> {
    fetch_json("/v1/economy/wallets").await
}

pub async fn fetch_economy_wallet(agent_id: &str) -> Result<EconomyWalletDetail, String> {
    fetch_json(&format!("/v1/economy/wallets/{}", agent_id)).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Bounty {
    #[serde(default)] pub id: String,
    #[serde(default)] pub title: String,
    #[serde(default)] pub description: String,
    #[serde(default)] pub poster_id: String,
    #[serde(default)] pub poster_name: String,
    #[serde(default)] pub reward_credits: u64,
    #[serde(default)] pub status: String,   // open, claimed, submitted, completed, cancelled
    #[serde(default)] pub tags_json: String,
    #[serde(default)] pub claimant_id: Option<String>,
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub deadline: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BountiesResponse {
    #[serde(default)] pub bounties: Vec<Bounty>,
    #[serde(default)] pub total: u64,
}

pub async fn fetch_bounties(status: Option<&str>) -> Result<BountiesResponse, String> {
    let q = status.map(|s| format!("?status={}", s)).unwrap_or_default();
    fetch_json(&format!("/v1/marketplace/bounties{}", q)).await
}

pub async fn claim_bounty(bounty_id: &str, agent_id: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/marketplace/bounties/{}/claim", bounty_id),
        &serde_json::json!({ "agent_id": agent_id })).await
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PantheonEconomyResponse {
    #[serde(default)] pub stats: serde_json::Value,
    #[serde(default)] pub agents: Vec<serde_json::Value>,
}

pub async fn fetch_pantheon_economy() -> Result<PantheonEconomyResponse, String> {
    fetch_json("/v1/pantheon/economy").await
}

// ══════════════════════════════════════════════════════════════
// TEAMS — /v1/teams/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TeamRecommendation {
    #[serde(default)]
    pub team_name: String,
    #[serde(default)]
    pub coordinators: Vec<serde_json::Value>,
    #[serde(default)]
    pub workers: Vec<serde_json::Value>,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub estimated_complexity: String,
    #[serde(default)]
    pub estimated_steps: u32,
}

pub async fn fetch_team(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/teams/{}", id)).await
}

pub async fn update_team(id: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    put_json(&format!("/v1/teams/{}", id), body).await
}

pub async fn delete_team(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/teams/{}", id)).await
}

pub async fn recommend_team(goal: &str) -> Result<TeamRecommendation, String> {
    post_json("/v1/teams/recommend", &serde_json::json!({ "goal": goal })).await
}

// ══════════════════════════════════════════════════════════════
// AGENTS — /v1/agents/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AgentStatusResponse {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub last_active: String,
    #[serde(default)]
    pub message_count: u32,
}

pub async fn fetch_agent_status(id: &str) -> Result<AgentStatusResponse, String> {
    fetch_json(&format!("/v1/agents/{}/status", id)).await
}

pub async fn agent_chat(id: &str, message: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/agents/{}/chat", id), &serde_json::json!({ "message": message })).await
}

pub async fn agent_send(id: &str, message: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/agents/{}/send", id), &serde_json::json!({ "message": message })).await
}

// ══════════════════════════════════════════════════════════════
// SESSIONS — /v1/sessions/* (enhanced)
// ══════════════════════════════════════════════════════════════

pub async fn search_sessions(query: &str) -> Result<SessionsResponse, String> {
    fetch_json(&format!("/v1/sessions/search?q={}", query)).await
}

pub async fn fetch_session_replay_turn(id: &str, turn: u32) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/sessions/{}/replay/{}", id, turn)).await
}

// ══════════════════════════════════════════════════════════════
// MEMORY — /v1/memory/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryTimelineResponse {
    #[serde(default)]
    pub entries: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryCommunitiesResponse {
    #[serde(default)]
    pub communities: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryGraphResponse {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub connections: Vec<serde_json::Value>,
}

pub async fn fetch_memory_timeline() -> Result<MemoryTimelineResponse, String> {
    fetch_json("/v1/memory/timeline").await
}

pub async fn fetch_memory_communities() -> Result<MemoryCommunitiesResponse, String> {
    fetch_json("/v1/memory/communities").await
}

pub async fn fetch_memory_graph(entity_id: &str) -> Result<MemoryGraphResponse, String> {
    fetch_json(&format!("/v1/memory/graph/{}", entity_id)).await
}

pub async fn search_memory_graph(query: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/memory/graph/search", &serde_json::json!({ "query": query })).await
}



// ══════════════════════════════════════════════════════════════
// SCHEDULES — /v1/schedules/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ScheduleRunsResponse {
    #[serde(default)]
    pub runs: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

pub async fn pause_schedule(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/schedules/{}/pause", id), &serde_json::json!({})).await
}

pub async fn resume_schedule(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/schedules/{}/resume", id), &serde_json::json!({})).await
}

pub async fn fetch_schedule_runs(id: &str) -> Result<ScheduleRunsResponse, String> {
    fetch_json(&format!("/v1/schedules/{}/runs", id)).await
}

// ══════════════════════════════════════════════════════════════
// SECURITY — /v1/security/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RotationStatusResponse {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_rotation: Option<String>,
    #[serde(default)]
    pub next_rotation: Option<String>,
    #[serde(default)]
    pub rotation_count: u32,
}

pub async fn rotate_api_key() -> Result<MsgResponse, String> {
    post_json("/v1/security/rotate-key", &serde_json::json!({})).await
}

pub async fn fetch_rotation_status() -> Result<RotationStatusResponse, String> {
    fetch_json("/v1/security/rotation-status").await
}

// ══════════════════════════════════════════════════════════════
// CONFIG — /v1/config/* (enhanced)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ConfigHistoryResponse {
    #[serde(default)]
    pub history: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

pub async fn fetch_config_history() -> Result<ConfigHistoryResponse, String> {
    fetch_json("/v1/config/history").await
}

pub async fn reload_config() -> Result<MsgResponse, String> {
    post_json("/v1/config/reload", &serde_json::json!({})).await
}

// ══════════════════════════════════════════════════════════════
// CHANNELS — /v1/channels/* (enhanced)
// ══════════════════════════════════════════════════════════════

pub async fn pair_channel(id: &str, body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/pair", id), body).await
}

pub async fn fetch_channel_pairings(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/channels/{}/pairings", id)).await
}

pub async fn verify_channel(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/verify", id), &serde_json::json!({})).await
}

pub async fn poll_channel(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/channels/{}/poll", id)).await
}

// ══════════════════════════════════════════════════════════════
// EXTENSIONS — /v1/extensions/* (enhanced)
// ══════════════════════════════════════════════════════════════

pub async fn start_extension(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/extensions/{}/start", id), &serde_json::json!({})).await
}

pub async fn stop_extension(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/extensions/{}/stop", id), &serde_json::json!({})).await
}

// ══════════════════════════════════════════════════════════════
// TOOLS — /v1/tools/* (enhanced)
// ══════════════════════════════════════════════════════════════

pub async fn fetch_tool_detail(name: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/tools/{}", name)).await
}

// ══════════════════════════════════════════════════════════════
// TTS — /v1/tts/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TtsProvidersResponse {
    #[serde(default)]
    pub providers: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TtsVoicesResponse {
    #[serde(default)]
    pub voices: Vec<serde_json::Value>,
}

pub async fn fetch_tts_providers() -> Result<TtsProvidersResponse, String> {
    fetch_json("/v1/tts/providers").await
}

pub async fn fetch_tts_voices() -> Result<TtsVoicesResponse, String> {
    fetch_json("/v1/tts/voices").await
}

// ══════════════════════════════════════════════════════════════
// IMAGES — /v1/images/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ImagesListResponse {
    #[serde(default)]
    pub images: Vec<serde_json::Value>,
    #[serde(default)]
    pub total: u32,
}

pub async fn fetch_images() -> Result<ImagesListResponse, String> {
    fetch_json("/v1/images").await
}

pub async fn fetch_image(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/images/{}", id)).await
}



// ══════════════════════════════════════════════════════════════
// ROUTING / COST — /v1/routing/*
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RoutingCostsResponse {
    #[serde(default)]
    pub costs: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RoutingBudgetResponse {
    #[serde(default)]
    pub budget: f64,
    #[serde(default)]
    pub spent: f64,
    #[serde(default)]
    pub remaining: f64,
    #[serde(default)]
    pub period: String,
}

pub async fn fetch_routing_costs() -> Result<RoutingCostsResponse, String> {
    fetch_json("/v1/routing/costs").await
}

pub async fn fetch_routing_budget() -> Result<RoutingBudgetResponse, String> {
    fetch_json("/v1/routing/budget").await
}

pub async fn fetch_routing_recommend(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/routing/recommend", body).await
}

pub async fn fetch_cost_recommend(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/routing/cost-recommend", body).await
}

// ═══════════════════════════════════════════════════════════
// Pantheon — Multi-Agent Missions
// ═══════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonMission {
    #[serde(default)] pub id: String,
    #[serde(default)] pub goal: String,
    #[serde(default)] pub status: String,
    #[serde(default)] pub team: Vec<PantheonTeamMember>,
    #[serde(default)] pub tasks: Vec<PantheonTask>,
    #[serde(default)] pub progress_pct: f64,
    #[serde(default)] pub tasks_done: usize,
    #[serde(default)] pub tasks_total: usize,
    #[serde(default)] pub tokens_used: u64,
    #[serde(default)] pub constraints: PantheonConstraints,
    #[serde(default)] pub feed: Vec<PantheonActivityEntry>,
    #[serde(default)] pub artifacts: Vec<PantheonArtifact>,
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub updated_at: String,
    #[serde(default)] pub completed_at: Option<String>,
    #[serde(default)] pub summary: Option<String>,
    // summary fields from list endpoint
    #[serde(default)] pub team_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonTeamMember {
    #[serde(default)] pub agent_id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub role: String,
    #[serde(default)] pub status: String,
    #[serde(default)] pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonTask {
    #[serde(default)] pub id: String,
    #[serde(default)] pub description: String,
    #[serde(default)] pub assigned_to: Option<String>,
    #[serde(default)] pub status: String,
    #[serde(default)] pub result: Option<String>,
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonConstraints {
    #[serde(default)] pub budget_tokens: Option<u64>,
    #[serde(default)] pub timeout_seconds: Option<u64>,
    #[serde(default)] pub max_agents: Option<usize>,
    #[serde(default)] pub require_review: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonActivityEntry {
    #[serde(default)] pub agent_id: String,
    #[serde(default)] pub agent_name: String,
    #[serde(default)] pub activity: String,
    #[serde(default)] pub detail: serde_json::Value,
    #[serde(default)] pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonArtifact {
    #[serde(default)] pub name: String,
    #[serde(default)] pub path: String,
    #[serde(default, rename = "type")] pub artifact_type: String,
    #[serde(default)] pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionRequest {
    pub goal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<PantheonConstraints>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateMissionResponse {
    #[serde(default)] pub id: String,
    #[serde(default)] pub goal: String,
    #[serde(default)] pub status: String,
    #[serde(default)] pub team: Vec<PantheonTeamMember>,
    #[serde(default)] pub created_at: String,
}

// Pantheon API functions

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionsListResponse {
    pub missions: Vec<PantheonMission>,
    #[serde(default)] pub total: usize,
    #[serde(default)] pub offset: usize,
    #[serde(default)] pub limit: usize,
}

pub async fn fetch_pantheon_missions() -> Result<Vec<PantheonMission>, String> {
    // Try envelope format first (Zeus112 `1083e9c`), fall back to bare array
    if let Ok(resp) = fetch_json::<MissionsListResponse>("/v1/pantheon/missions").await {
        return Ok(resp.missions);
    }
    fetch_json::<Vec<PantheonMission>>("/v1/pantheon/missions").await
}

pub async fn fetch_pantheon_mission(id: &str) -> Result<PantheonMission, String> {
    fetch_json(&format!("/v1/pantheon/missions/{}", id)).await
}

pub async fn create_pantheon_mission(req: &CreateMissionRequest) -> Result<CreateMissionResponse, String> {
    post_json("/v1/pantheon/missions", req).await
}

pub async fn intervene_pantheon_mission(id: &str, action: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/pantheon/missions/{}/intervene", id), &serde_json::json!({"action": action})).await
}

pub async fn dispatch_mission_task(id: &str, task: &str, agent_id: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/pantheon/missions/{}/intervene", id), &serde_json::json!({
        "action": "assign_task",
        "task": task,
        "agent_id": agent_id,
    })).await
}

pub async fn fetch_pantheon_feed(id: &str) -> Result<Vec<PantheonActivityEntry>, String> {
    fetch_json(&format!("/v1/pantheon/missions/{}/feed", id)).await
}

pub async fn fetch_pantheon_artifacts(id: &str) -> Result<Vec<PantheonArtifact>, String> {
    fetch_json(&format!("/v1/pantheon/missions/{}/artifacts", id)).await
}

// Fleet API

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FleetAgent {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub capabilities: Vec<String>,
    #[serde(default)] pub status: String,
    #[serde(default)] pub health_score: f32,
    #[serde(default)] pub load_pct: f64,
    #[serde(default)] pub metadata: std::collections::HashMap<String, String>,
    #[serde(default)] pub last_heartbeat: String,
}

pub async fn fetch_fleet_agents() -> Result<Vec<FleetAgent>, String> {
    fetch_json("/v1/fleet").await
}

// Agent Discovery

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDiscoverResponse {
    #[serde(default)] pub agents: Vec<FleetAgent>,
    #[serde(default)] pub total: usize,
    #[serde(default)] pub fleet_size: usize,
    #[serde(default)] pub capabilities: Vec<String>,
}

pub async fn discover_agents(capability: Option<&str>, status: Option<&str>, q: Option<&str>) -> Result<AgentDiscoverResponse, String> {
    let mut params = vec![];
    if let Some(c) = capability { params.push(format!("capability={}", c)); }
    if let Some(s) = status { params.push(format!("status={}", s)); }
    if let Some(q) = q { params.push(format!("q={}", q)); }
    let url = if params.is_empty() {
        "/v1/agents/discover".to_string()
    } else {
        format!("/v1/agents/discover?{}", params.join("&"))
    };
    fetch_json(&url).await
}

// Pantheon Rooms

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonRoom {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub description: Option<String>,
    #[serde(default)] pub room_type: String,
    #[serde(default)] pub mission_id: Option<String>,
    #[serde(default)] pub created_by: String,
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub member_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonRoomMember {
    #[serde(default)] pub agent_id: String,
    #[serde(default)] pub agent_name: String,
    #[serde(default)] pub role: String,
    #[serde(default)] pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonRoomMessage {
    #[serde(default)] pub id: String,
    #[serde(default)] pub room_id: String,
    #[serde(default)] pub sender_id: String,
    #[serde(default)] pub sender_name: String,
    #[serde(default)] pub content: String,
    #[serde(default)] pub message_type: String,
    #[serde(default)] pub metadata: Option<serde_json::Value>,
    #[serde(default)] pub timestamp: String,
    #[serde(default)] pub reply_to: Option<String>,
    #[serde(default)] pub edited: bool,
    #[serde(default)] pub attachments: Vec<MessageAttachment>,
}

/// File attachment on a war room message
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageAttachment {
    #[serde(default)] pub filename: String,
    #[serde(default)] pub url: String,
    #[serde(default)] pub content_type: String,
    #[serde(default)] pub size: u64,
}

/// Plan card step for approval flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    #[serde(default)] pub description: String,
    #[serde(default)] pub agent_type: String,
    #[serde(default)] pub status: String, // "pending" | "running" | "done" | "failed"
    #[serde(default)] pub elapsed_ms: Option<u64>,
}

/// Plan card metadata (embedded in message metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCardMeta {
    #[serde(default)] pub plan_id: String,
    #[serde(default)] pub goal: String,
    #[serde(default)] pub steps: Vec<PlanStep>,
    #[serde(default)] pub status: String, // "awaiting_approval" | "approved" | "executing" | "done" | "rejected"
    #[serde(default)] pub revision: u32,
}

/// Approve or reject a plan
pub async fn approve_plan(plan_id: &str, approver_id: &str, approver_name: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/pantheon/plans/{}/approve", plan_id), &serde_json::json!({"approver_id": approver_id, "approver_name": approver_name})).await
}

pub async fn reject_plan(plan_id: &str, reason: &str, approver_id: &str, approver_name: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/pantheon/plans/{}/reject", plan_id), &serde_json::json!({"reason": reason, "approver_id": approver_id, "approver_name": approver_name})).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRoomMessageRequest {
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

pub async fn fetch_pantheon_rooms() -> Result<Vec<PantheonRoom>, String> {
    #[derive(Deserialize, Default)]
    struct Envelope { #[serde(default)] rooms: Vec<PantheonRoom> }
    if let Ok(e) = fetch_json::<Envelope>("/v1/pantheon/rooms").await {
        return Ok(e.rooms);
    }
    fetch_json::<Vec<PantheonRoom>>("/v1/pantheon/rooms").await
}

pub async fn fetch_room_messages(room_id: &str, limit: usize) -> Result<Vec<PantheonRoomMessage>, String> {
    #[derive(Deserialize, Default)]
    struct Envelope { #[serde(default)] messages: Vec<PantheonRoomMessage> }
    let url = format!("/v1/pantheon/rooms/{}/messages?limit={}", room_id, limit);
    if let Ok(e) = fetch_json::<Envelope>(&url).await {
        return Ok(e.messages);
    }
    fetch_json::<Vec<PantheonRoomMessage>>(&url).await
}

pub async fn send_room_message(room_id: &str, req: &SendRoomMessageRequest) -> Result<PantheonRoomMessage, String> {
    post_json(&format!("/v1/pantheon/rooms/{}/messages", room_id), req).await
}

/// Upload a file to a Pantheon war room (multipart/form-data)
pub async fn upload_room_file(room_id: &str, file: &web_sys::File, sender_id: &str, sender_name: &str, message: &str) -> Result<PantheonRoomMessage, String> {
    let form_data = web_sys::FormData::new().map_err(|e| format!("FormData: {:?}", e))?;
    form_data.append_with_blob("file", file).map_err(|e| format!("Append blob: {:?}", e))?;
    form_data.append_with_str("sender_id", sender_id).map_err(|e| format!("Append: {:?}", e))?;
    form_data.append_with_str("sender_name", sender_name).map_err(|e| format!("Append: {:?}", e))?;
    if !message.is_empty() {
        form_data.append_with_str("message", message).map_err(|e| format!("Append: {:?}", e))?;
    }

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form_data);

    let url = format!("/v1/pantheon/rooms/{}/upload", room_id);
    let request = web_sys::Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| format!("Request: {:?}", e))?;
    if let Some(auth) = auth_bearer() {
        request.headers().set("Authorization", &auth).map_err(|e| format!("Header: {:?}", e))?;
    }

    let window = web_sys::window().ok_or("No window")?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await.map_err(|e| format!("Fetch: {:?}", e))?;
    let resp: web_sys::Response = resp_val.dyn_into().map_err(|_| "Response cast failed".to_string())?;

    if !resp.ok() {
        let status = resp.status();
        let body = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|e| format!("Text: {:?}", e))?)
            .await.map_err(|e| format!("Read: {:?}", e))?.as_string().unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, body));
    }

    let text = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|e| format!("Text: {:?}", e))?)
        .await.map_err(|e| format!("Read: {:?}", e))?.as_string().unwrap_or_default();
    serde_json::from_str::<PantheonRoomMessage>(&text)
        .map_err(|e| format!("Parse: {} (body: {})", e, &text[..text.len().min(200)]))
}

pub async fn fetch_room_members(room_id: &str) -> Result<Vec<PantheonRoomMember>, String> {
    #[derive(Deserialize, Default)]
    struct Envelope { #[serde(default)] members: Vec<PantheonRoomMember> }
    let url = format!("/v1/pantheon/rooms/{}/members", room_id);
    if let Ok(e) = fetch_json::<Envelope>(&url).await {
        return Ok(e.members);
    }
    fetch_json::<Vec<PantheonRoomMember>>(&url).await
}

/// Create a Pantheon room (public or private)
pub async fn create_room(name: &str, description: Option<&str>, room_type: &str, created_by: &str) -> Result<PantheonRoom, String> {
    let mut body = serde_json::json!({
        "name": name,
        "room_type": room_type,
        "created_by": created_by,
    });
    if let Some(desc) = description {
        body["description"] = serde_json::Value::String(desc.to_string());
    }
    post_json("/v1/pantheon/rooms", &body).await
}

/// Invite agent to a private room
pub async fn invite_to_room(room_id: &str, agent_id: &str, agent_name: &str) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/pantheon/rooms/{}/join", room_id),
        &serde_json::json!({ "agent_id": agent_id, "agent_name": agent_name }),
    ).await
}

pub async fn join_room(room_id: &str, agent_id: &str, agent_name: &str) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/pantheon/rooms/{}/join", room_id),
        &serde_json::json!({ "agent_id": agent_id, "agent_name": agent_name }),
    ).await
}

pub async fn leave_room(room_id: &str, agent_id: &str) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/pantheon/rooms/{}/leave", room_id),
        &serde_json::json!({ "agent_id": agent_id }),
    ).await
}

pub async fn delete_room_message(room_id: &str, msg_id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/pantheon/rooms/{}/messages/{}", room_id, msg_id)).await
}

pub async fn edit_room_message(room_id: &str, msg_id: &str, content: &str) -> Result<PantheonRoomMessage, String> {
    put_json(
        &format!("/v1/pantheon/rooms/{}/messages/{}", room_id, msg_id),
        &serde_json::json!({ "content": content }),
    ).await
}

pub async fn add_reaction(room_id: &str, msg_id: &str, emoji: &str, user_id: &str) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/pantheon/rooms/{}/messages/{}/reactions", room_id, msg_id),
        &serde_json::json!({ "emoji": emoji, "user_id": user_id }),
    ).await
}

pub async fn remove_reaction(room_id: &str, msg_id: &str, emoji: &str, user_id: &str) -> Result<(), String> {
    delete_json::<serde_json::Value>(&format!(
        "/v1/pantheon/rooms/{}/messages/{}/reactions?emoji={}&user_id={}",
        room_id, msg_id, emoji, user_id
    )).await.map(|_| ())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoomReaction {
    #[serde(default)] pub emoji: String,
    #[serde(default)] pub count: usize,
    #[serde(default)] pub user_ids: Vec<String>,
}

pub async fn fetch_reactions(room_id: &str, msg_id: &str) -> Result<Vec<RoomReaction>, String> {
    #[derive(Deserialize, Default)]
    struct Envelope { #[serde(default)] reactions: Vec<RoomReaction> }
    let url = format!("/v1/pantheon/rooms/{}/messages/{}/reactions", room_id, msg_id);
    if let Ok(e) = fetch_json::<Envelope>(&url).await {
        return Ok(e.reactions);
    }
    fetch_json::<Vec<RoomReaction>>(&url).await
}

// ═══════════════════════════════════════════════════════════
// Deploy — Phase 4: One-Click Deploy
// ═══════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployTarget {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub target_type: String,      // vercel, netlify, docker, vibesaas, self_hosted
    #[serde(default)] pub config: DeployTargetConfig,
    #[serde(default)] pub status: String,            // active, inactive, error
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployTargetConfig {
    #[serde(default)] pub api_key: String,
    #[serde(default)] pub project_id: String,
    #[serde(default)] pub team_id: String,
    #[serde(default)] pub region: String,
    #[serde(default)] pub custom_domain: String,
    #[serde(default)] pub extra: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployTargetsResponse {
    #[serde(default)] pub targets: Vec<DeployTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Deployment {
    #[serde(default)] pub id: String,
    #[serde(default)] pub target_id: String,
    #[serde(default)] pub target_type: String,
    #[serde(default)] pub project_name: String,
    #[serde(default)] pub status: String,            // building, deploying, live, failed, rolled_back
    #[serde(default)] pub url: String,
    #[serde(default)] pub preview_url: String,
    #[serde(default)] pub version: u32,
    #[serde(default)] pub created_at: String,
    #[serde(default)] pub completed_at: String,
    #[serde(default)] pub error_message: String,
    #[serde(default)] pub room_id: String,
    #[serde(default)] pub spawn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployHistoryResponse {
    #[serde(default)] pub deployments: Vec<Deployment>,
    #[serde(default)] pub total: u64,
}

/// GET /v1/deploy/targets — List configured deploy targets
pub async fn fetch_deploy_targets() -> Result<DeployTargetsResponse, String> {
    fetch_json("/v1/deploy/targets").await
}

/// POST /v1/deploy/targets — Add a new deploy target
pub async fn create_deploy_target(name: &str, target_type: &str, config: &DeployTargetConfig) -> Result<DeployTarget, String> {
    post_json("/v1/deploy/targets", &serde_json::json!({
        "name": name,
        "target_type": target_type,
        "config": config,
    })).await
}

/// DELETE /v1/deploy/targets/:id — Remove a deploy target
pub async fn delete_deploy_target(target_id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/deploy/targets/{}", target_id)).await
}

/// GET /v1/deploy/history — List deployments
pub async fn fetch_deploy_history(limit: Option<u32>) -> Result<DeployHistoryResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/deploy/history{}", q)).await
}

/// POST /v1/deploy/:id/rollback — Rollback a deployment
pub async fn rollback_deployment(deploy_id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/deploy/{}/rollback", deploy_id), &serde_json::json!({})).await
}

// ══════════════════════════════════════════════════════════════
// NOUS — /v1/nous/* (Phase 7 Intelligence Layer)
// ══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousImprovementNeed {
    #[serde(default)] pub area: String,
    #[serde(default)] pub current_level: f64,
    #[serde(default)] pub target_level: f64,
    #[serde(default)] pub priority: u32,
    #[serde(default)] pub suggested_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousReflection {
    #[serde(default)] pub health: f64,
    #[serde(default)] pub state: String,
    #[serde(default)] pub current_focus: String,
    #[serde(default)] pub recent_successes: Vec<String>,
    #[serde(default)] pub recent_challenges: Vec<String>,
    #[serde(default)] pub improvement_needs: Vec<NousImprovementNeed>,
    #[serde(default)] pub learned_insights: Vec<String>,
    #[serde(default)] pub summary: String,
    #[serde(default)] pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousCapability {
    #[serde(default)] pub name: String,
    #[serde(default)] pub description: String,
    #[serde(default)] pub proficiency: f64,
    #[serde(default)] pub usage_count: u64,
    #[serde(default)] pub success_rate: f64,
    #[serde(default)] pub limitations: Vec<String>,
    #[serde(default)] pub improvement_areas: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousCapabilitiesResponse {
    #[serde(default)] pub capabilities: Vec<NousCapability>,
    #[serde(default)] pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousLearningStats {
    #[serde(default)] pub total_lessons: u64,
    #[serde(default)] pub total_intents: u64,
    #[serde(default)] pub total_outcomes: u64,
    #[serde(default)] pub success_rate: f64,
    #[serde(default)] pub avg_lesson_confidence: f64,
    #[serde(default)] pub lessons_by_category: std::collections::HashMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousLesson {
    #[serde(default)] pub id: String,
    #[serde(default)] pub insight: String,
    #[serde(default)] pub category: String,
    #[serde(default)] pub conditions: Vec<String>,
    #[serde(default)] pub recommendation: String,
    #[serde(default)] pub confidence: f64,
    #[serde(default)] pub reinforcements: u32,
    #[serde(default)] pub learned_at: String,
    #[serde(default)] pub last_reinforced: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NousLessonsResponse {
    #[serde(default)] pub lessons: Vec<NousLesson>,
    #[serde(default)] pub count: usize,
    #[serde(default)] pub total: usize,
}

pub async fn fetch_nous_reflection() -> Result<NousReflection, String> {
    fetch_json("/v1/nous/reflect").await
}

pub async fn fetch_nous_capabilities() -> Result<NousCapabilitiesResponse, String> {
    fetch_json("/v1/nous/capabilities").await
}

pub async fn fetch_nous_learning_stats() -> Result<NousLearningStats, String> {
    fetch_json("/v1/nous/learning/stats").await
}

pub async fn fetch_nous_lessons(limit: Option<usize>) -> Result<NousLessonsResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/nous/learning/lessons{}", q)).await
}

pub async fn nous_understand(input: &str) -> Result<serde_json::Value, String> {
    post_json::<serde_json::Value, serde_json::Value>(
        "/v1/nous/understand",
        &serde_json::json!({ "input": input }),
    ).await
}

pub async fn nous_reason(problem: &str) -> Result<serde_json::Value, String> {
    post_json::<serde_json::Value, serde_json::Value>(
        "/v1/nous/reason",
        &serde_json::json!({ "problem": problem }),
    ).await
}

// ── Memory Graph — /v1/memory/graph/* ──

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GraphNode {
    #[serde(default)] pub id: i64,
    #[serde(default)] pub name: String,
    #[serde(rename = "type", default)] pub node_type: String,
    #[serde(default)] pub aliases: Vec<String>,
    #[serde(default)] pub mention_count: u64,
    #[serde(default)] pub first_seen: String,
    #[serde(default)] pub last_seen: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GraphNodesResponse {
    #[serde(default)] pub nodes: Vec<GraphNode>,
    #[serde(default)] pub count: usize,
    #[serde(default)] pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GraphEdgeType {
    #[serde(default)] pub relationship_type: String,
    #[serde(default)] pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GraphEdgesResponse {
    #[serde(default)] pub edge_types: Vec<GraphEdgeType>,
    #[serde(default)] pub total_edges: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryGraphStats {
    #[serde(default)] pub memory: serde_json::Value,
    #[serde(default)] pub graph: serde_json::Value,
    #[serde(default)] pub patterns: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryPattern {
    #[serde(default)] pub id: i64,
    #[serde(default)] pub pattern_type: String,
    #[serde(default)] pub content: String,
    #[serde(default)] pub frequency: u64,
    #[serde(default)] pub first_seen: String,
    #[serde(default)] pub last_seen: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MemoryPatternsResponse {
    #[serde(default)] pub patterns: Vec<MemoryPattern>,
    #[serde(default)] pub count: usize,
}

pub async fn fetch_graph_nodes(limit: Option<usize>) -> Result<GraphNodesResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/memory/graph/nodes{}", q)).await
}

pub async fn fetch_graph_edges() -> Result<GraphEdgesResponse, String> {
    fetch_json("/v1/memory/graph/edges").await
}

pub async fn fetch_graph_stats() -> Result<MemoryGraphStats, String> {
    fetch_json("/v1/memory/graph/stats").await
}

pub async fn fetch_memory_patterns(limit: Option<usize>) -> Result<MemoryPatternsResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/memory/patterns{}", q)).await
}

// ── Predictive Spawning — Zeus112 `c5df7610` ─────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnerHealth {
    #[serde(default)] pub active_spawns: u64,
    #[serde(default)] pub completed_spawns: u64,
    #[serde(default)] pub failed_spawns: u64,
    #[serde(default)] pub success_rate: f64,
    #[serde(default)] pub is_healthy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnerCriteria {
    #[serde(default)] pub min_complexity: String,
    #[serde(default)] pub max_spawn_count: u64,
    #[serde(default)] pub max_active_agents: u64,
    #[serde(default)] pub enable_parallel: bool,
    #[serde(default)] pub enable_specialization: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnerStatus {
    #[serde(default)] pub health: SpawnerHealth,
    #[serde(default)] pub criteria: SpawnerCriteria,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ActiveSpawn {
    #[serde(default)] pub agent_id: String,
    #[serde(default)] pub role: String,
    #[serde(default)] pub task: String,
    #[serde(default)] pub tools: Vec<String>,
    #[serde(default)] pub started_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnHistoryEntry {
    #[serde(default)] pub request_id: String,
    #[serde(default)] pub success: bool,
    #[serde(default)] pub duration_ms: u64,
    #[serde(default)] pub output: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnAgentRecommendation {
    #[serde(default)] pub id: String,
    #[serde(default)] pub role: String,
    #[serde(default)] pub task: String,
    #[serde(default)] pub tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnAnalysis {
    #[serde(default)] pub detected_complexity: String,
    #[serde(default)] pub tool_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpawnAnalyzeResponse {
    #[serde(default)] pub should_spawn: bool,
    #[serde(default)] pub rationale: String,
    #[serde(default)] pub estimated_speedup: f64,
    #[serde(default)] pub agents: Vec<SpawnAgentRecommendation>,
    #[serde(default)] pub analysis: SpawnAnalysis,
}

pub async fn fetch_spawner_status() -> Result<SpawnerStatus, String> {
    fetch_json("/v1/spawner/status").await
}

pub async fn fetch_spawner_active() -> Result<Vec<ActiveSpawn>, String> {
    fetch_json("/v1/spawner/active").await
}

pub async fn fetch_spawner_history() -> Result<Vec<SpawnHistoryEntry>, String> {
    fetch_json("/v1/spawner/history").await
}

pub async fn spawner_analyze(task: &str, tools: Vec<String>) -> Result<SpawnAnalyzeResponse, String> {
    post_json::<serde_json::Value, SpawnAnalyzeResponse>(
        "/v1/spawner/analyze",
        &serde_json::json!({"task": task, "tools": tools}),
    ).await
}

// ─── Outcome Templates (S12-7) ────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OutcomeTemplate {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub description: String,
    #[serde(default)] pub categories: Vec<String>,
    #[serde(default)] pub required_skills: Vec<String>,
    #[serde(default)] pub required_providers: Vec<String>,
    #[serde(default)] pub tags: Vec<String>,
    #[serde(default)] pub builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TemplatesListResponse {
    #[serde(default)] pub templates: Vec<OutcomeTemplate>,
    #[serde(default)] pub total: u64,
    #[serde(default)] pub offset: u64,
    #[serde(default)] pub limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CategoriesResponse {
    #[serde(default)] pub categories: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AppliedTemplate {
    #[serde(default)] pub enriched_prompt: String,
    #[serde(default)] pub missing_providers: Vec<String>,
    #[serde(default)] pub missing_skills: Vec<String>,
    #[serde(default)] pub warnings: Vec<String>,
    #[serde(default)] pub tool_policy: serde_json::Value,
    #[serde(default)] pub planning_config: serde_json::Value,
}

pub async fn fetch_templates(category: Option<&str>, limit: Option<u64>) -> Result<TemplatesListResponse, String> {
    let mut url = "/v1/templates?".to_string();
    if let Some(cat) = category { url.push_str(&format!("category={}&", cat)); }
    if let Some(l) = limit { url.push_str(&format!("limit={}&", l)); }
    fetch_json(&url).await
}

pub async fn fetch_template_categories() -> Result<Vec<String>, String> {
    fetch_json::<CategoriesResponse>("/v1/templates/categories").await.map(|r| r.categories)
}

pub async fn search_templates(q: &str) -> Result<Vec<OutcomeTemplate>, String> {
    fetch_json::<TemplatesListResponse>(&format!("/v1/templates/search?q={}", q)).await.map(|r| r.templates)
}

pub async fn apply_template(id: &str, goal: &str) -> Result<AppliedTemplate, String> {
    post_json::<serde_json::Value, AppliedTemplate>(
        &format!("/v1/templates/{}/apply", id),
        &serde_json::json!({ "goal": goal }),
    ).await
}

// ─── Blog CMS ─────────────────────────────────────────────

// Blog API removed — marketing site is a separate project (zeuslab.ai)

// ─── Agent Studio — /v1/studio/* ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StudioSession {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub total_actions: u64,
    #[serde(default)]
    pub completed_actions: u64,
    #[serde(default)]
    pub failed_actions: u64,
    #[serde(default)]
    pub error_message: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StudioSessionsResponse {
    #[serde(default)]
    pub sessions: Vec<StudioSession>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StudioStats {
    #[serde(default)]
    pub total_sessions: u64,
    #[serde(default)]
    pub active_sessions: u64,
    #[serde(default)]
    pub completed_sessions: u64,
    #[serde(default)]
    pub total_actions: u64,
    #[serde(default)]
    pub total_artifacts: u64,
}

pub async fn create_studio_session(goal: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/studio/sessions", &serde_json::json!({"goal": goal, "user_id": "default"})).await
}

pub async fn fetch_studio_sessions() -> Result<StudioSessionsResponse, String> {
    fetch_json("/v1/studio/sessions").await
}

pub async fn fetch_studio_session(id: &str) -> Result<StudioSession, String> {
    fetch_json(&format!("/v1/studio/sessions/{}", id)).await
}

pub async fn delete_studio_session(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/studio/sessions/{}", id)).await
}

pub async fn drive_studio_session(id: &str, approved: bool) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/studio/sessions/{}/drive", id),
        &serde_json::json!({"approved": approved}),
    ).await
}

pub async fn fetch_studio_stats() -> Result<StudioStats, String> {
    fetch_json("/v1/studio/stats").await
}

pub async fn fetch_active_studio_sessions() -> Result<StudioSessionsResponse, String> {
    fetch_json("/v1/studio/sessions/active").await
}
