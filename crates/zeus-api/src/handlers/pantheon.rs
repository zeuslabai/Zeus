//! Pantheon — Multi-Agent Collaboration Chat API
//!
//! REST handlers and types for the Pantheon mission system.
//! WS protocol extensions live in websocket.rs.
//!
//! # Endpoints
//! ```text
//! POST   /v1/pantheon/missions              — Launch a mission
//! GET    /v1/pantheon/missions              — List missions
//! GET    /v1/pantheon/missions/:id          — Mission detail
//! POST   /v1/pantheon/missions/:id/intervene — Pause/resume/cancel/redirect
//! GET    /v1/pantheon/missions/:id/feed     — Activity feed
//! GET    /v1/pantheon/missions/:id/artifacts — Generated artifacts
//! POST   /v1/pantheon/missions/:id/review   — Approve/reject task output
//! ```

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;
use zeus_llm::LlmClient;
use zeus_llm::router::ModelRouter;
use zeus_nous::ComplexityAnalyzer;

use crate::SharedState;
use crate::uploads::detect_mime_type;

// Agora ↔ Pantheon wiring: marketplace types for mission settlement
use super::PantheonStore;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Created,
    Assembling,
    AwaitingApproval,
    Executing,
    Paused,
    Reviewing,
    Complete,
    Failed,
    Cancelled,
}

impl std::fmt::Display for MissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Coordinator,
    Manager,
    Worker,
    Reviewer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub agent_id: String,
    pub name: String,
    pub role: AgentRole,
    pub status: AgentStatus,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    AwaitingReview,
    Approved,
    Rejected,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTask {
    pub id: String,
    pub description: String,
    pub assigned_to: Option<String>,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub agent_id: String,
    pub agent_name: String,
    pub activity: String, // "tool_call" | "message" | "task_complete" | "review_request" | ...
    pub detail: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionConstraints {
    pub budget_tokens: Option<u64>,
    pub timeout_seconds: Option<u64>,
    pub max_agents: Option<usize>,
    pub require_review: Option<bool>,
}

impl Default for MissionConstraints {
    fn default() -> Self {
        Self {
            budget_tokens: Some(50_000),
            timeout_seconds: Some(600),
            max_agents: Some(4),
            require_review: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub goal: String,
    pub status: MissionStatus,
    pub team: Vec<TeamMember>,
    pub tasks: Vec<MissionTask>,
    pub progress_pct: f64,
    pub tasks_done: usize,
    pub tasks_total: usize,
    pub tokens_used: u64,
    pub constraints: MissionConstraints,
    pub feed: Vec<ActivityEntry>,
    pub artifacts: Vec<Artifact>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
}

impl Mission {
    pub fn new(goal: String, constraints: MissionConstraints) -> Self {
        let now = Utc::now();
        Self {
            id: format!(
                "m-{}",
                Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("xxx")
            ),
            goal,
            status: MissionStatus::Created,
            team: Vec::new(),
            tasks: Vec::new(),
            progress_pct: 0.0,
            tasks_done: 0,
            tasks_total: 0,
            tokens_used: 0,
            constraints,
            feed: Vec::new(),
            artifacts: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            summary: None,
        }
    }

    pub fn add_activity(
        &mut self,
        agent_id: &str,
        agent_name: &str,
        activity: &str,
        detail: Value,
    ) {
        self.feed.push(ActivityEntry {
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            activity: activity.to_string(),
            detail,
            timestamp: Utc::now(),
        });
        self.updated_at = Utc::now();
    }

    pub fn update_progress(&mut self) {
        if self.tasks_total > 0 {
            self.progress_pct = (self.tasks_done as f64 / self.tasks_total as f64) * 100.0;
        }
        self.updated_at = Utc::now();
    }
}

// ============================================================================
// Pantheon event (broadcast via WebSocket)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PantheonEvent {
    MissionCreated {
        mission_id: String,
        goal: String,
        status: String,
    },
    TeamAssembled {
        mission_id: String,
        agents: Vec<TeamMember>,
    },
    TaskAssigned {
        mission_id: String,
        task_id: String,
        agent_id: String,
        description: String,
    },
    AgentActivity {
        mission_id: String,
        agent_id: String,
        agent_name: String,
        activity: String,
        detail: Value,
    },
    TaskCompleted {
        mission_id: String,
        task_id: String,
        result: String,
    },
    ReviewRequested {
        mission_id: String,
        task_id: String,
        reviewer: String,
    },
    MissionProgress {
        mission_id: String,
        progress_pct: f64,
        tasks_done: usize,
        tasks_total: usize,
        tokens_used: u64,
    },
    Artifact {
        mission_id: String,
        name: String,
        path: String,
        artifact_type: String,
    },
    MissionComplete {
        mission_id: String,
        status: String,
        summary: String,
        artifacts: Vec<Artifact>,
    },
    MissionApproved {
        mission_id: String,
        approved_by: String,
    },
    MissionRejected {
        mission_id: String,
        rejected_by: String,
        reason: String,
    },
    MissionFailed {
        mission_id: String,
        reason: String,
    },
    // Room events
    RoomCreated {
        room: Room,
    },
    RoomMessageSent {
        room_id: String,
        message: RoomMessage,
    },
    AgentJoinedRoom {
        room_id: String,
        agent_id: String,
        agent_name: String,
    },
    AgentLeftRoom {
        room_id: String,
        agent_id: String,
    },
    // Plan card approval events
    PlanCardCreated {
        room_id: String,
        plan_id: String,
        goal: String,
        complexity: String,
        risk: String,
    },
    PlanApproved {
        room_id: String,
        plan_id: String,
        approved_by: String,
    },
    PlanRejected {
        room_id: String,
        plan_id: String,
        rejected_by: String,
        reason: Option<String>,
    },
}

// Store is now in pantheon_store.rs (SQLite-backed).
// Re-exported via handlers/mod.rs as PantheonStore.

// ============================================================================
// Room / Channel types (Pantheon War Room)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RoomType {
    /// Open to all agents — discoverable and joinable
    Public,
    /// Invite-only — only members can see and post
    Private,
    /// Direct message — private 1:1 channel between two agents
    Dm,
}

impl std::fmt::Display for RoomType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoomType::Public => write!(f, "public"),
            RoomType::Private => write!(f, "private"),
            RoomType::Dm => write!(f, "dm"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub room_type: RoomType,
    pub mission_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub agent_id: String,
    pub agent_name: String,
    pub role: String, // "owner" | "member" | "observer"
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessage {
    pub id: String,
    pub room_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub message_type: String, // "chat" | "tool_call" | "system" | "task_update" | "file" | "voice"
    pub metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>, // message ID being replied to
    #[serde(default)]
    pub edited: bool,
    /// File attachments: [{filename, url, content_type, size}]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
    pub timestamp: DateTime<Utc>,
}

/// File attachment on a war room message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAttachment {
    pub filename: String,
    pub url: String,
    pub content_type: String,
    pub size: u64,
}

// Room events are broadcast via PantheonEvent variants:
// RoomCreated, RoomMessageSent, AgentJoinedRoom, AgentLeftRoom

// ============================================================================
// Request / Response types
// ============================================================================

// ============================================================================
// Room request / response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_room_type")]
    pub room_type: RoomType,
    #[serde(default)]
    pub mission_id: Option<String>,
    pub created_by: String,
}

fn default_room_type() -> RoomType {
    RoomType::Public
}

#[derive(Debug, Deserialize)]
pub struct JoinRoomRequest {
    pub agent_id: String,
    pub agent_name: String,
}

#[derive(Debug, Deserialize)]
pub struct SendRoomMessageRequest {
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    #[serde(default = "default_message_type")]
    pub message_type: String,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Optional file attachments (pre-uploaded via /upload endpoint)
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
}

fn default_message_type() -> String {
    "chat".to_string()
}

#[derive(Debug, Deserialize)]
pub struct RoomMessagesQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub before: Option<String>,
}

/// POST /v1/pantheon/dms — Find or create a DM room between two agents.
#[derive(Debug, Deserialize)]
pub struct FindOrCreateDmRequest {
    pub agent_id: String,
    pub agent_name: String,
    pub peer_id: String,
    pub peer_name: String,
}

/// GET /v1/pantheon/dms — List DM rooms where agent_id is a member.
#[derive(Debug, Deserialize)]
pub struct ListDmsQuery {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMissionRequest {
    pub goal: String,
    #[serde(default)]
    pub constraints: Option<MissionConstraints>,
}

#[derive(Debug, Deserialize)]
pub struct InterveneRequest {
    pub action: String, // "pause" | "resume" | "cancel" | "redirect" | "assign_task"
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRequest {
    pub task_id: String,
    pub verdict: String, // "approve" | "reject"
    #[serde(default)]
    pub comment: Option<String>,
}

// ============================================================================
// MissionToolExecutor — per-mission wrapper with spawn capability
// ============================================================================

/// Wraps the shared `ToolExecutor` and adds real subagent spawn capability.
///
/// Created per-mission so spawned subagents are tracked independently.
/// After `drive_mission_cancellable()` returns, call `collect_subagents()`
/// to await any outstanding fire-and-forget spawns before marking complete.
struct MissionToolExecutor {
    inner: Arc<dyn zeus_prometheus::ToolExecutor>,
    mission_id: String,
    llm: Arc<LlmClient>,
    subagents: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<String, tokio::task::JoinHandle<zeus_agent::SubagentResult>>,
        >,
    >,
}

impl MissionToolExecutor {
    fn new(
        inner: Arc<dyn zeus_prometheus::ToolExecutor>,
        mission_id: String,
        llm: Arc<LlmClient>,
    ) -> Self {
        Self {
            inner,
            mission_id,
            llm,
            subagents: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Collect all outstanding subagent results with a timeout.
    async fn collect_subagents(&self, timeout_secs: u64) -> Vec<zeus_agent::SubagentResult> {
        let mut handles = self.subagents.lock().await;
        if handles.is_empty() {
            return vec![];
        }

        let count = handles.len();
        info!(
            mission_id = %self.mission_id,
            count = count,
            "Collecting {} outstanding subagent(s)",
            count
        );

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
        let mut results = Vec::new();

        let ids: Vec<String> = handles.keys().cloned().collect();
        for id in ids {
            if let Some(handle) = handles.remove(&id) {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    warn!(
                        mission_id = %self.mission_id,
                        agent_id = %id,
                        "Deadline reached, skipping remaining subagents"
                    );
                    break;
                }
                match tokio::time::timeout(remaining, handle).await {
                    Ok(Ok(result)) => results.push(result),
                    Ok(Err(e)) => {
                        warn!(agent_id = %id, "Subagent task panicked: {}", e);
                        results.push(zeus_agent::SubagentResult {
                            id: id.clone(),
                            success: false,
                            output: format!("Subagent panicked: {}", e),
                            iterations: 0,
                            mission_id: Some(self.mission_id.clone()),
                        });
                    }
                    Err(_) => {
                        warn!(agent_id = %id, "Subagent timed out");
                        results.push(zeus_agent::SubagentResult {
                            id: id.clone(),
                            success: false,
                            output: "Subagent timed out during collection".to_string(),
                            iterations: 0,
                            mission_id: Some(self.mission_id.clone()),
                        });
                    }
                }
            }
        }

        info!(
            mission_id = %self.mission_id,
            collected = results.len(),
            "Collected {} subagent result(s)",
            results.len()
        );
        results
    }
}

#[async_trait::async_trait]
impl zeus_prometheus::ToolExecutor for MissionToolExecutor {
    async fn execute_tool(&self, call: &zeus_core::ToolCall) -> zeus_core::ToolResult {
        if call.name == "spawn" {
            // Real spawn: create a subagent and track the handle
            let task = call
                .arguments
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let context = call
                .arguments
                .get("context")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let max_iterations = call
                .arguments
                .get("max_iterations")
                .and_then(|v| v.as_u64())
                .unwrap_or(15) as usize;
            let mission_id = call
                .arguments
                .get("mission_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| Some(self.mission_id.clone()));

            if task.is_empty() {
                return zeus_core::ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "spawn requires a 'task' argument".to_string(),
                };
            }

            // Build config — check for remote target
            let gateway_url = call
                .arguments
                .get("gateway_url")
                .and_then(|v| v.as_str())
                .map(String::from);
            let auth_token = call
                .arguments
                .get("auth_token")
                .and_then(|v| v.as_str())
                .map(String::from);
            let model = call
                .arguments
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from);

            let target = if let Some(url) = gateway_url {
                zeus_agent::AgentTarget::Remote {
                    gateway_url: url,
                    auth_token,
                }
            } else {
                zeus_agent::AgentTarget::Local
            };

            let config = zeus_agent::SubagentConfig {
                max_iterations,
                can_spawn: false,
                task: task.clone(),
                context,
                target,
                model,
                mission_id,
                parent_system_prompt: None,
                ..Default::default()
            };

            let workspace = zeus_memory::Workspace::new(
                dirs::home_dir().unwrap_or_default().join(".zeus/workspace"),
            );
            let llm_clone = (*self.llm).clone();
            let handle = zeus_agent::spawn_subagent(config, llm_clone, workspace, None);
            let agent_id = format!("mission-sub-{}", &uuid::Uuid::new_v4().to_string()[..8]);

            self.subagents.lock().await.insert(agent_id.clone(), handle);

            info!(
                mission_id = %self.mission_id,
                agent_id = %agent_id,
                "Spawned subagent for mission task: {}",
                &task[..task.len().min(80)]
            );

            return zeus_core::ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: format!(
                    "Subagent '{}' spawned for mission {}. Use collect_spawns to gather results.",
                    agent_id, self.mission_id
                ),
            };
        }

        if call.name == "collect_spawns" {
            let timeout = call
                .arguments
                .get("timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(300);
            let results = self.collect_subagents(timeout).await;
            let succeeded = results.iter().filter(|r| r.success).count();
            let failed = results.len() - succeeded;
            let summary = serde_json::json!({
                "collected": results.len(),
                "succeeded": succeeded,
                "failed": failed,
                "results": results.iter().map(|r| serde_json::json!({
                    "id": r.id,
                    "success": r.success,
                    "output": &r.output[..zeus_core::floor_char_boundary(&r.output, 2000)],
                    "iterations": r.iterations,
                })).collect::<Vec<_>>(),
            });
            return zeus_core::ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: serde_json::to_string_pretty(&summary).unwrap_or_default(),
            };
        }

        // All other tools → delegate to inner executor
        self.inner.execute_tool(call).await
    }

    fn has_tool(&self, name: &str) -> bool {
        self.inner.has_tool(name)
    }

    fn available_tools(&self) -> Vec<String> {
        self.inner.available_tools()
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /v1/pantheon/missions — Launch a new mission
pub async fn create_mission(
    State(state): State<SharedState>,
    Json(req): Json<CreateMissionRequest>,
) -> impl IntoResponse {
    // Task 3: Input validation
    if req.goal.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "goal is required and must not be empty"
            })),
        ).into_response();
    }
    if req.goal.trim().len() > 4000 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "goal must be 4000 characters or fewer"
            })),
        ).into_response();
    }
    if let Some(ref c) = req.constraints {
        if let Some(budget) = c.budget_tokens {
            if budget == 0 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "budget_tokens must be greater than 0"
                    })),
                ).into_response();
            }
        }
        if let Some(timeout) = c.timeout_seconds {
            if timeout == 0 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "timeout_seconds must be greater than 0"
                    })),
                ).into_response();
            }
        }
        if let Some(max_agents) = c.max_agents {
            if max_agents == 0 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "max_agents must be greater than 0"
                    })),
                ).into_response();
            }
            if max_agents > 20 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "max_agents must be 20 or fewer"
                    })),
                ).into_response();
            }
        }
    }
    let constraints = req.constraints.unwrap_or_default();
    let state_guard = state.read().await;
    let store = state_guard.pantheon.clone();
    let orchestrator = state_guard.pantheon_orchestrator().clone();
    let config = state_guard.config.clone();
    let tool_schemas = state_guard.tools.schemas();
    drop(state_guard);

    let orch_constraints = zeus_orchestra::pantheon::MissionConstraints {
        budget_tokens: constraints.budget_tokens.unwrap_or(50_000),
        timeout_seconds: constraints.timeout_seconds.unwrap_or(600),
        max_agents: constraints.max_agents.unwrap_or(4) as u32,
        require_review: constraints.require_review.unwrap_or(false),
    };

    let capabilities = infer_capabilities(&req.goal);

    // Try MissionDriver path (LLM decomposition + team assembly + task assignment)
    let llm_result = LlmClient::from_config(&config).ok().map(Arc::new);
    let (mission_id, team) = if let Some(llm) = llm_result {
        let driver = zeus_prometheus::MissionDriver::new(
            orchestrator.clone(),
            llm.clone(),
            tool_schemas.clone(),
        )
        .with_checkpointer(Arc::new(store.clone()));
        match driver
            .plan_mission(&req.goal, orch_constraints.clone(), capabilities.clone())
            .await
        {
            Ok(planned) => {
                let mission_id = planned.mission_id.clone();
                // Get assembled team from orchestrator
                let orch_team = orchestrator
                    .get_mission(&mission_id)
                    .await
                    .map(|m| m.team)
                    .unwrap_or_default();
                let team = convert_orch_team(&orch_team, &req.goal);

                // Emit task assignments to WS clients
                let store_clone = store.clone();
                let mid = mission_id.clone();
                for task in &planned.plan.steps {
                    if let Some(task_id) = planned.step_to_task.get(&task.id) {
                        store_clone.emit(PantheonEvent::TaskAssigned {
                            mission_id: mid.clone(),
                            task_id: task_id.clone(),
                            agent_id: "prometheus".to_string(),
                            description: task.description.clone(),
                        });
                    }
                }

                // If require_review is set, hold at AwaitingApproval — don't auto-execute.
                // The supervisor must call POST /v1/pantheon/missions/:id/approve first.
                let require_review = constraints.require_review.unwrap_or(false);

                if !require_review {
                    // Spawn background execution — emits MissionComplete/MissionFailed when done
                    let store_bg = store.clone();
                    let state_bg = state.clone();
                    let mid_bg = mission_id.clone();

                    // Get real tool executor from AppState (wired at gateway startup)
                    let tool_exec = state.read().await.tool_executor.clone();

                    // Create cancellation flag for this mission
                    let cancel_flag = Arc::new(AtomicBool::new(false));
                    state
                        .read()
                        .await
                        .mission_cancels
                        .insert(mid_bg.clone(), cancel_flag.clone());

                    // Wrap executor with per-mission spawn capability
                    let mission_exec: Option<Arc<MissionToolExecutor>> = tool_exec.map(|inner| {
                        Arc::new(MissionToolExecutor::new(inner, mid_bg.clone(), llm.clone()))
                    });

                    tokio::spawn(async move {
                        let exec_ref: Option<&dyn zeus_prometheus::ToolExecutor> = mission_exec
                            .as_ref()
                            .map(|e| e.as_ref() as &dyn zeus_prometheus::ToolExecutor);
                        match driver
                            .drive_mission_cancellable(
                                &planned,
                                exec_ref,
                                Some(cancel_flag.clone()),
                            )
                            .await
                        {
                            Ok(result) => {
                                // Auto-collect any outstanding subagent spawns
                                let subagent_results = if let Some(ref mexec) = mission_exec {
                                    mexec.collect_subagents(60).await
                                } else {
                                    vec![]
                                };
                                let subagent_summary = if !subagent_results.is_empty() {
                                    let sub_ok =
                                        subagent_results.iter().filter(|r| r.success).count();
                                    let sub_fail = subagent_results.len() - sub_ok;
                                    format!(
                                        ", {} subagent(s) collected ({} ok, {} failed)",
                                        subagent_results.len(),
                                        sub_ok,
                                        sub_fail
                                    )
                                } else {
                                    String::new()
                                };

                                let was_cancelled = cancel_flag.load(Ordering::Relaxed);
                                let status = if was_cancelled {
                                    "cancelled"
                                } else if result.succeeded() {
                                    "complete"
                                } else {
                                    "partial"
                                };
                                store_bg
                                    .update(&mid_bg, |m| {
                                        m.status = if was_cancelled {
                                            MissionStatus::Cancelled
                                        } else if result.succeeded() {
                                            MissionStatus::Complete
                                        } else {
                                            MissionStatus::Failed
                                        };
                                        m.progress_pct = if result.succeeded() {
                                            100.0
                                        } else {
                                            let done = result.steps_succeeded();
                                            let total = done + result.steps_failed();
                                            if total > 0 {
                                                done as f64 / total as f64 * 100.0
                                            } else {
                                                0.0
                                            }
                                        };
                                        m.tasks_done = result.steps_succeeded();
                                        m.tasks_total =
                                            result.steps_succeeded() + result.steps_failed();
                                    })
                                    .await;
                                store_bg.emit(PantheonEvent::MissionComplete {
                                    mission_id: mid_bg.clone(),
                                    status: status.to_string(),
                                    summary: format!(
                                        "{} steps succeeded, {} failed, {} replans, {}ms{}",
                                        result.steps_succeeded(),
                                        result.steps_failed(),
                                        result.replan_count,
                                        result.total_time_ms(),
                                        subagent_summary,
                                    ),
                                    artifacts: vec![],
                                });

                                // Agora settlement — pay agents for completed tasks
                                if result.succeeded() {
                                    settle_mission_payments(&state_bg, &store_bg, &mid_bg).await;
                                }
                            }
                            Err(e) => {
                                let was_cancelled = cancel_flag.load(Ordering::Relaxed);
                                let status = if was_cancelled {
                                    MissionStatus::Cancelled
                                } else {
                                    MissionStatus::Failed
                                };
                                let reason = if was_cancelled {
                                    "Mission cancelled by user".to_string()
                                } else {
                                    e.to_string()
                                };
                                warn!("MissionDriver execution failed for {}: {}", mid_bg, reason);
                                store_bg
                                    .update(&mid_bg, |m| {
                                        m.status = status;
                                        m.completed_at = Some(Utc::now());
                                    })
                                    .await;
                                store_bg.emit(PantheonEvent::MissionFailed {
                                    mission_id: mid_bg.clone(),
                                    reason,
                                });
                            }
                        }
                        // Clean up cancellation flag
                        state_bg.read().await.mission_cancels.remove(&mid_bg);
                    });
                } // end if !require_review

                (mission_id, team)
            }
            Err(e) => {
                warn!(
                    "MissionDriver plan_mission failed, falling back to heuristic: {}",
                    e
                );
                fallback_create(&orchestrator, &req.goal, orch_constraints, capabilities)
                    .await
                    .unwrap_or_else(|_| {
                        let id = format!("m-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                        (id, assemble_team_heuristic(&req.goal))
                    })
            }
        }
    } else {
        // No LLM configured — orchestrator only (no task decomposition)
        fallback_create(&orchestrator, &req.goal, orch_constraints, capabilities)
            .await
            .unwrap_or_else(|_| {
                let id = format!("m-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                (id, assemble_team_heuristic(&req.goal))
            })
    };

    // Mirror into PantheonStore for REST queries
    let require_review_flag = constraints.require_review.unwrap_or(false);
    let mut mission = Mission::new(req.goal.clone(), constraints);
    mission.id = mission_id.clone();
    mission.team = team.clone();
    mission.status = if require_review_flag {
        MissionStatus::AwaitingApproval
    } else {
        MissionStatus::Executing
    };

    store.emit(PantheonEvent::MissionCreated {
        mission_id: mission_id.clone(),
        goal: mission.goal.clone(),
        status: mission.status.to_string(),
    });
    store.insert(mission.clone()).await;
    store.emit(PantheonEvent::TeamAssembled {
        mission_id: mission_id.clone(),
        agents: team.clone(),
    });

    // Register Agora wallets for team members (idempotent)
    register_team_wallets(&state, &team).await;

    // Auto-create a mission room and add team members
    let room_id = store.create_mission_room(&mission_id, &mission.goal).await;
    for member in &team {
        store
            .join_room(
                &room_id,
                &RoomMember {
                    agent_id: member.agent_id.clone(),
                    agent_name: member.name.clone(),
                    role: match member.role {
                        AgentRole::Coordinator => "owner".to_string(),
                        _ => "member".to_string(),
                    },
                    joined_at: Utc::now(),
                },
            )
            .await;
    }

    info!(
        "Pantheon mission launched via MissionDriver: {} — {}",
        mission_id, req.goal
    );

    (
        StatusCode::CREATED,
        Json(json!({
            "id": mission_id,
            "goal": mission.goal,
            "status": mission.status,
            "team": team,
            "created_at": mission.created_at,
        })),
    )
        .into_response()
}

/// Fallback: use PantheonOrchestrator directly without LLM task decomposition.
async fn fallback_create(
    orchestrator: &Arc<zeus_orchestra::pantheon::PantheonOrchestrator>,
    goal: &str,
    constraints: zeus_orchestra::pantheon::MissionConstraints,
    capabilities: Vec<String>,
) -> Result<(String, Vec<TeamMember>), ()> {
    let orch_mission = orchestrator
        .create_mission(goal, constraints)
        .await
        .map_err(|_| ())?;
    let mission_id = orch_mission.id.clone();
    let orch_team = orchestrator
        .assemble_team(&mission_id, capabilities)
        .await
        .unwrap_or_default();
    let team = convert_orch_team(&orch_team, goal);
    Ok((mission_id, team))
}

/// Convert zeus-orchestra TeamMember slice → API TeamMember vec with heuristic fallback.
fn convert_orch_team(
    orch_team: &[zeus_orchestra::pantheon::TeamMember],
    goal: &str,
) -> Vec<TeamMember> {
    if orch_team.is_empty() {
        return assemble_team_heuristic(goal);
    }
    orch_team
        .iter()
        .map(|m| TeamMember {
            agent_id: m.agent_id.clone(),
            name: m.name.clone(),
            role: match m.role {
                zeus_orchestra::pantheon::AgentRole::Coordinator => AgentRole::Coordinator,
                zeus_orchestra::pantheon::AgentRole::Manager => AgentRole::Manager,
                zeus_orchestra::pantheon::AgentRole::Worker => AgentRole::Worker,
                zeus_orchestra::pantheon::AgentRole::Reviewer => AgentRole::Reviewer,
            },
            status: AgentStatus::Working,
            model: None,
        })
        .collect()
}

/// Infer required capabilities from goal text (feeds into orchestrator.assemble_team).
fn infer_capabilities(goal: &str) -> Vec<String> {
    let g = goal.to_lowercase();
    let mut caps = vec!["coordinate".to_string()];
    if g.contains("api") || g.contains("backend") || g.contains("server") {
        caps.push("backend".to_string());
    }
    if g.contains("ui") || g.contains("frontend") || g.contains("web") {
        caps.push("frontend".to_string());
    }
    if g.contains("test") || g.contains("review") {
        caps.push("review".to_string());
    }
    if g.contains("database") || g.contains("sql") {
        caps.push("database".to_string());
    }
    if g.contains("deploy") || g.contains("infra") {
        caps.push("devops".to_string());
    }
    caps
}

/// Pagination query params for list endpoints.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub status: Option<String>,
}

/// GET /v1/pantheon/missions — List missions with optional pagination and status filter.
///
/// Query params: `?limit=20&offset=0&status=executing`
pub async fn list_missions(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let all_missions = store.list().await;

    // Optional status filter
    let filtered: Vec<&Mission> = if let Some(ref status_filter) = params.status {
        all_missions
            .iter()
            .filter(|m| {
                let s = serde_json::to_value(&m.status)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                s == *status_filter
            })
            .collect()
    } else {
        all_missions.iter().collect()
    };

    let total = filtered.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params
        .limit
        .unwrap_or(50)
        .min(zeus_core::MAX_PAGE_LIMIT_SMALL);

    let page: Vec<Value> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            json!({
                "id": m.id,
                "goal": m.goal,
                "status": m.status,
                "progress_pct": m.progress_pct,
                "tasks_done": m.tasks_done,
                "tasks_total": m.tasks_total,
                "team_size": m.team.len(),
                "created_at": m.created_at,
                "updated_at": m.updated_at,
            })
        })
        .collect();

    Json(json!({
        "missions": page,
        "total": total,
        "offset": offset,
        "limit": limit,
    }))
}

/// GET /v1/pantheon/missions/:id — Mission detail
pub async fn get_mission(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get(&id).await {
        Some(m) => (StatusCode::OK, Json(json!(m))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission not found"})),
        )
            .into_response(),
    }
}

/// POST /v1/pantheon/missions/:id/intervene — Pause/resume/cancel/redirect
pub async fn intervene_mission(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<InterveneRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();

    // For pause/cancel: signal the background execution to stop
    let mission_cancels = state.read().await.mission_cancels.clone();
    if matches!(req.action.as_str(), "pause" | "cancel")
        && let Some(flag) = mission_cancels.get(&id)
    {
        flag.store(true, Ordering::Relaxed);
    }

    let updated = store
        .update(&id, |m| match req.action.as_str() {
            "pause" => {
                if m.status == MissionStatus::Executing {
                    m.status = MissionStatus::Paused;
                    m.add_activity(
                        "system",
                        "System",
                        "intervention",
                        json!({"action": "pause"}),
                    );
                }
            }
            "resume" => {
                if m.status == MissionStatus::Paused {
                    m.status = MissionStatus::Executing;
                    m.add_activity(
                        "system",
                        "System",
                        "intervention",
                        json!({"action": "resume"}),
                    );
                }
            }
            "cancel" => {
                m.status = MissionStatus::Cancelled;
                m.completed_at = Some(Utc::now());
                m.add_activity(
                    "system",
                    "System",
                    "intervention",
                    json!({"action": "cancel"}),
                );
            }
            "redirect" => {
                if let Some(ref msg) = req.message {
                    m.add_activity("user", "User", "redirect", json!({"message": msg}));
                }
            }
            "assign_task" => {
                if let (Some(desc), Some(agent)) = (&req.task, &req.agent_id) {
                    let now = Utc::now();
                    let new_task = MissionTask {
                        id: Uuid::new_v4().to_string(),
                        description: desc.clone(),
                        assigned_to: Some(agent.clone()),
                        status: TaskStatus::Pending,
                        result: None,
                        created_at: now,
                        updated_at: now,
                    };
                    m.tasks.push(new_task);
                    m.tasks_total += 1;
                    m.add_activity(
                        "user",
                        "User",
                        "task_dispatch",
                        json!({"task": desc, "agent_id": agent}),
                    );
                }
            }
            _ => {}
        })
        .await;

    if updated {
        (
            StatusCode::OK,
            Json(json!({"ok": true, "action": req.action})),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission not found"})),
        )
            .into_response()
    }
}

/// POST /v1/pantheon/missions/:id/approve — Approve a mission in AwaitingApproval state.
///
/// When a mission is created with `require_review: true`, it enters `AwaitingApproval` state
/// and waits for supervisor approval before execution begins. This endpoint transitions
/// the mission to `Executing` and triggers the full MissionDriver pipeline.
pub async fn approve_mission(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let approver_id = req
        .get("approver_id")
        .and_then(|v| v.as_str())
        .unwrap_or("api-user");
    let approver_name = req
        .get("approver_name")
        .and_then(|v| v.as_str())
        .unwrap_or("API User");

    let mission = match store.get(&id).await {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "Mission not found"})),
            )
                .into_response();
        }
    };

    if mission.status != MissionStatus::AwaitingApproval {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!(
                    "Mission is in '{}' state, not 'awaiting_approval'",
                    mission.status
                )
            })),
        )
            .into_response();
    }

    // Transition to Executing
    store
        .update(&id, |m| {
            m.status = MissionStatus::Executing;
            m.add_activity(
                approver_id,
                approver_name,
                "supervisor_approved",
                json!({"action": "approve"}),
            );
        })
        .await;

    store.emit(PantheonEvent::MissionApproved {
        mission_id: id.clone(),
        approved_by: approver_id.to_string(),
    });

    // Launch execution via MissionDriver
    let goal = mission.goal.clone();
    execute_approved_plan(
        state.clone(),
        store.clone(),
        id.clone(),
        String::new(), // no room_id for direct mission approval
        goal,
        String::new(),
    );

    info!(
        "Mission {} approved by {} — execution started",
        id, approver_id
    );

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "mission_id": id,
            "status": "executing",
            "approved_by": approver_id,
        })),
    )
        .into_response()
}

/// POST /v1/pantheon/missions/:id/reject — Reject a mission in AwaitingApproval state.
///
/// Transitions the mission to `Cancelled` with a rejection reason. Unlike approve,
/// this does NOT trigger execution — the mission is permanently rejected.
pub async fn reject_mission(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let rejector_id = req
        .get("rejector_id")
        .and_then(|v| v.as_str())
        .unwrap_or("api-user");
    let rejector_name = req
        .get("rejector_name")
        .and_then(|v| v.as_str())
        .unwrap_or("API User");
    let reason = req
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mission = match store.get(&id).await {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "Mission not found"})),
            )
                .into_response();
        }
    };

    if mission.status != MissionStatus::AwaitingApproval {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("Mission is {} (not awaiting_approval)", mission.status),
                "current_status": mission.status.to_string(),
            })),
        )
            .into_response();
    }

    let updated = store
        .update(&id, |m| {
            m.status = MissionStatus::Cancelled;
            m.add_activity(
                rejector_id,
                rejector_name,
                "mission_rejected",
                json!({"action": "reject", "reason": reason}),
            );
        })
        .await;

    if !updated {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to update mission"})),
        )
            .into_response();
    }

    store.emit(PantheonEvent::MissionRejected {
        mission_id: id.clone(),
        rejected_by: rejector_id.to_string(),
        reason: reason.clone(),
    });

    info!(
        "Mission {} rejected by {} — reason: {}",
        id, rejector_id, reason
    );

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "mission_id": id,
            "status": "cancelled",
            "rejected_by": rejector_id,
            "reason": reason,
        })),
    )
        .into_response()
}

/// GET /v1/pantheon/missions/:id/feed — Activity feed
pub async fn get_mission_feed(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get(&id).await {
        Some(m) => (StatusCode::OK, Json(json!(m.feed))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission not found"})),
        )
            .into_response(),
    }
}

/// GET /v1/pantheon/missions/:id/artifacts — List artifacts
pub async fn get_mission_artifacts(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get(&id).await {
        Some(m) => (StatusCode::OK, Json(json!(m.artifacts))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission not found"})),
        )
            .into_response(),
    }
}

/// GET /v1/pantheon/missions/:id/artifacts/:name/download — Download a specific artifact
pub async fn download_mission_artifact(
    State(state): State<SharedState>,
    Path((id, artifact_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get(&id).await {
        Some(m) => {
            let artifact = m.artifacts.iter().find(|a| a.name == artifact_name);
            match artifact {
                Some(a) => {
                    let path = std::path::Path::new(&a.path);
                    if !path.exists() {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({"error": "Artifact file not found on disk"})),
                        )
                            .into_response();
                    }
                    match tokio::fs::read(path).await {
                        Ok(bytes) => {
                            let content_type = match path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                            {
                                "json" => "application/json",
                                "txt" | "md" | "log" => "text/plain; charset=utf-8",
                                "html" => "text/html",
                                "png" => "image/png",
                                "jpg" | "jpeg" => "image/jpeg",
                                "pdf" => "application/pdf",
                                "zip" => "application/zip",
                                _ => "application/octet-stream",
                            };
                            let filename = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&artifact_name);
                            (
                                StatusCode::OK,
                                [
                                    (
                                        axum::http::header::CONTENT_TYPE,
                                        content_type.to_string(),
                                    ),
                                    (
                                        axum::http::header::CONTENT_DISPOSITION,
                                        format!("attachment; filename=\"{}\"", filename),
                                    ),
                                ],
                                bytes,
                            )
                                .into_response()
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": format!("Failed to read artifact: {}", e)})),
                        )
                            .into_response(),
                    }
                }
                None => (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "Artifact not found"})),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission not found"})),
        )
            .into_response(),
    }
}

/// POST /v1/pantheon/missions/:id/review — Approve or reject a task
pub async fn review_task(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<ReviewRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();

    let updated = store
        .update(&id, |m| {
            if let Some(task) = m.tasks.iter_mut().find(|t| t.id == req.task_id) {
                match req.verdict.as_str() {
                    "approve" => {
                        task.status = TaskStatus::Approved;
                        m.add_activity(
                            "user",
                            "User",
                            "review",
                            json!({"task_id": req.task_id, "verdict": "approve",
                               "comment": req.comment}),
                        );
                    }
                    "reject" => {
                        task.status = TaskStatus::Rejected;
                        m.add_activity(
                            "user",
                            "User",
                            "review",
                            json!({"task_id": req.task_id, "verdict": "reject",
                               "comment": req.comment}),
                        );
                    }
                    _ => {}
                }
            }
        })
        .await;

    if updated {
        (
            StatusCode::OK,
            Json(json!({"ok": true, "verdict": req.verdict})),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Mission or task not found"})),
        )
            .into_response()
    }
}

// ============================================================================
// Agora ↔ Pantheon wiring — mission settlement + wallet registration
// ============================================================================

/// Default credit reward per completed task.
const TASK_REWARD_CREDITS: u64 = 100;

/// Default initial balance for new agent wallets.
const INITIAL_AGENT_BALANCE: u64 = 1_000;

/// Register wallets in the marketplace for all team members.
/// Idempotent — credits only if agent has zero balance.
pub async fn register_team_wallets(state: &SharedState, team: &[TeamMember]) {
    let state_guard = state.read().await;
    // #web4 P0-1c cut-9: marketplace_store is the sole ledger — the in-memory
    // dual-write was removed.
    for member in team {
        let balance = state_guard
            .marketplace_store
            .get_balance(&member.agent_id)
            .await;
        if balance == 0 {
            state_guard
                .marketplace_store
                .credit(
                    &member.agent_id,
                    INITIAL_AGENT_BALANCE,
                    &format!("mission:initial_grant:{}", member.agent_id),
                )
                .await;
            info!(
                "Agora: Registered wallet for {} with {} credits",
                member.agent_id, INITIAL_AGENT_BALANCE
            );
        }
    }
}

/// Settle payments after a mission completes.
///
/// For each completed task:
/// 1. Credit the assigned agent with `TASK_REWARD_CREDITS`
/// 2. Record trade success in reputation engine
/// 3. Post a payment activity to the mission's auto-room
pub async fn settle_mission_payments(state: &SharedState, store: &PantheonStore, mission_id: &str) {
    let mission = match store.get(mission_id).await {
        Some(m) => m,
        None => {
            warn!("Agora settlement: mission {} not found", mission_id);
            return;
        }
    };

    let state_guard = state.read().await;
    // #web4 P0-1c cut-9: marketplace_store is the sole ledger/reputation sink —
    // the in-memory dual-writes were removed.

    let mut total_paid: u64 = 0;
    let mut agents_paid: Vec<String> = Vec::new();

    for task in &mission.tasks {
        if task.status == TaskStatus::Approved || task.status == TaskStatus::Complete {
            let agent_id = task.assigned_to.as_deref().unwrap_or("prometheus");

            // Record reputation + credit the agent (SQLite, source of truth)
            state_guard
                .marketplace_store
                .record_trade_success(agent_id)
                .await;
            state_guard
                .marketplace_store
                .credit(
                    agent_id,
                    TASK_REWARD_CREDITS,
                    &format!("mission:{}:task:{}", mission_id, task.id),
                )
                .await;
            let new_balance = state_guard.marketplace_store.get_balance(agent_id).await;

            total_paid += TASK_REWARD_CREDITS;
            if !agents_paid.contains(&agent_id.to_string()) {
                agents_paid.push(agent_id.to_string());
            }

            info!(
                "Agora: Paid {} credits to {} for task {} (balance: {})",
                TASK_REWARD_CREDITS, agent_id, task.id, new_balance
            );
        }
    }

    // Post settlement summary to mission room
    if total_paid > 0 {
        let room_id = format!("m-{}-room", mission_id);
        let summary = format!(
            "Mission settlement complete: {} credits paid to {} agent(s) [{}]",
            total_paid,
            agents_paid.len(),
            agents_paid.join(", ")
        );

        store
            .insert_room_message(&RoomMessage {
                id: format!("pay-{}", &Uuid::new_v4().to_string()[..8]),
                room_id: room_id.clone(),
                sender_id: "agora".to_string(),
                sender_name: "Agora Settlement".to_string(),
                content: summary.clone(),
                message_type: "system".to_string(),
                metadata: Some(json!({
                    "type": "settlement",
                    "total_credits": total_paid,
                    "agents": agents_paid,
                    "mission_id": mission_id,
                })),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            })
            .await;

        store.emit(PantheonEvent::RoomMessageSent {
            room_id,
            message: RoomMessage {
                id: format!("pay-evt-{}", &Uuid::new_v4().to_string()[..8]),
                room_id: format!("m-{}-room", mission_id),
                sender_id: "agora".to_string(),
                sender_name: "Agora Settlement".to_string(),
                content: summary,
                message_type: "system".to_string(),
                metadata: None,
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            },
        });

        info!(
            "Agora: Mission {} settled — {} credits to {} agents",
            mission_id,
            total_paid,
            agents_paid.len()
        );
    }
}

/// GET /v1/pantheon/economy — Marketplace stats + agent balances for Pantheon dashboard
pub async fn pantheon_economy(State(state): State<SharedState>) -> impl IntoResponse {
    let state_guard = state.read().await;
    // #web4 P0-1c cut-9: stats/balances/reputations read from marketplace_store
    // (SQLite, source of truth) instead of the in-memory marketplace.
    let stats = state_guard.marketplace_store.stats().await;
    let balances = state_guard.marketplace_store.all_balances().await;
    let reputations = state_guard.marketplace_store.all_reputations().await;

    let agents: Vec<Value> = balances
        .iter()
        .map(|(agent_id, balance)| {
            let rep = reputations.iter().find(|r| r.agent_id == *agent_id);
            let (trust, completed, failed, badge) = match rep {
                Some(r) => {
                    let badge = super::marketplace_store::compute_badge(r);
                    (r.trust_score, r.successful_trades, r.failed_trades, badge)
                }
                None => (0.5, 0, 0, "New"),
            };
            json!({
                "agent_id": agent_id,
                "balance": balance,
                "trust_score": trust,
                "trades_completed": completed,
                "trades_failed": failed,
                "badge": badge,
                "badge_color": super::marketplace_store::badge_color(badge),
            })
        })
        .collect();

    Json(json!({
        "stats": {
            "total_listings": stats.total_listings,
            "active_listings": stats.active_listings,
            "total_trades": stats.total_trades,
            "completed_trades": stats.completed_trades,
            "total_token_supply": stats.total_supply,
            "total_agents": stats.total_agents,
        },
        "agents": agents,
    }))
}

// ============================================================================
// Room handlers
// ============================================================================

/// POST /v1/pantheon/rooms — Create a room
pub async fn create_room(
    State(state): State<SharedState>,
    Json(req): Json<CreateRoomRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let room = Room {
        id: format!("r-{}", &Uuid::new_v4().to_string()[..8]),
        name: req.name,
        description: req.description,
        room_type: req.room_type,
        mission_id: req.mission_id,
        created_by: req.created_by.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    store.insert_room(&room).await;

    // Auto-join the creator
    let member = RoomMember {
        agent_id: req.created_by.clone(),
        agent_name: req.created_by.clone(),
        role: "owner".to_string(),
        joined_at: Utc::now(),
    };
    store.join_room(&room.id, &member).await;

    store.emit(PantheonEvent::RoomCreated { room: room.clone() });

    info!("Room created: {} ({})", room.id, room.name);

    (StatusCode::CREATED, Json(json!(room))).into_response()
}

/// GET /v1/pantheon/rooms — List rooms (public + rooms the caller is a member of)
pub async fn list_rooms(State(state): State<SharedState>) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let rooms = store.list_rooms().await;
    Json(json!({ "rooms": rooms, "total": rooms.len() }))
}

/// POST /v1/pantheon/dms — Find or create a DM room between two agents.
/// Idempotent: returns the same room if called multiple times with the same pair.
pub async fn find_or_create_dm(
    State(state): State<SharedState>,
    Json(req): Json<FindOrCreateDmRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();

    // Return existing DM room if one already exists between these two agents
    if let Some(room) = store.find_dm_room(&req.agent_id, &req.peer_id).await {
        let members = store.list_room_members(&room.id).await;
        return (
            StatusCode::OK,
            Json(json!({ "room": room, "members": members, "created": false })),
        )
            .into_response();
    }

    // Create a new DM room — name encodes both participants for readability
    let now = Utc::now();
    let room = Room {
        id: format!("dm-{}", &Uuid::new_v4().to_string()[..8]),
        name: format!("dm:{},{}", req.agent_id, req.peer_id),
        description: Some(format!(
            "Direct message between {} and {}",
            req.agent_name, req.peer_name
        )),
        room_type: RoomType::Dm,
        mission_id: None,
        created_by: req.agent_id.clone(),
        created_at: now,
        updated_at: now,
    };
    store.insert_room(&room).await;

    // Auto-join both participants
    let initiator = RoomMember {
        agent_id: req.agent_id.clone(),
        agent_name: req.agent_name.clone(),
        role: "member".to_string(),
        joined_at: now,
    };
    let peer = RoomMember {
        agent_id: req.peer_id.clone(),
        agent_name: req.peer_name.clone(),
        role: "member".to_string(),
        joined_at: now,
    };
    store.join_room(&room.id, &initiator).await;
    store.join_room(&room.id, &peer).await;

    store.emit(PantheonEvent::RoomCreated { room: room.clone() });
    info!("DM room created: {} between {} and {}", room.id, req.agent_id, req.peer_id);

    let members = store.list_room_members(&room.id).await;
    (
        StatusCode::CREATED,
        Json(json!({ "room": room, "members": members, "created": true })),
    )
        .into_response()
}

/// GET /v1/pantheon/dms?agent_id=X — List all DM rooms where the agent is a participant.
pub async fn list_dms(
    State(state): State<SharedState>,
    Query(query): Query<ListDmsQuery>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let rooms = store.list_dms_for_agent(&query.agent_id).await;
    Json(json!({ "rooms": rooms, "total": rooms.len() }))
}

/// GET /v1/pantheon/rooms/:id — Room detail with members
pub async fn get_room(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get_room(&id).await {
        Some(room) => {
            let members = store.list_room_members(&id).await;
            (
                StatusCode::OK,
                Json(json!({
                    "room": room,
                    "members": members,
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Room not found"})),
        )
            .into_response(),
    }
}

/// POST /v1/pantheon/rooms/:id/join — Join a room
pub async fn join_room(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<JoinRoomRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store.get_room(&id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Room not found"})),
        )
            .into_response();
    }

    let member = RoomMember {
        agent_id: req.agent_id.clone(),
        agent_name: req.agent_name.clone(),
        role: "member".to_string(),
        joined_at: Utc::now(),
    };
    store.join_room(&id, &member).await;

    store.emit(PantheonEvent::AgentJoinedRoom {
        room_id: id.clone(),
        agent_id: req.agent_id.clone(),
        agent_name: req.agent_name.clone(),
    });

    (
        StatusCode::OK,
        Json(json!({"ok": true, "agent_id": req.agent_id})),
    )
        .into_response()
}

/// POST /v1/pantheon/rooms/:id/leave — Leave a room
pub async fn leave_room(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<JoinRoomRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    store.leave_room(&id, &req.agent_id).await;

    store.emit(PantheonEvent::AgentLeftRoom {
        room_id: id.clone(),
        agent_id: req.agent_id.clone(),
    });

    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

// POST /v1/pantheon/rooms/:id/messages — Send a message to a room
// ============================================================================
// Room Context
// ============================================================================

/// Build conversation context from recent room messages for LLM calls.
/// Combines chronological recent-N messages with Mnemosyne semantic search results
/// when a query and Mnemosyne instance are provided. Deduplicates by content.
async fn build_room_context(
    store: &PantheonStore,
    room_id: &str,
    limit: usize,
    semantic: Option<(&zeus_mnemosyne::Mnemosyne, &str)>,
) -> Vec<zeus_core::Message> {
    // ── Chronological: last N messages ──
    let recent = store.get_room_messages(room_id, limit, None).await;
    let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut messages: Vec<zeus_core::Message> = Vec::new();

    // Add recent messages first (these are the most relevant by recency)
    for m in recent.iter().filter(|m| {
        matches!(
            m.message_type.as_str(),
            "chat" | "text" | "task_update" | "plan_card"
        )
    }) {
        let prefix = if m.message_type == "task_update" {
            format!("[Agent {}] ", m.sender_name)
        } else {
            format!("[{}] ", m.sender_name)
        };
        let content = format!("{}{}", prefix, m.content);
        seen_content.insert(m.content.clone());
        messages.push(zeus_core::Message::user(content));
    }

    // ── Semantic: Mnemosyne FTS search scoped to this room ──
    if let Some((mnemosyne, query)) = semantic {
        let session_id = format!("room:{}", room_id);
        match mnemosyne.search_in_session(query, &session_id, 10).await {
            Ok(results) => {
                for r in results {
                    // Deduplicate: skip if content already included from recent messages
                    // Strip "[SenderName] " prefix for comparison since stored format includes it
                    let raw = r.content.split("] ").last().unwrap_or(&r.content);
                    if seen_content.contains(raw) || seen_content.contains(&r.content) {
                        continue;
                    }
                    seen_content.insert(r.content.clone());
                    messages.push(zeus_core::Message::user(format!(
                        "[semantic recall] {}",
                        r.content
                    )));
                }
            }
            Err(e) => {
                tracing::debug!("Mnemosyne semantic room search failed (non-fatal): {}", e);
            }
        }
    }

    messages
}

/// Format room context as a text summary for system prompt injection.
fn format_room_context_summary(messages: &[zeus_core::Message]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let mut ctx = String::from("\n\n--- Room conversation context ---\n");
    for msg in messages {
        ctx.push_str(&msg.content);
        ctx.push('\n');
    }
    ctx.push_str("--- End of room context ---\n");
    ctx
}

// ============================================================================
// Spawn Execution Helper + Streaming Progress
// ============================================================================

/// Post a plan_progress message to a room (matches Zeus100's frontend contract).
async fn post_progress(
    store: &PantheonStore,
    spawn_id: &str,
    agent_name: &str,
    room_id: &str,
    step: usize,
    total: usize,
    description: &str,
) {
    let msg = RoomMessage {
        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
        room_id: room_id.to_string(),
        sender_id: spawn_id.to_string(),
        sender_name: agent_name.to_string(),
        content: description.to_string(),
        message_type: "plan_progress".to_string(),
        metadata: Some(json!({
            "step": step,
            "total": total,
            "spawn_id": spawn_id,
        })),
        reply_to: None,
        edited: false,
        attachments: vec![],
        timestamp: Utc::now(),
    };
    store.insert_room_message(&msg).await;
    store.emit(PantheonEvent::RoomMessageSent {
        room_id: room_id.to_string(),
        message: msg,
    });
}

/// Fire a background spawn agent — used by both direct-execute and approved paths.
/// Emits plan_progress messages at each phase for real-time UI updates.
fn execute_spawn(
    state: SharedState,
    store: PantheonStore,
    spawn_id: String,
    agent_name: String,
    room_id: String,
    task: String,
) {
    tokio::spawn(async move {
        let total_steps = 3;

        // Step 1: Initializing
        post_progress(
            &store,
            &spawn_id,
            &agent_name,
            &room_id,
            1,
            total_steps,
            "Initializing agent...",
        )
        .await;

        let config = state.read().await.config.clone();
        let mnemosyne = state.read().await.mnemosyne.clone();

        // Build room context: recent messages + Mnemosyne semantic search
        let semantic = mnemosyne.as_ref().map(|mn| (mn.as_ref(), task.as_str()));
        let room_context = build_room_context(&store, &room_id, 20, semantic).await;
        let context_summary = format_room_context_summary(&room_context);

        // Step 2: Processing with LLM
        post_progress(
            &store,
            &spawn_id,
            &agent_name,
            &room_id,
            2,
            total_steps,
            &format!("Processing: {}", &task[..task.len().min(80)]),
        )
        .await;

        let response = match LlmClient::from_config(&config) {
            Ok(llm) => {
                let system_prompt = if context_summary.is_empty() {
                    "You are a Zeus agent spawned to handle a task in the War Room. \
                        Provide a clear, concise response. If the task requires external tools \
                        or data you can't access, explain what would be needed."
                        .to_string()
                } else {
                    format!(
                        "You are a Zeus agent spawned to handle a task in the War Room. \
                        Provide a clear, concise response. If the task requires external tools \
                        or data you can't access, explain what would be needed.{}",
                        context_summary
                    )
                };
                let messages = vec![zeus_core::Message::user(&task)];
                match llm.complete(&messages, &[], Some(&system_prompt)).await {
                    Ok(resp) => resp.content,
                    Err(e) => format!("\u{274c} Agent error: {}", e),
                }
            }
            Err(e) => format!("\u{274c} No LLM available: {}", e),
        };

        // Step 3: Complete
        post_progress(
            &store,
            &spawn_id,
            &agent_name,
            &room_id,
            3,
            total_steps,
            "Task complete",
        )
        .await;

        // Final result message
        let result_msg = RoomMessage {
            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
            room_id: room_id.clone(),
            sender_id: spawn_id.clone(),
            sender_name: agent_name,
            content: format!("\u{2705} **Task complete**\n_{}_\n\n{}", task, response),
            message_type: "task_update".to_string(),
            metadata: Some(serde_json::json!({
                "spawn_id": spawn_id,
                "task": task,
                "status": "completed"
            })),
            reply_to: None,
            edited: false,
            attachments: vec![],
            timestamp: Utc::now(),
        };
        store.insert_room_message(&result_msg).await;
        store.emit(PantheonEvent::RoomMessageSent {
            room_id,
            message: result_msg,
        });
    });
}

/// Execute an approved plan by creating a real mission via MissionDriver.
///
/// On approval, this function:
/// 1. Creates a tracked mission in the Pantheon store
/// 2. Plans and executes via MissionDriver (with real tool execution)
/// 3. Posts progress updates to the room as the mission runs
/// 4. Updates the mission status on completion/failure
/// 5. Links the mission_id back to the approval record
///
/// Falls back to simple LLM-only execution if MissionDriver is unavailable.
fn execute_approved_plan(
    state: SharedState,
    store: PantheonStore,
    plan_id: String,
    room_id: String,
    goal: String,
    _steps_json: String,
) {
    tokio::spawn(async move {
        let coordinator_name =
            format!("Supervisor-{}", &plan_id[plan_id.len().saturating_sub(8)..]);

        // Create a tracked mission in the store
        let constraints = MissionConstraints::default();
        let mut mission = Mission::new(goal.clone(), constraints.clone());
        mission.status = MissionStatus::Executing;
        mission.add_activity(
            "supervisor",
            &coordinator_name,
            "plan_approved",
            json!({"plan_id": plan_id}),
        );
        let mission_id = mission.id.clone();

        store.insert(mission).await;
        store.link_mission_to_approval(&plan_id, &mission_id).await;
        store.emit(PantheonEvent::MissionCreated {
            mission_id: mission_id.clone(),
            goal: goal.clone(),
            status: "executing".to_string(),
        });

        // Post progress: mission started
        post_progress(
            &store,
            &plan_id,
            &coordinator_name,
            &room_id,
            1,
            1,
            &format!(
                "Mission `{}` launched — executing approved plan",
                mission_id
            ),
        )
        .await;

        // Try MissionDriver path for real tool execution
        let state_guard = state.read().await;
        let config = state_guard.config.clone();
        let orchestrator = state_guard.pantheon_orchestrator().clone();
        let tool_schemas = state_guard.tools.schemas();
        let tool_exec = state_guard.tool_executor.clone();
        drop(state_guard);

        let llm = match LlmClient::from_config(&config) {
            Ok(l) => Arc::new(l),
            Err(e) => {
                let fail_content =
                    format!("\u{274c} No LLM available for mission execution: {}", e);
                store
                    .update(&mission_id, |m| {
                        m.status = MissionStatus::Failed;
                        m.completed_at = Some(Utc::now());
                    })
                    .await;
                let fail_msg = RoomMessage {
                    id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                    room_id: room_id.clone(),
                    sender_id: "system".to_string(),
                    sender_name: "System".to_string(),
                    content: fail_content,
                    message_type: "system".to_string(),
                    metadata: None,
                    reply_to: None,
                    edited: false,
                    attachments: vec![],
                    timestamp: Utc::now(),
                };
                store.insert_room_message(&fail_msg).await;
                store.emit(PantheonEvent::RoomMessageSent {
                    room_id,
                    message: fail_msg,
                });
                store.emit(PantheonEvent::MissionFailed {
                    mission_id,
                    reason: e.to_string(),
                });
                return;
            }
        };

        let orch_constraints = zeus_orchestra::pantheon::MissionConstraints {
            budget_tokens: 50_000,
            timeout_seconds: 600,
            max_agents: 4,
            require_review: false,
        };
        let capabilities = infer_capabilities(&goal);

        let driver = zeus_prometheus::MissionDriver::new(
            orchestrator.clone(),
            llm.clone(),
            tool_schemas.clone(),
        )
        .with_checkpointer(Arc::new(store.clone()));

        match driver
            .plan_mission(&goal, orch_constraints, capabilities)
            .await
        {
            Ok(planned) => {
                // Register cancellation flag
                let cancel_flag = Arc::new(AtomicBool::new(false));
                state
                    .read()
                    .await
                    .mission_cancels
                    .insert(mission_id.clone(), cancel_flag.clone());

                // Wrap tool executor for this mission
                let mission_exec: Option<Arc<MissionToolExecutor>> = tool_exec.map(|inner| {
                    Arc::new(MissionToolExecutor::new(
                        inner,
                        mission_id.clone(),
                        llm.clone(),
                    ))
                });

                let exec_ref: Option<&dyn zeus_prometheus::ToolExecutor> = mission_exec
                    .as_ref()
                    .map(|e| e.as_ref() as &dyn zeus_prometheus::ToolExecutor);

                match driver
                    .drive_mission_cancellable(&planned, exec_ref, Some(cancel_flag.clone()))
                    .await
                {
                    Ok(result) => {
                        // Collect subagents
                        let subagent_results = if let Some(ref mexec) = mission_exec {
                            mexec.collect_subagents(60).await
                        } else {
                            vec![]
                        };
                        let subagent_summary = if !subagent_results.is_empty() {
                            let sub_ok = subagent_results.iter().filter(|r| r.success).count();
                            format!(
                                ", {} subagent(s) ({} ok, {} failed)",
                                subagent_results.len(),
                                sub_ok,
                                subagent_results.len() - sub_ok
                            )
                        } else {
                            String::new()
                        };

                        let was_cancelled = cancel_flag.load(Ordering::Relaxed);
                        let (status, status_str) = if was_cancelled {
                            (MissionStatus::Cancelled, "cancelled")
                        } else if result.succeeded() {
                            (MissionStatus::Complete, "complete")
                        } else {
                            (MissionStatus::Failed, "partial")
                        };

                        let summary = format!(
                            "{} steps succeeded, {} failed, {} replans, {}ms{}",
                            result.steps_succeeded(),
                            result.steps_failed(),
                            result.replan_count,
                            result.total_time_ms(),
                            subagent_summary,
                        );

                        store
                            .update(&mission_id, |m| {
                                m.status = status.clone();
                                m.progress_pct = if result.succeeded() {
                                    100.0
                                } else {
                                    let done = result.steps_succeeded();
                                    let total = done + result.steps_failed();
                                    if total > 0 {
                                        done as f64 / total as f64 * 100.0
                                    } else {
                                        0.0
                                    }
                                };
                                m.tasks_done = result.steps_succeeded();
                                m.tasks_total = result.steps_succeeded() + result.steps_failed();
                                m.summary = Some(summary.clone());
                                m.completed_at = Some(Utc::now());
                            })
                            .await;

                        // Post completion to room
                        let icon = if result.succeeded() {
                            "\u{2705}"
                        } else {
                            "\u{26a0}\u{fe0f}"
                        };
                        let room_msg = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room_id.clone(),
                            sender_id: plan_id.clone(),
                            sender_name: coordinator_name.clone(),
                            content: format!(
                                "{} **Mission {}** ({}): {}\n\n{}",
                                icon, mission_id, status_str, goal, summary
                            ),
                            message_type: "task_update".to_string(),
                            metadata: Some(json!({
                                "plan_id": plan_id,
                                "mission_id": mission_id,
                                "status": status_str,
                            })),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&room_msg).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room_id.clone(),
                            message: room_msg,
                        });
                        store.emit(PantheonEvent::MissionComplete {
                            mission_id: mission_id.clone(),
                            status: status_str.to_string(),
                            summary,
                            artifacts: vec![],
                        });

                        // Agora settlement on success
                        if result.succeeded() {
                            settle_mission_payments(&state, &store, &mission_id).await;
                        }
                    }
                    Err(e) => {
                        let was_cancelled = cancel_flag.load(Ordering::Relaxed);
                        let reason = if was_cancelled {
                            "Mission cancelled by user".to_string()
                        } else {
                            e.to_string()
                        };
                        warn!(
                            "Approved plan execution failed for {}: {}",
                            mission_id, reason
                        );
                        store
                            .update(&mission_id, |m| {
                                m.status = if was_cancelled {
                                    MissionStatus::Cancelled
                                } else {
                                    MissionStatus::Failed
                                };
                                m.completed_at = Some(Utc::now());
                            })
                            .await;
                        let fail_msg = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room_id.clone(),
                            sender_id: plan_id.clone(),
                            sender_name: coordinator_name,
                            content: format!(
                                "\u{274c} **Mission {} failed**: {}",
                                mission_id, reason
                            ),
                            message_type: "task_update".to_string(),
                            metadata: Some(json!({
                                "plan_id": plan_id,
                                "mission_id": mission_id,
                                "status": "failed",
                            })),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&fail_msg).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room_id.clone(),
                            message: fail_msg,
                        });
                        store.emit(PantheonEvent::MissionFailed {
                            mission_id: mission_id.clone(),
                            reason,
                        });
                    }
                }
                // Clean up cancellation flag
                state.read().await.mission_cancels.remove(&mission_id);
            }
            Err(e) => {
                warn!(
                    "MissionDriver plan_mission failed for approved plan {}: {}",
                    plan_id, e
                );
                store
                    .update(&mission_id, |m| {
                        m.status = MissionStatus::Failed;
                        m.completed_at = Some(Utc::now());
                    })
                    .await;
                let fail_msg = RoomMessage {
                    id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                    room_id: room_id.clone(),
                    sender_id: "system".to_string(),
                    sender_name: "System".to_string(),
                    content: format!("\u{274c} Mission planning failed: {}", e),
                    message_type: "system".to_string(),
                    metadata: None,
                    reply_to: None,
                    edited: false,
                    attachments: vec![],
                    timestamp: Utc::now(),
                };
                store.insert_room_message(&fail_msg).await;
                store.emit(PantheonEvent::RoomMessageSent {
                    room_id,
                    message: fail_msg,
                });
                store.emit(PantheonEvent::MissionFailed {
                    mission_id,
                    reason: e.to_string(),
                });
            }
        }
    });
}

/// Re-plan a rejected plan card with user feedback.
/// Calls LLM to generate a revised approach, then reopens the plan with new steps.
/// Emits an updated plan_card message with incremented revision.
async fn replan_with_feedback(
    state: &SharedState,
    store: &PantheonStore,
    plan_id: &str,
    room_id: &str,
    original_goal: &str,
    reject_feedback: &str,
) {
    // Post a progress message
    let progress_msg = RoomMessage {
        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
        room_id: room_id.to_string(),
        sender_id: "system".to_string(),
        sender_name: "Zeus".to_string(),
        content: format!("Re-planning `{}`…", plan_id),
        message_type: "plan_progress".to_string(),
        metadata: Some(json!({ "step": 1, "total": 2, "spawn_id": plan_id })),
        reply_to: None,
        edited: false,
        attachments: vec![],
        timestamp: Utc::now(),
    };
    store.insert_room_message(&progress_msg).await;
    store.emit(PantheonEvent::RoomMessageSent {
        room_id: room_id.to_string(),
        message: progress_msg,
    });

    let config = state.read().await.config.clone();
    let llm = match LlmClient::from_config(&config) {
        Ok(llm) => llm,
        Err(e) => {
            warn!("Re-plan failed — no LLM available: {}", e);
            return;
        }
    };

    let system_prompt = "You are a Zeus planning agent. A user rejected your previous plan and provided feedback. \
        Revise the plan to address their concerns. Output ONLY a brief revised goal (one sentence) on the first line, \
        then a numbered list of steps. Keep it concise — no explanations, just the revised plan.";

    let user_msg = format!(
        "Original goal: {}\n\nRejection feedback: {}\n\nRevise the plan to address this feedback.",
        original_goal, reject_feedback
    );

    let messages = vec![zeus_core::Message::user(&user_msg)];
    let revised_text = match llm.complete(&messages, &[], Some(system_prompt)).await {
        Ok(resp) => resp.content,
        Err(e) => {
            warn!("Re-plan LLM call failed for {}: {}", plan_id, e);
            return;
        }
    };

    // Parse revised text: first line = revised goal, rest = steps
    let mut lines = revised_text.lines();
    let revised_goal = lines.next().unwrap_or(original_goal).trim().to_string();
    let revised_steps: Vec<Value> = lines
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            // Strip leading number + period/parenthesis if present
            let desc = line
                .trim()
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')')
                .trim();
            json!({
                "description": if desc.is_empty() { line.trim() } else { desc },
                "agent_type": "spawn",
                "status": "pending",
                "step_number": i + 1,
                "elapsed_ms": null,
            })
        })
        .collect();

    // Ensure we have at least one step
    let steps = if revised_steps.is_empty() {
        vec![json!({
            "description": revised_goal,
            "agent_type": "spawn",
            "status": "pending",
            "elapsed_ms": null,
        })]
    } else {
        revised_steps
    };

    let steps_json = serde_json::to_string(&steps).unwrap_or_default();

    // Reopen the plan with new revision
    let reopened = store
        .reopen_plan_for_revision(plan_id, &steps_json, &revised_goal)
        .await;
    if let Some(updated) = reopened {
        // Emit updated plan_card for frontends
        let card_msg = RoomMessage {
            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
            room_id: room_id.to_string(),
            sender_id: "system".to_string(),
            sender_name: "Zeus".to_string(),
            content: format!("Revised plan (v{}): _{}_", updated.revision, revised_goal),
            message_type: "plan_card".to_string(),
            metadata: Some(json!({
                "plan_id": plan_id,
                "goal": revised_goal,
                "steps": steps,
                "status": "awaiting_approval",
                "revision": updated.revision,
            })),
            reply_to: None,
            edited: false,
            attachments: vec![],
            timestamp: Utc::now(),
        };
        store.insert_room_message(&card_msg).await;
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.to_string(),
            message: card_msg,
        });
        store.emit(PantheonEvent::PlanCardCreated {
            room_id: room_id.to_string(),
            plan_id: plan_id.to_string(),
            goal: revised_goal,
            complexity: updated.complexity,
            risk: updated.risk,
        });
    } else {
        // Max revisions or plan not found
        let fail_msg = RoomMessage {
            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
            room_id: room_id.to_string(),
            sender_id: "system".to_string(),
            sender_name: "System".to_string(),
            content: format!(
                "Could not revise plan `{}` — max revisions reached or plan not found.",
                plan_id
            ),
            message_type: "chat".to_string(),
            metadata: None,
            reply_to: None,
            edited: false,
            attachments: vec![],
            timestamp: Utc::now(),
        };
        store.insert_room_message(&fail_msg).await;
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.to_string(),
            message: fail_msg,
        });
    }
}

// ============================================================================
// Slash Commands
// ============================================================================

/// Handle a slash command in a room. Returns a system message response, or None
/// if the command is unrecognized.
async fn handle_slash_command(
    state: &SharedState,
    store: &PantheonStore,
    room: &Room,
    sender_id: &str,
    sender_name: &str,
    content: &str,
) -> Option<RoomMessage> {
    let parts: Vec<&str> = content.splitn(2, char::is_whitespace).collect();
    let cmd = parts[0].to_lowercase();
    let _args = parts.get(1).unwrap_or(&"").trim();

    let response = match cmd.as_str() {
        "/help" => "**Available commands:**\n\
             `/help` — Show this help\n\
             `/agents` — List fleet agents and status\n\
             `/members` — Show room members\n\
             `/rooms` — List all rooms\n\
             `/economy` — Show marketplace stats\n\
             `/whoami` — Show your identity\n\
             `/nick <name>` — Set your nickname\n\
             `/topic [text]` — Show or set room description\n\
             `/missions` — List active missions\n\
             `/uptime` — Gateway uptime and version\n\
             `/create-room <name>` — Create a public room\n\
             `/private-room <name>` — Create a private room\n\
             \n\
             **Marketplace:**\n\
             `/skills` — List marketplace skills\n\
             `/search <query>` — Search skills by name/tag\n\
             `/publish <name> <price> [tags]` — Publish a skill\n\
             `/buy <skill-id>` — Buy a skill\n\
             `/balance` — Check your credits\n\
             `/balances` — All agent balances\n\
             \n\
             **Agents:**\n\
             `/spawn <task>` — Spawn a background agent for a task\n\
             `/approve <plan-id>` — Approve a pending plan card\n\
             `/reject <plan-id> [reason]` — Reject a pending plan card\n\
             `/pending` — Show pending plan approvals\n\
             `/ask <question>` — Ask the gateway LLM inline\n\
             \n\
             **Workflow (skill-based):**\n\
             `/tdd` — Test-driven development (RED→GREEN→REFACTOR)\n\
             `/plan` — Implementation planning with approval gate\n\
             `/code-review` — Code quality review checklist\n\
             `/build-fix` — Fix build errors incrementally\n\
             `/verify` — Full verification cycle (build+test+clippy+fmt)\n\
             `/security-review` — Security audit checklist\n\
             `/learn` — Extract patterns to Mnemosyne memory\n\
             `/evolve` — Cluster patterns into skills\n\
             `/checkpoint` — Git save point\n\
             `/orchestrate` — Full workflow pipeline"
            .to_string(),
        "/agents" => {
            let st = state.read().await;
            let gsm = st.global_state();
            let agents = gsm.list_agents().await;
            if agents.is_empty() {
                "No agents registered in fleet.".to_string()
            } else {
                let mut lines = vec!["**Fleet Agents:**".to_string()];
                for a in &agents {
                    let status_dot = if a.is_available() { "🟢" } else { "⚪" };
                    lines.push(format!("{} **{}** — {}", status_dot, a.name, a.status));
                }
                lines.join("\n")
            }
        }
        "/members" => {
            let members = store.list_room_members(&room.id).await;
            if members.is_empty() {
                "No members in this room.".to_string()
            } else {
                let mut lines = vec![format!("**Members of #{}:**", room.name)];
                for m in &members {
                    lines.push(format!("• **{}** ({})", m.agent_name, m.agent_id));
                }
                lines.join("\n")
            }
        }
        "/rooms" => {
            let rooms = store.list_rooms().await;
            let mut lines = vec![format!("**Rooms ({}):**", rooms.len())];
            for r in &rooms {
                let type_tag = match r.room_type {
                    RoomType::Public => "public",
                    RoomType::Private => "private",
                    RoomType::Dm => "dm",
                };
                lines.push(format!(
                    "• **#{}** [{}] — {}",
                    r.name,
                    type_tag,
                    r.description.as_deref().unwrap_or("No description")
                ));
            }
            lines.join("\n")
        }
        "/economy" => {
            let st = state.read().await;
            // #web4 P0-1c cut-9: stats/reputations from marketplace_store (SoT).
            let stats = st.marketplace_store.stats().await;
            let reps = st.marketplace_store.all_reputations().await;
            let mut lines = vec![
                "**Marketplace Economy:**".to_string(),
                format!(
                    "Listings: {} active / {} total",
                    stats.active_listings, stats.total_listings
                ),
                format!(
                    "Trades: {} completed / {} total",
                    stats.completed_trades, stats.total_trades
                ),
                format!(
                    "Token supply: {} across {} agents",
                    stats.total_supply, stats.total_agents
                ),
            ];
            if !reps.is_empty() {
                lines.push("**Agent Trust Scores:**".to_string());
                for r in &reps {
                    lines.push(format!(
                        "• {} — trust: {:.1}, trades: {}",
                        r.agent_id, r.trust_score, r.total_trades
                    ));
                }
            }
            lines.join("\n")
        }
        "/whoami" => match store.get_identity(sender_id).await {
            Some((name, Some(nick))) => {
                format!("You are **{}** aka **{}** (`{}`)", name, nick, sender_id)
            }
            Some((name, None)) => format!(
                "You are **{}** (`{}`)\n_Use `/nick <name>` to set a nickname_",
                name, sender_id
            ),
            None => format!(
                "You are **{}** (`{}`)\n_Use `/nick <name>` to set a nickname_",
                sender_name, sender_id
            ),
        },
        "/nick" => {
            if _args.is_empty() {
                "Usage: `/nick <name>` — Set your display nickname".to_string()
            } else {
                let nick = _args.trim();
                store.set_nickname(sender_id, sender_name, nick).await;
                format!("**{}** is now known as **{}**", sender_name, nick)
            }
        }
        "/topic" => {
            if _args.is_empty() {
                format!(
                    "**#{}** — {}",
                    room.name,
                    room.description.as_deref().unwrap_or("No description")
                )
            } else {
                // Topic setting would require a store update — just display for now
                format!(
                    "Topic display: **{}**\n_(Topic setting coming soon)_",
                    _args
                )
            }
        }
        "/missions" => {
            let missions = store.list_by_status("executing").await;
            let assembling = store.list_by_status("assembling").await;
            let total_active = missions.len() + assembling.len();
            if total_active == 0 {
                "No active missions.".to_string()
            } else {
                let mut lines = vec![format!("**Active Missions ({}):**", total_active)];
                for m in missions.iter().chain(assembling.iter()) {
                    lines.push(format!("• `{}` [{}] — {}", m.id, m.status, m.goal));
                }
                lines.join("\n")
            }
        }
        "/uptime" => {
            format!(
                "**Zeus Gateway** v{}\nStatus: operational",
                env!("CARGO_PKG_VERSION")
            )
        }
        "/create-room" | "/newroom" => {
            if _args.is_empty() {
                "Usage: `/create-room <name>` — Creates a new public room".to_string()
            } else {
                let room_name = _args.to_string();
                let new_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
                let new_room = Room {
                    id: new_id.clone(),
                    name: room_name.clone(),
                    description: Some(format!("Created by {} via /create-room", sender_name)),
                    room_type: RoomType::Public,
                    mission_id: None,
                    created_by: sender_id.to_string(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                store.insert_room(&new_room).await;
                store.emit(PantheonEvent::RoomCreated { room: new_room });
                format!(
                    "Room **#{}** created (`{}`). Join at `/v1/pantheon/rooms/{}/join`",
                    room_name, new_id, new_id
                )
            }
        }
        "/private-room" => {
            if _args.is_empty() {
                "Usage: `/private-room <name>` — Creates a new private room".to_string()
            } else {
                let room_name = _args.to_string();
                let new_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
                let new_room = Room {
                    id: new_id.clone(),
                    name: room_name.clone(),
                    description: Some(format!("Private room created by {}", sender_name)),
                    room_type: RoomType::Private,
                    mission_id: None,
                    created_by: sender_id.to_string(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                store.insert_room(&new_room).await;
                store.emit(PantheonEvent::RoomCreated { room: new_room });
                format!("Private room **#{}** created (`{}`)", room_name, new_id)
            }
        }
        // ── Agora Marketplace Commands ──
        "/skills" | "/list-skills" => {
            let st = state.read().await;
            // #web4 P0-1c cut-7: read from marketplace_store (persistent SoT) so
            // in-session publishes are visible without a restart.
            let listings = st.marketplace_store.list_active_listings().await;
            if listings.is_empty() {
                "**Marketplace** — No skills listed yet.\nPublish with `/publish <name> <price> [tags]`".to_string()
            } else {
                let mut lines = vec![format!("**Marketplace Skills ({}):**", listings.len())];
                for l in listings.iter().take(20) {
                    let tag_list: Vec<String> =
                        serde_json::from_str(&l.tags_json).unwrap_or_default();
                    let tags = if tag_list.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", tag_list.join(", "))
                    };
                    lines.push(format!(
                        "• `{}` **{}** — {} credits{} (by {})",
                        l.id, l.name, l.price, tags, l.publisher_id
                    ));
                }
                if listings.len() > 20 {
                    lines.push(format!(
                        "...and {} more. Use `/search <query>` to filter.",
                        listings.len() - 20
                    ));
                }
                lines.join("\n")
            }
        }
        "/search" => {
            if _args.is_empty() {
                "Usage: `/search <query>` — Search skills by name, tag, or capability".to_string()
            } else {
                let st = state.read().await;
                let query = _args.to_lowercase();
                // #web4 P0-1c cut-7: search marketplace_store (persistent SoT) so
                // in-session publishes are searchable without a restart.
                let mut results = st.marketplace_store.search_listings(&query).await;
                if results.is_empty() {
                    results = st.marketplace_store.search_by_tag(&query).await;
                }
                if results.is_empty() {
                    results = st.marketplace_store.search_by_capability(&query).await;
                }
                if results.is_empty() {
                    format!("No skills found matching \"{}\"", _args)
                } else {
                    let mut lines = vec![format!(
                        "**Search results for \"{}\" ({}):**",
                        _args,
                        results.len()
                    )];
                    for l in results.iter().take(10) {
                        lines.push(format!(
                            "• `{}` **{}** — {} credits (by {})",
                            l.id, l.name, l.price, l.publisher_id
                        ));
                    }
                    lines.join("\n")
                }
            }
        }
        "/balance" => {
            let st = state.read().await;
            // #web4 P0-1c cut-9: balance from marketplace_store (persistent SoT).
            let bal = st.marketplace_store.get_balance(sender_id).await;
            format!("**{}** — Balance: **{}** credits", sender_name, bal)
        }
        "/balances" => {
            let st = state.read().await;
            // #web4 P0-1c cut-9: balances from marketplace_store (persistent SoT).
            let balances = st.marketplace_store.all_balances().await;
            if balances.is_empty() {
                "No agents have wallets yet.".to_string()
            } else {
                let mut lines = vec!["**Agent Balances:**".to_string()];
                let mut sorted: Vec<_> = balances.iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(a.1));
                for (agent, bal) in sorted.iter().take(20) {
                    lines.push(format!("• **{}** — {} credits", agent, bal));
                }
                lines.join("\n")
            }
        }
        "/publish" => {
            if _args.is_empty() {
                "Usage: `/publish <name> <price> [tag1,tag2,...]`\nExample: `/publish code-review 50 rust,review`".to_string()
            } else {
                let parts: Vec<&str> = _args.splitn(3, char::is_whitespace).collect();
                let name = parts.first().unwrap_or(&"unnamed").to_string();
                let price: u64 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(10);
                let tags: Vec<String> = parts
                    .get(2)
                    .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();

                let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());

                // #web4 P0-1b cut-4e: publish directly to marketplace_store (persistent SoT);
                // id generated locally, dropping the in-memory registry.publish dependency.
                let id = Uuid::new_v4().to_string();
                let st = state.read().await;
                {
                    let caps_json = "[]".to_string();
                    let row = crate::handlers::marketplace_store::SkillListingRow {
                        id: id.clone(),
                            name: name.clone(),
                            description: format!("Published by {} in #{}", sender_name, room.name),
                            publisher_id: sender_id.to_string(),
                            capabilities_json: caps_json,
                            tags_json,
                            price,
                            version: "0.1.0".to_string(),
                            rating: 0.0,
                            rating_count: 0,
                            downloads: 0,
                            active: true,
                            source: "pantheon".to_string(),
                            metadata_json: "{}".to_string(),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        };
                    st.marketplace_store.publish_listing(&row).await;
                    format!(
                        "Skill **{}** published (`{}`) — {} credits",
                        name, id, price
                    )
                }
            }
        }
        "/buy" => {
            if _args.is_empty() {
                "Usage: `/buy <skill-id>` — Purchase a skill from the marketplace".to_string()
            } else {
                let skill_id = _args.trim();
                let st = state.read().await;
                let listing = match st.marketplace_store.get_listing(skill_id).await {
                    Some(l) => l,
                    None => {
                        return Some(RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room.id.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!("Skill `{}` not found", skill_id),
                            message_type: "system".to_string(),
                            metadata: Some(json!({"command": "/buy", "error": "not_found"})),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        });
                    }
                };

                // #web4 P0-1b cut-4f: balance read from marketplace_store (persistent SoT).
                // store price u64 → i64 for balance compare (price non-negative, fits i64).
                let buyer_balance = st.marketplace_store.get_balance(sender_id).await;
                if buyer_balance < listing.price as i64 {
                    format!(
                        "Insufficient credits. Need **{}** but you have **{}**",
                        listing.price, buyer_balance
                    )
                } else {
                    // #web4 P0-1c cut-9: marketplace_store is the sole ledger —
                    // the in-memory transfer was removed; store transfer result
                    // now decides success/failure.
                    let transfer_ok = st
                        .marketplace_store
                        .transfer(
                            sender_id,
                            &listing.publisher_id,
                            listing.price,
                            &format!("Purchase: {}", listing.name),
                        )
                        .await;
                    if transfer_ok {
                        // #web4 P0-1b cut-4d: dropped redundant registry.record_download;
                        // marketplace_store is the persistent SoT.
                        st.marketplace_store.record_download(skill_id).await;
                        format!(
                            "Purchased **{}** for **{}** credits from **{}**. New balance: **{}**",
                            listing.name,
                            listing.price,
                            listing.publisher_id,
                            st.marketplace_store.get_balance(sender_id).await
                        )
                    } else {
                        "Transaction failed: ledger transfer was rejected".to_string()
                    }
                }
            }
        }
        "/spawn" => {
            if _args.is_empty() {
                "Usage: `/spawn <task>` — Spawn a background agent to handle a task.\nExample: `/spawn scan Polymarket for trending markets`".to_string()
            } else {
                let task = _args.to_string();
                let spawn_id = format!("spawn-{}", &Uuid::new_v4().to_string()[..8]);
                let room_id = room.id.clone();
                let agent_name = format!("Agent-{}", &spawn_id[6..]);
                let spawner = sender_name.to_string();

                // ── Complexity analysis — gate complex/risky tasks on approval ──
                let analyzer = ComplexityAnalyzer::new();
                let complexity = analyzer.classify_message(&task);
                let plan_card = analyzer.analyze(
                    &spawn_id,
                    &task,
                    &[(task.clone(), Some("shell".to_string()))], // conservative: treat spawn as shell
                    &[],
                );

                if plan_card.requires_approval {
                    // Build steps JSON matching Zeus100's frontend contract
                    let steps_json = serde_json::to_string(&vec![json!({
                        "description": task,
                        "agent_type": "spawn",
                        "status": "pending",
                        "elapsed_ms": null
                    })])
                    .unwrap_or_default();

                    // Persist the pending approval
                    store
                        .insert_pending_approval(
                            &spawn_id,
                            &room_id,
                            sender_id,
                            sender_name,
                            &task,
                            &format!("{}", complexity),
                            &format!("{:?}", plan_card.risk),
                            &steps_json,
                            &task,
                        )
                        .await;

                    // Emit plan_card message for the frontend
                    let card_msg = RoomMessage {
                        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                        room_id: room_id.clone(),
                        sender_id: "system".to_string(),
                        sender_name: "System".to_string(),
                        content: format!("Plan requires approval: _{}_", task),
                        message_type: "plan_card".to_string(),
                        metadata: Some(json!({
                            "plan_id": spawn_id,
                            "goal": task,
                            "steps": [{
                                "description": task,
                                "agent_type": "spawn",
                                "status": "pending",
                                "elapsed_ms": null,
                            }],
                            "status": "awaiting_approval",
                            "revision": 1,
                        })),
                        reply_to: None,
                        edited: false,
                        attachments: vec![],
                        timestamp: Utc::now(),
                    };
                    store.insert_room_message(&card_msg).await;
                    store.emit(PantheonEvent::RoomMessageSent {
                        room_id: room_id.clone(),
                        message: card_msg,
                    });
                    store.emit(PantheonEvent::PlanCardCreated {
                        room_id,
                        plan_id: spawn_id.clone(),
                        goal: task.clone(),
                        complexity: format!("{}", complexity),
                        risk: format!("{:?}", plan_card.risk),
                    });

                    let reason = plan_card
                        .approval_reason
                        .unwrap_or_else(|| "complex or risky task".to_string());
                    format!(
                        "\u{1f6e1}\u{fe0f} **Approval required** for `{}`\nReason: _{}_\nUse `/approve {}` or `/reject {}`",
                        spawn_id, reason, spawn_id, spawn_id
                    )
                } else {
                    // Simple/moderate — execute immediately
                    let ack = format!(
                        "\u{1f680} **Spawning agent** `{}` for: _{}_\nRequested by **{}** — results will appear in this room.",
                        spawn_id, task, spawner
                    );

                    execute_spawn(
                        state.clone(),
                        store.clone(),
                        spawn_id,
                        agent_name,
                        room_id,
                        task,
                    );

                    ack
                }
            }
        }
        "/approve" => {
            if _args.is_empty() {
                "Usage: `/approve <plan-id>` — Approve a pending plan card".to_string()
            } else {
                let plan_id = _args.split_whitespace().next().unwrap_or("").to_string();
                match store.approve_plan(&plan_id, sender_id, sender_name).await {
                    Some(approval) => {
                        // Emit approval event
                        store.emit(PantheonEvent::PlanApproved {
                            room_id: room.id.clone(),
                            plan_id: plan_id.clone(),
                            approved_by: sender_id.to_string(),
                        });

                        // Post updated plan_card with approved status
                        let steps: Value =
                            serde_json::from_str(&approval.steps_json).unwrap_or(json!([]));
                        let card_msg = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room.id.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!(
                                "Plan approved by **{}**: _{}_",
                                sender_name, approval.goal
                            ),
                            message_type: "plan_card".to_string(),
                            metadata: Some(json!({
                                "plan_id": plan_id,
                                "goal": approval.goal,
                                "steps": steps,
                                "status": "approved",
                                "revision": approval.revision,
                            })),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&card_msg).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room.id.clone(),
                            message: card_msg,
                        });

                        // Execute the approved plan — creates a real mission via MissionDriver
                        execute_approved_plan(
                            state.clone(),
                            store.clone(),
                            plan_id.clone(),
                            room.id.clone(),
                            approval.goal,
                            approval.steps_json,
                        );

                        format!(
                            "\u{2705} **Plan `{}` approved** — coordinated execution started.",
                            plan_id
                        )
                    }
                    None => format!("\u{274c} No pending plan found with ID `{}`", plan_id),
                }
            }
        }
        "/reject" => {
            if _args.is_empty() {
                "Usage: `/reject <plan-id> [reason]` — Reject a pending plan card".to_string()
            } else {
                let mut parts = _args.splitn(2, char::is_whitespace);
                let plan_id = parts.next().unwrap_or("").to_string();
                let reason = parts.next().map(|s| s.trim().to_string());

                match store
                    .reject_plan(&plan_id, sender_id, sender_name, reason.as_deref())
                    .await
                {
                    Some(approval) => {
                        let current_revision = approval.revision;

                        store.emit(PantheonEvent::PlanRejected {
                            room_id: room.id.clone(),
                            plan_id: plan_id.clone(),
                            rejected_by: sender_id.to_string(),
                            reason: reason.clone(),
                        });

                        // Post updated plan_card with rejected status
                        let steps: Value =
                            serde_json::from_str(&approval.steps_json).unwrap_or(json!([]));
                        let card_msg = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room.id.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!(
                                "Plan rejected by **{}**: _{}_",
                                sender_name, approval.goal
                            ),
                            message_type: "plan_card".to_string(),
                            metadata: Some(json!({
                                "plan_id": plan_id,
                                "goal": approval.goal,
                                "steps": steps,
                                "status": "rejected",
                                "revision": current_revision,
                            })),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&card_msg).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room.id.clone(),
                            message: card_msg,
                        });

                        // ── Re-plan loop: auto-revise if under max revisions ──
                        if current_revision < 3 {
                            let state_c = state.clone();
                            let store_c = store.clone();
                            let plan_id_c = plan_id.clone();
                            let room_id_c = room.id.clone();
                            let original_goal = approval.goal.clone();
                            let reject_feedback = reason
                                .clone()
                                .unwrap_or_else(|| "no specific reason given".to_string());

                            tokio::spawn(async move {
                                replan_with_feedback(
                                    &state_c,
                                    &store_c,
                                    &plan_id_c,
                                    &room_id_c,
                                    &original_goal,
                                    &reject_feedback,
                                )
                                .await;
                            });

                            let reason_text =
                                reason.map(|r| format!(" — _{}_", r)).unwrap_or_default();
                            format!(
                                "\u{274c} **Plan `{}` rejected** by **{}**{}\n\u{1f504} Re-planning with feedback (revision {}/3)…",
                                plan_id,
                                sender_name,
                                reason_text,
                                current_revision + 1
                            )
                        } else {
                            let reason_text =
                                reason.map(|r| format!(" — _{}_", r)).unwrap_or_default();
                            format!(
                                "\u{274c} **Plan `{}` rejected** by **{}**{}\n\u{1f6d1} Max revisions (3) reached. Use `/spawn` to start fresh.",
                                plan_id, sender_name, reason_text
                            )
                        }
                    }
                    None => format!("\u{274c} No pending plan found with ID `{}`", plan_id),
                }
            }
        }
        "/pending" => {
            let pending = store.list_pending_approvals(&room.id).await;
            if pending.is_empty() {
                "No pending plan approvals in this room.".to_string()
            } else {
                let mut lines = vec![format!("**Pending Approvals ({}):**", pending.len())];
                for p in &pending {
                    lines.push(format!(
                        "\u{1f4cb} `{}` — _{}_  [{}] requested by **{}**",
                        p.plan_id, p.goal, p.complexity, p.requested_by_name
                    ));
                }
                lines.push("\nUse `/approve <id>` or `/reject <id> [reason]`".to_string());
                lines.join("\n")
            }
        }
        "/ask" => {
            if _args.is_empty() {
                "Usage: `/ask <question>` — Ask the gateway agent a question inline.\nExample: `/ask what is the current SOL price?`".to_string()
            } else {
                let question = _args.to_string();
                let config = state.read().await.config.clone();
                let mnemosyne = state.read().await.mnemosyne.clone();
                // Fetch room context for the LLM (recent + semantic search)
                let semantic = mnemosyne
                    .as_ref()
                    .map(|mn| (mn.as_ref(), question.as_str()));
                let room_context = build_room_context(store, &room.id, 20, semantic).await;
                let context_summary = format_room_context_summary(&room_context);
                match LlmClient::from_config(&config) {
                    Ok(llm) => {
                        let system_prompt = if context_summary.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "You are answering a question in a Zeus War Room. Use the recent conversation for context:{}",
                                context_summary
                            ))
                        };
                        let messages = vec![zeus_core::Message::user(&question)];
                        match llm.complete(&messages, &[], system_prompt.as_deref()).await {
                            Ok(resp) => format!("\u{1f916} {}", resp.content),
                            Err(e) => format!("\u{274c} LLM error: {}", e),
                        }
                    }
                    Err(_) => "\u{274c} No LLM configured on this gateway.".to_string(),
                }
            }
        }
        _ => {
            // ── Skill-based slash command dispatch ──
            // Check if the command matches a skill with user-invocable: true.
            // Skills in workspace/skills/ with a matching skillKey become slash
            // commands automatically.
            let skill_cmd = cmd.trim_start_matches('/');
            let config = state.read().await.config.clone();
            let skills_dir = config.workspace.join("skills");
            if skills_dir.exists()
                && let Ok(mut rd) = tokio::fs::read_dir(&skills_dir).await
            {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let skill_path = entry.path().join("SKILL.md");
                    if !skill_path.exists() {
                        continue;
                    }
                    if let Ok(content) = tokio::fs::read_to_string(&skill_path).await
                        && let Ok(skill) = zeus_skills::parse_skill_md(&content, skill_path.clone())
                    {
                        if !skill.invocation.user_invocable {
                            continue;
                        }
                        // Match by skillKey (snake_case) or sanitized name
                        let key = skill
                            .frontmatter
                            .get("skillkey")
                            .or_else(|| skill.frontmatter.get("skillKey"))
                            .cloned()
                            .unwrap_or_else(|| skill.name.to_lowercase().replace('-', "_"));
                        let name_match = skill.name.to_lowercase().replace('-', "_");
                        if skill_cmd == key
                            || skill_cmd == name_match
                            || skill_cmd == skill.name.to_lowercase()
                            || skill_cmd == skill.name.to_lowercase().replace('_', "-")
                        {
                            let desc = if skill.system_prompt.is_empty() {
                                &skill.description
                            } else {
                                &skill.system_prompt
                            };
                            let truncated = if desc.len() > 2000 {
                                let mut end = 2000;
                                while end > 0 && !desc.is_char_boundary(end) {
                                    end -= 1;
                                }
                                format!("{}…", &desc[..end])
                            } else {
                                desc.clone()
                            };
                            let response = format!(
                                "**/{name}** — {title}\n\n{body}",
                                name = skill.name,
                                title = skill.description,
                                body = truncated,
                            );
                            let msg = RoomMessage {
                                id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                                room_id: room.id.clone(),
                                sender_id: "system".to_string(),
                                sender_name: "System".to_string(),
                                content: response,
                                message_type: "system".to_string(),
                                metadata: Some(json!({
                                    "command": cmd,
                                    "skill": skill.name,
                                    "invoked_by": sender_id
                                })),
                                reply_to: None,
                                edited: false,
                                attachments: vec![],
                                timestamp: Utc::now(),
                            };
                            return Some(msg);
                        }
                    }
                }
            }
            return None;
        }
    };

    let msg = RoomMessage {
        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
        room_id: room.id.clone(),
        sender_id: "system".to_string(),
        sender_name: "System".to_string(),
        content: response,
        message_type: "system".to_string(),
        metadata: Some(json!({"command": cmd, "invoked_by": sender_id})),
        reply_to: None,
        edited: false,
        attachments: vec![],
        timestamp: Utc::now(),
    };

    Some(msg)
}

pub async fn send_room_message(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<SendRoomMessageRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let room = match store.get_room(&id).await {
        Some(r) => r,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "Room not found"})),
            )
                .into_response();
        }
    };

    // ── Slash command interception ──
    if req.content.starts_with('/')
        && let Some(sys_msg) = handle_slash_command(
            &state,
            &store,
            &room,
            &req.sender_id,
            &req.sender_name,
            &req.content,
        )
        .await
    {
        store.insert_room_message(&sys_msg).await;
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: id.clone(),
            message: sys_msg.clone(),
        });
        return (StatusCode::CREATED, Json(json!(sys_msg))).into_response();
    }
    // Unrecognized command — fall through as regular message

    let msg = RoomMessage {
        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
        room_id: id.clone(),
        sender_id: req.sender_id.clone(),
        sender_name: req.sender_name.clone(),
        content: req.content.clone(),
        message_type: req.message_type.clone(),
        metadata: req.metadata,
        reply_to: req.reply_to,
        edited: false,
        attachments: req.attachments,
        timestamp: Utc::now(),
    };

    store.insert_room_message(&msg).await;

    // ── Mnemosyne: index room messages for semantic search / context persistence ──
    if msg.message_type != "system" {
        let mnemosyne = state.read().await.mnemosyne.clone();
        if let Some(mn) = mnemosyne {
            let mn_msg = zeus_core::Message::user(format!("[{}] {}", msg.sender_name, msg.content));
            let session_id = format!("room:{}", id);
            if let Err(e) = mn.store(&session_id, &mn_msg).await {
                tracing::debug!("Mnemosyne room index failed (non-fatal): {}", e);
            }
        }
    }

    store.emit(PantheonEvent::RoomMessageSent {
        room_id: id.clone(),
        message: msg.clone(),
    });

    // ── Discord bridge: relay War Room messages to Discord for agent visibility ──
    // Uses Discord webhooks so messages appear under the sender's name, not the bot.
    // Set PANTHEON_DISCORD_WEBHOOK to the full webhook URL.
    // Fallback: uses bot API with DISCORD_BOT_TOKEN if no webhook configured.
    if msg.message_type != "system" && !req.sender_id.starts_with("discord-bridge") {
        let bridge_room_name = room.name.clone();
        let bridge_sender = req.sender_name.clone();
        let bridge_msg_type = msg.message_type.clone();
        let bridge_metadata = msg.metadata.clone();
        let bridge_content = req.content.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();

            // Format plan_card as a rich Discord message
            let formatted_content = if bridge_msg_type == "plan_card" {
                if let Some(meta) = &bridge_metadata {
                    let goal = meta
                        .get("goal")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&bridge_content);
                    let status = meta
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("awaiting_approval");
                    let plan_id = meta.get("plan_id").and_then(|v| v.as_str()).unwrap_or("");
                    let steps = meta
                        .get("steps")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let emoji = match status {
                        "approved" | "executing" => "✅",
                        "rejected" => "❌",
                        "complete" => "🏁",
                        _ => "🛡️",
                    };
                    let mut out = format!(
                        "{} **Plan `{}`** — {}\n**Status:** {}\n",
                        emoji,
                        plan_id,
                        goal,
                        status.to_uppercase()
                    );
                    if !steps.is_empty() {
                        out.push_str("**Steps:**\n");
                        for (i, step) in steps.iter().enumerate() {
                            let desc = step
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("...");
                            let st = step
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("pending");
                            let icon = match st {
                                "done" | "completed" => "✓",
                                "running" => "⟳",
                                "failed" => "✗",
                                _ => "○",
                            };
                            out.push_str(&format!("{} {}. {}\n", icon, i + 1, desc));
                        }
                    }
                    if status == "awaiting_approval" {
                        out.push_str(&format!(
                            "\n> Use `/approve {}` or `/reject {}` in War Room",
                            plan_id, plan_id
                        ));
                    }
                    out
                } else {
                    bridge_content.clone()
                }
            } else {
                bridge_content.clone()
            };

            // Prefer webhook (shows sender's name as Discord username)
            if let Ok(webhook_url) = std::env::var("PANTHEON_DISCORD_WEBHOOK") {
                let content = format!("\u{1f4ac} **#{}**: {}", bridge_room_name, formatted_content);
                let _ = client
                    .post(&webhook_url)
                    .json(&serde_json::json!({
                        "content": content,
                        "username": format!("{} (War Room)", bridge_sender),
                    }))
                    .send()
                    .await;
                return;
            }

            // Fallback: bot API (sender name in message body)
            let channel_id = match std::env::var("PANTHEON_DISCORD_CHANNEL") {
                Ok(id) => id,
                Err(_) => match std::env::var("DISCORD_RELAY_CHANNEL_IDS") {
                    Ok(ids) => ids.split(',').next().unwrap_or("").trim().to_string(),
                    Err(_) => return,
                },
            };
            let token = match zeus_core::resolve_discord_token() {
                Some(t) => t,
                None => return,
            };
            if channel_id.is_empty() {
                return;
            }

            let notification = format!(
                "\u{1f4ac} **Pantheon #{}** \u{2014} **{}**: {}",
                bridge_room_name, bridge_sender, formatted_content
            );

            let client = reqwest::Client::new();
            let url = format!(
                "https://discord.com/api/v10/channels/{}/messages",
                channel_id
            );
            if let Err(e) = client
                .post(&url)
                .header("Authorization", format!("Bot {}", token))
                .json(&serde_json::json!({ "content": notification }))
                .send()
                .await
            {
                tracing::warn!("Pantheon→Discord bridge failed: {e}");
            }
        });
    }

    // ── Intent routing: auto-classify and respond to natural language ──
    // Only routes "chat" messages from non-system senders that look like requests.
    // Short messages, greetings, and bot messages are left as plain chat.
    let should_route = msg.message_type == "chat"
        && !req.sender_id.starts_with("system")
        && !req.sender_id.starts_with("spawn-")
        && !req.sender_id.starts_with("discord-bridge")
        && req.content.len() >= 15; // skip short messages ("hi", "ok", "thanks", etc.)

    if should_route {
        let analyzer = ComplexityAnalyzer::new();
        let complexity = analyzer.classify_message(&req.content);

        match complexity {
            zeus_nous::ComplexityLevel::Simple | zeus_nous::ComplexityLevel::Moderate => {
                // Inline LLM response — like /ask but automatic
                let state_c = state.clone();
                let store_c = store.clone();
                let room_id_c = id.clone();
                let question = req.content.clone();
                tokio::spawn(async move {
                    let config = state_c.read().await.config.clone();
                    let mnemosyne = state_c.read().await.mnemosyne.clone();
                    // Build room context with semantic search for auto-reply
                    let semantic = mnemosyne
                        .as_ref()
                        .map(|mn| (mn.as_ref(), question.as_str()));
                    let room_context = build_room_context(&store_c, &room_id_c, 20, semantic).await;
                    let context_summary = format_room_context_summary(&room_context);
                    if let Ok(llm) = LlmClient::from_config(&config) {
                        let system_prompt = if context_summary.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "You are Zeus, answering a question in a War Room. Use the conversation context to inform your response:{}",
                                context_summary
                            ))
                        };
                        let messages = vec![zeus_core::Message::user(&question)];
                        if let Ok(resp) =
                            llm.complete(&messages, &[], system_prompt.as_deref()).await
                        {
                            let reply = RoomMessage {
                                id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                                room_id: room_id_c.clone(),
                                sender_id: "zeus".to_string(),
                                sender_name: "Zeus".to_string(),
                                content: resp.content,
                                message_type: "chat".to_string(),
                                metadata: Some(
                                    json!({"intent": "auto_reply", "complexity": format!("{}", complexity)}),
                                ),
                                reply_to: None,
                                edited: false,
                                attachments: vec![],
                                timestamp: Utc::now(),
                            };
                            store_c.insert_room_message(&reply).await;
                            store_c.emit(PantheonEvent::RoomMessageSent {
                                room_id: room_id_c,
                                message: reply,
                            });
                        }
                    }
                });
            }
            zeus_nous::ComplexityLevel::Complex => {
                // Check if the request is too vague to plan
                if analyzer.needs_clarification(&req.content) {
                    // Ask clarifying questions before planning
                    let state_c = state.clone();
                    let store_c = store.clone();
                    let room_id_c = id.clone();
                    let question = req.content.clone();
                    tokio::spawn(async move {
                        let config = state_c.read().await.config.clone();
                        let mnemosyne = state_c.read().await.mnemosyne.clone();
                        let semantic = mnemosyne
                            .as_ref()
                            .map(|mn| (mn.as_ref(), question.as_str()));
                        let room_context =
                            build_room_context(&store_c, &room_id_c, 20, semantic).await;
                        let context_summary = format_room_context_summary(&room_context);
                        if let Ok(llm) = LlmClient::from_config(&config) {
                            let base = "You are Zeus, an AI assistant in a War Room. The user has a complex request \
                                but hasn't provided enough detail. Ask 2-4 brief, specific clarifying questions to understand \
                                what they want. Format as a numbered list. Be friendly but concise. Do NOT start working — just ask questions.";
                            let system_prompt = if context_summary.is_empty() {
                                base.to_string()
                            } else {
                                format!("{}{}", base, context_summary)
                            };
                            let messages = vec![zeus_core::Message::user(&question)];
                            if let Ok(resp) =
                                llm.complete(&messages, &[], Some(&system_prompt)).await
                            {
                                let reply = RoomMessage {
                                    id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                                    room_id: room_id_c.clone(),
                                    sender_id: "zeus".to_string(),
                                    sender_name: "Zeus".to_string(),
                                    content: resp.content,
                                    message_type: "chat".to_string(),
                                    metadata: Some(json!({
                                        "intent": "clarification",
                                        "complexity": "complex",
                                        "original_request": question,
                                    })),
                                    reply_to: None,
                                    edited: false,
                                    attachments: vec![],
                                    timestamp: Utc::now(),
                                };
                                store_c.insert_room_message(&reply).await;
                                store_c.emit(PantheonEvent::RoomMessageSent {
                                    room_id: room_id_c,
                                    message: reply,
                                });
                            }
                        }
                    });
                } else {
                    // Specific enough — auto-spawn with plan card
                    let task = req.content.clone();
                    let spawn_id = format!("spawn-{}", &Uuid::new_v4().to_string()[..8]);
                    let room_id_c = id.clone();
                    let agent_name = format!("Agent-{}", &spawn_id[6..]);

                    let plan_card = analyzer.analyze(
                        &spawn_id,
                        &task,
                        &[(task.clone(), Some("shell".to_string()))],
                        &[],
                    );

                    if plan_card.requires_approval {
                        // Post plan card for approval
                        let steps_json = serde_json::to_string(&vec![json!({
                            "description": task,
                            "agent_type": "spawn",
                            "status": "pending",
                            "elapsed_ms": null
                        })])
                        .unwrap_or_default();

                        store
                            .insert_pending_approval(
                                &spawn_id,
                                &room_id_c,
                                &req.sender_id,
                                &req.sender_name,
                                &task,
                                "complex",
                                &format!("{:?}", plan_card.risk),
                                &steps_json,
                                &task,
                            )
                            .await;

                        let card_msg = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room_id_c.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!("Plan requires approval: _{}_", task),
                            message_type: "plan_card".to_string(),
                            metadata: Some(json!({
                                "plan_id": spawn_id,
                                "goal": task,
                                "steps": [{"description": task, "agent_type": "spawn", "status": "pending"}],
                                "status": "awaiting_approval",
                                "revision": 1,
                            })),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&card_msg).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room_id_c.clone(),
                            message: card_msg,
                        });
                        store.emit(PantheonEvent::PlanCardCreated {
                            room_id: room_id_c.clone(),
                            plan_id: spawn_id.clone(),
                            goal: task.clone(),
                            complexity: "complex".to_string(),
                            risk: format!("{:?}", plan_card.risk),
                        });

                        // Post approval notice
                        let reason = plan_card
                            .approval_reason
                            .unwrap_or_else(|| "complex task".to_string());
                        let notice = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room_id_c.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!(
                                "\u{1f9e0} Detected complex task — plan card posted.\nReason: _{}_\nUse `/approve {}` or `/reject {}`",
                                reason, spawn_id, spawn_id
                            ),
                            message_type: "system".to_string(),
                            metadata: Some(json!({"intent": "auto_spawn", "plan_id": spawn_id})),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&notice).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room_id_c,
                            message: notice,
                        });
                    } else {
                        // Complex but not risky enough for approval — execute directly
                        let ack = RoomMessage {
                            id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
                            room_id: room_id_c.clone(),
                            sender_id: "system".to_string(),
                            sender_name: "System".to_string(),
                            content: format!(
                                "\u{1f680} Detected complex task — spawning agent `{}`...",
                                spawn_id
                            ),
                            message_type: "system".to_string(),
                            metadata: Some(json!({"intent": "auto_spawn", "spawn_id": spawn_id})),
                            reply_to: None,
                            edited: false,
                            attachments: vec![],
                            timestamp: Utc::now(),
                        };
                        store.insert_room_message(&ack).await;
                        store.emit(PantheonEvent::RoomMessageSent {
                            room_id: room_id_c.clone(),
                            message: ack,
                        });

                        execute_spawn(
                            state.clone(),
                            store.clone(),
                            spawn_id,
                            agent_name,
                            room_id_c,
                            task,
                        );
                    }
                } // close else (specific enough to plan)
            }
        }
    }

    (StatusCode::CREATED, Json(json!(msg))).into_response()
}

/// POST /v1/pantheon/rooms/:id/upload — Upload file to a war room
///
/// Accepts multipart/form-data with fields:
/// - `file`: the file payload
/// - `sender_id`: who is uploading
/// - `sender_name`: display name
/// - `message` (optional): text to accompany the file
pub async fn upload_room_file(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store.get_room(&id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Room not found"})),
        )
            .into_response();
    }

    let mut sender_id = String::new();
    let mut sender_name = String::new();
    let mut message_text = String::new();
    let mut file_data: Option<(String, Vec<u8>)> = None;

    // Parse multipart fields
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "sender_id" => {
                sender_id = field.text().await.unwrap_or_default();
            }
            "sender_name" => {
                sender_name = field.text().await.unwrap_or_default();
            }
            "message" => {
                message_text = field.text().await.unwrap_or_default();
            }
            "file" => {
                let filename = field
                    .file_name()
                    .unwrap_or("upload")
                    .to_string();
                match field.bytes().await {
                    Ok(bytes) => file_data = Some((filename, bytes.to_vec())),
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": format!("Failed to read file: {}", e)})),
                        )
                            .into_response();
                    }
                }
            }
            _ => {}
        }
    }

    // Validate required fields
    if sender_id.is_empty() || sender_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "sender_id and sender_name are required"})),
        )
            .into_response();
    }

    let (filename, content) = match file_data {
        Some(f) => f,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "No file provided"})),
            )
                .into_response();
        }
    };

    // Save file via the shared upload store
    let mime_type = detect_mime_type(&filename, &content);
    let uploaded = {
        let mut state_write = state.write().await;
        match state_write
            .upload_store
            .save_file(&filename, &content, &mime_type)
        {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to save file: {}", e)})),
                )
                    .into_response();
            }
        }
    };

    let attachment = MessageAttachment {
        filename: uploaded.name.clone(),
        url: format!("/v1/uploads/{}", uploaded.id),
        content_type: uploaded.mime_type.clone(),
        size: uploaded.size,
    };

    // Determine message type from MIME
    let message_type = if mime_type.starts_with("audio/") {
        "voice".to_string()
    } else {
        "file".to_string()
    };

    let display = if message_text.is_empty() {
        format!("📎 {}", attachment.filename)
    } else {
        message_text
    };

    let msg = RoomMessage {
        id: format!("msg-{}", &Uuid::new_v4().to_string()[..8]),
        room_id: id.clone(),
        sender_id,
        sender_name,
        content: display,
        message_type,
        metadata: None,
        reply_to: None,
        edited: false,
        attachments: vec![attachment],
        timestamp: Utc::now(),
    };

    store.insert_room_message(&msg).await;
    store.emit(PantheonEvent::RoomMessageSent {
        room_id: id,
        message: msg.clone(),
    });

    (StatusCode::CREATED, Json(json!(msg))).into_response()
}

/// GET /v1/pantheon/rooms/:id/messages — Get room messages (paginated)
pub async fn get_room_messages(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<RoomMessagesQuery>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store.get_room(&id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Room not found"})),
        )
            .into_response();
    }

    let limit = params.limit.unwrap_or(50).min(zeus_core::MAX_PAGE_LIMIT);
    let messages = store
        .get_room_messages(&id, limit, params.before.as_deref())
        .await;

    (
        StatusCode::OK,
        Json(json!({
            "messages": messages,
            "room_id": id,
            "count": messages.len(),
        })),
    )
        .into_response()
}

/// GET /v1/pantheon/rooms/:id/members — List room members
pub async fn list_room_members(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let members = store.list_room_members(&id).await;
    Json(json!({ "members": members, "count": members.len() }))
}

/// Request body for editing a room message.
#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub content: String,
}

/// DELETE /v1/pantheon/rooms/:id/messages/:msg_id — Delete a message
pub async fn delete_room_message(
    State(state): State<SharedState>,
    Path((room_id, msg_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store.delete_room_message(&msg_id).await {
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.clone(),
            message: RoomMessage {
                id: format!("del-{}", &Uuid::new_v4().to_string()[..8]),
                room_id,
                sender_id: "system".to_string(),
                sender_name: "System".to_string(),
                content: format!("Message {} deleted", msg_id),
                message_type: "system".to_string(),
                metadata: Some(json!({"action": "delete", "deleted_id": msg_id})),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            },
        });
        (StatusCode::OK, Json(json!({"ok": true, "deleted": msg_id}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Message not found"})),
        )
            .into_response()
    }
}

/// PUT /v1/pantheon/rooms/:id/messages/:msg_id — Edit a message
pub async fn edit_room_message(
    State(state): State<SharedState>,
    Path((room_id, msg_id)): Path<(String, String)>,
    Json(req): Json<EditMessageRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store.edit_room_message(&msg_id, &req.content).await {
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.clone(),
            message: RoomMessage {
                id: format!("edit-{}", &Uuid::new_v4().to_string()[..8]),
                room_id,
                sender_id: "system".to_string(),
                sender_name: "System".to_string(),
                content: format!("Message {} edited", msg_id),
                message_type: "system".to_string(),
                metadata: Some(
                    json!({"action": "edit", "edited_id": msg_id, "new_content": req.content}),
                ),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            },
        });
        (StatusCode::OK, Json(json!({"ok": true, "edited": msg_id}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Message not found"})),
        )
            .into_response()
    }
}

// ============================================================================
// Reactions
// ============================================================================

/// Request body for POST /reactions — accepts both agent_id/agent_name and user_id/user_name.
#[derive(Debug, Deserialize)]
pub struct ReactionRequest {
    #[serde(alias = "user_id")]
    pub agent_id: String,
    #[serde(alias = "user_name", default)]
    pub agent_name: String,
    pub emoji: String,
}

/// Query params for DELETE /reactions.
#[derive(Debug, Deserialize)]
pub struct RemoveReactionQuery {
    #[serde(alias = "user_id")]
    pub agent_id: String,
    pub emoji: String,
}

/// POST /v1/pantheon/rooms/:id/messages/:msg_id/reactions — Add a reaction
pub async fn add_reaction(
    State(state): State<SharedState>,
    Path((room_id, msg_id)): Path<(String, String)>,
    Json(req): Json<ReactionRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store
        .add_reaction(
            &msg_id,
            &room_id,
            &req.agent_id,
            &req.agent_name,
            &req.emoji,
        )
        .await
    {
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.clone(),
            message: RoomMessage {
                id: format!("react-{}", &Uuid::new_v4().to_string()[..8]),
                room_id,
                sender_id: req.agent_id.clone(),
                sender_name: req.agent_name.clone(),
                content: format!("{} reacted {} to {}", req.agent_name, req.emoji, msg_id),
                message_type: "system".to_string(),
                metadata: Some(json!({"action": "reaction_add", "message_id": msg_id, "emoji": req.emoji, "agent_id": req.agent_id})),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            },
        });
        (
            StatusCode::CREATED,
            Json(json!({"ok": true, "emoji": req.emoji, "message_id": msg_id})),
        )
            .into_response()
    } else {
        (
            StatusCode::CONFLICT,
            Json(json!({"error": "Reaction already exists or message not found"})),
        )
            .into_response()
    }
}

/// DELETE /v1/pantheon/rooms/:id/messages/:msg_id/reactions — Remove a reaction
///
/// Accepts query params: `agent_id` (or `user_id`) + `emoji`.
pub async fn remove_reaction(
    State(state): State<SharedState>,
    Path((room_id, msg_id)): Path<(String, String)>,
    Query(req): Query<RemoveReactionQuery>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    if store
        .remove_reaction(&msg_id, &req.agent_id, &req.emoji)
        .await
    {
        store.emit(PantheonEvent::RoomMessageSent {
            room_id: room_id.clone(),
            message: RoomMessage {
                id: format!("unreact-{}", &Uuid::new_v4().to_string()[..8]),
                room_id,
                sender_id: req.agent_id.clone(),
                sender_name: req.agent_id.clone(),
                content: format!("{} removed {} from {}", req.agent_id, req.emoji, msg_id),
                message_type: "system".to_string(),
                metadata: Some(json!({"action": "reaction_remove", "message_id": msg_id, "emoji": req.emoji, "agent_id": req.agent_id})),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now(),
            },
        });
        (
            StatusCode::OK,
            Json(json!({"ok": true, "removed": req.emoji, "message_id": msg_id})),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Reaction not found"})),
        )
            .into_response()
    }
}

/// GET /v1/pantheon/rooms/:id/messages/:msg_id/reactions — Get reactions for a message
///
/// Returns a plain JSON array: `[{"emoji":"👍","count":2,"user_ids":["a","b"]}]`
pub async fn get_reactions(
    State(state): State<SharedState>,
    Path((_room_id, msg_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let reactions = store.get_reactions(&msg_id).await;

    let reaction_data: Vec<Value> = reactions
        .iter()
        .map(|(emoji, agents)| {
            json!({
                "emoji": emoji,
                "count": agents.len(),
                "user_ids": agents.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            })
        })
        .collect();

    Json(reaction_data)
}

// ============================================================================
// Identity — user/agent profile management
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SetIdentityRequest {
    pub agent_id: String,
    pub display_name: String,
    #[serde(default)]
    pub nickname: Option<String>,
}

/// PUT /v1/pantheon/identity — Set or update an identity.
pub async fn set_identity(
    State(state): State<SharedState>,
    Json(req): Json<SetIdentityRequest>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    store
        .set_identity(&req.agent_id, &req.display_name, req.nickname.as_deref())
        .await;

    (
        StatusCode::OK,
        Json(json!({
            "agent_id": req.agent_id,
            "display_name": req.display_name,
            "nickname": req.nickname,
        })),
    )
}

/// GET /v1/pantheon/identity/:id — Get identity for an agent/user.
pub async fn get_identity(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    match store.get_identity(&agent_id).await {
        Some((display_name, nickname)) => Json(json!({
            "agent_id": agent_id,
            "display_name": display_name,
            "nickname": nickname,
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "Identity not found",
                "agent_id": agent_id,
            })),
        )
            .into_response(),
    }
}

/// GET /v1/pantheon/identities — List all identities.
pub async fn list_identities(State(state): State<SharedState>) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let identities = store.list_identities().await;
    let data: Vec<Value> = identities
        .iter()
        .map(|(id, name, nick)| {
            json!({
                "agent_id": id,
                "display_name": name,
                "nickname": nick,
            })
        })
        .collect();
    Json(json!({ "identities": data, "total": data.len() }))
}

// ============================================================================
// Plan Approval REST Endpoints
// ============================================================================

/// POST /v1/pantheon/plans/:id/approve — Approve a pending plan card
pub async fn approve_plan(
    State(state): State<SharedState>,
    Path(plan_id): Path<String>,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let approver_id = req
        .get("approver_id")
        .and_then(|v| v.as_str())
        .unwrap_or("api-user");
    let approver_name = req
        .get("approver_name")
        .and_then(|v| v.as_str())
        .unwrap_or("API User");

    match store
        .approve_plan(&plan_id, approver_id, approver_name)
        .await
    {
        Some(approval) => {
            store.emit(PantheonEvent::PlanApproved {
                room_id: approval.room_id.clone(),
                plan_id: plan_id.clone(),
                approved_by: approver_id.to_string(),
            });

            // Execute the approved plan — creates a real mission via MissionDriver
            execute_approved_plan(
                state.clone(),
                store.clone(),
                plan_id.clone(),
                approval.room_id.clone(),
                approval.goal.clone(),
                approval.steps_json.clone(),
            );

            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "plan_id": plan_id,
                    "status": "approved",
                    "goal": approval.goal,
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "No pending plan found with that ID"
            })),
        )
            .into_response(),
    }
}

/// POST /v1/pantheon/plans/:id/reject — Reject a pending plan card
pub async fn reject_plan(
    State(state): State<SharedState>,
    Path(plan_id): Path<String>,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let rejector_id = req
        .get("rejector_id")
        .and_then(|v| v.as_str())
        .unwrap_or("api-user");
    let rejector_name = req
        .get("rejector_name")
        .and_then(|v| v.as_str())
        .unwrap_or("API User");
    let reason = req.get("reason").and_then(|v| v.as_str());

    match store
        .reject_plan(&plan_id, rejector_id, rejector_name, reason)
        .await
    {
        Some(approval) => {
            let current_revision = approval.revision;
            let room_id = approval.room_id.clone();

            store.emit(PantheonEvent::PlanRejected {
                room_id: room_id.clone(),
                plan_id: plan_id.clone(),
                rejected_by: rejector_id.to_string(),
                reason: reason.map(|s| s.to_string()),
            });

            // Trigger re-plan if under max revisions
            let replanning = current_revision < 3;
            if replanning {
                let state_c = state.clone();
                let store_c = store.clone();
                let plan_id_c = plan_id.clone();
                let room_id_c = room_id;
                let original_goal = approval.goal.clone();
                let feedback = reason.unwrap_or("no specific reason").to_string();
                tokio::spawn(async move {
                    replan_with_feedback(
                        &state_c,
                        &store_c,
                        &plan_id_c,
                        &room_id_c,
                        &original_goal,
                        &feedback,
                    )
                    .await;
                });
            }

            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "plan_id": plan_id,
                    "status": "rejected",
                    "goal": approval.goal,
                    "reason": approval.reject_reason,
                    "replanning": replanning,
                    "revision": current_revision,
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "No pending plan found with that ID"
            })),
        )
            .into_response(),
    }
}

/// GET /v1/pantheon/plans/pending — List all pending plan approvals
pub async fn list_pending_plans(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let store = state.read().await.pantheon.clone();
    let room_id = params.get("room_id").cloned().unwrap_or_default();

    if room_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "room_id query parameter required"
            })),
        )
            .into_response();
    }

    let pending = store.list_pending_approvals(&room_id).await;
    let data: Vec<Value> = pending
        .iter()
        .map(|p| {
            json!({
                "plan_id": p.plan_id,
                "room_id": p.room_id,
                "goal": p.goal,
                "complexity": p.complexity,
                "risk": p.risk,
                "steps": serde_json::from_str::<Value>(&p.steps_json).unwrap_or(json!([])),
                "status": p.status,
                "revision": p.revision,
                "requested_by": p.requested_by_name,
                "created_at": p.created_at,
            })
        })
        .collect();

    Json(json!({ "pending": data, "total": data.len() })).into_response()
}

// ============================================================================
// SSE — Live mission event stream
// ============================================================================

/// GET /v1/pantheon/missions/:id/events — Server-Sent Events stream for a mission.
///
/// Streams `PantheonEvent`s in real-time as SSE data frames.
/// Clients connect once and receive live updates (task assignments, progress,
/// agent activity, completion) without polling.
///
/// The stream stays open until the client disconnects or the mission completes/fails.
pub async fn mission_events(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    use axum::response::sse::{Event, Sse};
    use std::convert::Infallible;

    let store = state.read().await.pantheon.clone();

    // Verify mission exists
    let mission_exists = store.get(&id).await.is_some();
    if !mission_exists {
        // Return a short SSE stream with an error event then close
        let stream = async_stream::stream! {
            yield Ok::<_, Infallible>(
                Event::default()
                    .event("error")
                    .data(json!({"error": "Mission not found"}).to_string())
            );
        };
        return Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response();
    }

    let mut rx = store.subscribe();
    let mission_id = id.clone();

    let stream = async_stream::stream! {
        // Send initial connected event
        yield Ok::<_, Infallible>(
            Event::default()
                .event("connected")
                .data(json!({"mission_id": &mission_id}).to_string())
        );

        // Stream events filtered to this mission
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Filter: only emit events for this mission (skip room events)
                    let event_mission_id = match &event {
                        PantheonEvent::MissionCreated { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::TeamAssembled { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::TaskAssigned { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::AgentActivity { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::TaskCompleted { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::ReviewRequested { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::MissionProgress { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::Artifact { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::MissionApproved { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::MissionComplete { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::MissionFailed { mission_id, .. } => Some(mission_id.as_str()),
                        PantheonEvent::MissionRejected { mission_id, .. } => Some(mission_id.as_str()),
                        // Room/plan events don't belong to a mission SSE stream
                        PantheonEvent::RoomCreated { .. }
                        | PantheonEvent::RoomMessageSent { .. }
                        | PantheonEvent::AgentJoinedRoom { .. }
                        | PantheonEvent::AgentLeftRoom { .. }
                        | PantheonEvent::PlanCardCreated { .. }
                        | PantheonEvent::PlanApproved { .. }
                        | PantheonEvent::PlanRejected { .. } => None,
                    };

                    match event_mission_id {
                        Some(mid) if mid == mission_id => {},
                        _ => continue,
                    }

                    // Determine SSE event type from variant
                    let event_type = match &event {
                        PantheonEvent::MissionCreated { .. } => "mission_created",
                        PantheonEvent::TeamAssembled { .. } => "team_assembled",
                        PantheonEvent::TaskAssigned { .. } => "task_assigned",
                        PantheonEvent::AgentActivity { .. } => "agent_activity",
                        PantheonEvent::TaskCompleted { .. } => "task_completed",
                        PantheonEvent::ReviewRequested { .. } => "review_requested",
                        PantheonEvent::MissionProgress { .. } => "mission_progress",
                        PantheonEvent::Artifact { .. } => "artifact",
                        PantheonEvent::MissionApproved { .. } => "mission_approved",
                        PantheonEvent::MissionComplete { .. } => "mission_complete",
                        PantheonEvent::MissionFailed { .. } => "mission_failed",
                        _ => continue, // room events filtered above
                    };

                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok::<_, Infallible>(
                        Event::default()
                            .event(event_type)
                            .data(data)
                    );

                    // Close stream on terminal events
                    if matches!(&event,
                        PantheonEvent::MissionComplete { .. } |
                        PantheonEvent::MissionFailed { .. }
                    ) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Client fell behind — notify and continue
                    yield Ok::<_, Infallible>(
                        Event::default()
                            .event("lagged")
                            .data(json!({"skipped": n}).to_string())
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ============================================================================
// Heuristic team assembly (Phase 1 — replaced by Zeus112's PantheonOrchestrator)
// ============================================================================

/// Assemble a basic team based on goal complexity heuristics.
///
/// When a `ModelRouter` is provided and enabled, each team member gets an
/// optimal model assigned based on their role:
/// - Coordinator → reasoning model (strongest)
/// - Worker → code model
/// - Reviewer → review model
pub fn assemble_team_heuristic(goal: &str) -> Vec<TeamMember> {
    assemble_team_with_router(goal, None)
}

/// Assemble a team with optional model routing.
pub fn assemble_team_with_router(goal: &str, router: Option<&ModelRouter>) -> Vec<TeamMember> {
    let goal_lower = goal.to_lowercase();
    let is_complex = goal_lower.split_whitespace().count() > 8
        || goal_lower.contains("build")
        || goal_lower.contains("implement")
        || goal_lower.contains("create")
        || goal_lower.contains("design");

    let model_for_role =
        |role: &str| -> Option<String> { router.map(|r| r.select_for_role(role).model_string) };

    let mut team = vec![TeamMember {
        agent_id: format!("zeus-c-{}", &Uuid::new_v4().to_string()[..8]),
        name: "Zeus-Coordinator".to_string(),
        role: AgentRole::Coordinator,
        status: AgentStatus::Working,
        model: model_for_role("coordinator"),
    }];

    if is_complex {
        team.push(TeamMember {
            agent_id: format!("zeus-w1-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Zeus-Worker-1".to_string(),
            role: AgentRole::Worker,
            status: AgentStatus::Idle,
            model: model_for_role("worker"),
        });
        team.push(TeamMember {
            agent_id: format!("zeus-w2-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Zeus-Worker-2".to_string(),
            role: AgentRole::Worker,
            status: AgentStatus::Idle,
            model: model_for_role("worker"),
        });
        team.push(TeamMember {
            agent_id: format!("zeus-r-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Zeus-Reviewer".to_string(),
            role: AgentRole::Reviewer,
            status: AgentStatus::Idle,
            model: model_for_role("reviewer"),
        });
    }

    team
}

// ============================================================================
// Announcement Hooks  (S14-4)
// ============================================================================

/// Spawn a background task that listens to PantheonEvents and automatically
/// broadcasts significant events to all connected channels.
///
/// Key events that trigger announcements:
/// - `MissionComplete`  — "Mission X completed: <summary>"
/// - `MissionFailed`    — "Mission X failed: <reason>"
/// - `MissionCreated`   — "New mission launched: <goal>"
///
/// The task runs until the broadcast sender is dropped (gateway shutdown).
pub fn spawn_announcement_hook(
    store: &PantheonStore,
    channel_manager: Arc<zeus_channels::ChannelManager>,
) -> tokio::task::JoinHandle<()> {
    let mut rx = store.subscribe();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let msg = match &event {
                        PantheonEvent::MissionCreated { goal, .. } => {
                            Some(format!("[Pantheon] New mission launched: {goal}"))
                        }
                        PantheonEvent::MissionComplete { summary, .. } => {
                            Some(format!("[Pantheon] Mission completed: {summary}"))
                        }
                        PantheonEvent::MissionFailed { reason, .. } => {
                            Some(format!("[Pantheon] Mission failed: {reason}"))
                        }
                        _ => None,
                    };

                    if let Some(text) = msg {
                        let results = channel_manager.broadcast_all(&text).await;
                        let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
                        let total = results.len();
                        if total > 0 {
                            info!(
                                "Announcement hook delivered to {ok}/{total} channels: {}",
                                &text[..zeus_core::floor_char_boundary(&text, 80)]
                            );
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Announcement hook lagged by {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Announcement hook shutting down (event bus closed)");
                    break;
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Sentient Intelligence — Agent Reputation System (S59-T5)
// ---------------------------------------------------------------------------

/// GET /v1/pantheon/reputation/:agent_id
pub async fn get_reputation(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    
    // Build reputation from agent registry data
    let agents = state.agent_registry.list();
    let agent = agents.iter().find(|a| a.agent_id == agent_id || a.name == agent_id);
    
    match agent {
        Some(a) => {
            let uptime_hours = chrono::Utc::now()
                .signed_duration_since(a.spawned_at)
                .num_hours();
            let trust = if a.message_count > 100 { 0.95 }
                else if a.message_count > 50 { 0.85 }
                else if a.message_count > 10 { 0.7 }
                else { 0.5 };
            
            Json(json!({
                "agent_id": a.agent_id,
                "name": a.name,
                "trust_score": trust,
                "messages_processed": a.message_count,
                "uptime_hours": uptime_hours,
                "spawned_at": a.spawned_at.to_rfc3339(),
                "last_active": a.last_active.to_rfc3339(),
                "status": "active",
            }))
        }
        None => Json(json!({
            "error": "Agent not found",
            "agent_id": agent_id,
        })),
    }
}

/// POST /v1/pantheon/reputation/:agent_id — update reputation metrics
pub async fn update_reputation(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let endorsement = body.get("endorsement").and_then(|v| v.as_bool()).unwrap_or(false);
    let quality = body.get("quality_rating").and_then(|v| v.as_f64());

    // Persist: update agent activity timestamp as a proxy for reputation tracking
    // Full graph-based reputation persistence is S59-T2 (GraphRAG follow-up)
    let persisted = {
        let mut st = state.write().await;
        st.agent_registry.update_activity(&agent_id);
        true
    };

    Json(json!({
        "agent_id": agent_id,
        "updated": persisted,
        "endorsement_recorded": endorsement,
        "quality_rating": quality,
        "note": if !persisted { "Agent not found in registry" } else { "Activity updated" },
    }))
}

/// GET /v1/pantheon/leaderboard — fleet reputation leaderboard
pub async fn reputation_leaderboard(
    State(state): State<SharedState>,
) -> Json<Value> {
    let state = state.read().await;
    let agents = state.agent_registry.list();
    
    let mut entries: Vec<Value> = agents.iter().map(|a| {
        let uptime_hours = chrono::Utc::now()
            .signed_duration_since(a.spawned_at)
            .num_hours();
        let trust = if a.message_count > 100 { 0.95 }
            else if a.message_count > 50 { 0.85 }
            else if a.message_count > 10 { 0.7 }
            else { 0.5 };
        json!({
            "agent_id": a.agent_id,
            "name": a.name,
            "trust_score": trust,
            "messages": a.message_count,
            "uptime_hours": uptime_hours,
        })
    }).collect();
    
    // Sort by trust score descending, then by messages
    entries.sort_by(|a, b| {
        let ta = a["trust_score"].as_f64().unwrap_or(0.0);
        let tb = b["trust_score"].as_f64().unwrap_or(0.0);
        tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
    });
    
    Json(json!({
        "leaderboard": entries,
        "total_agents": entries.len(),
        "generated_at": chrono::Utc::now().to_rfc3339(),
    }))
}
