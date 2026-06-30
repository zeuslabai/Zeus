//! Pantheon — Multi-agent collaboration mission engine
//!
//! Provides the core mission lifecycle:
//! - **Mission** — goal + constraints + state machine
//! - **MissionTask** — decomposed subtask assigned to an agent
//! - **PantheonOrchestrator** — assembles teams, delegates tasks, tracks progress
//! - **MissionEvent** — real-time events broadcast to frontends via WebSocket
//!
//! Integration points:
//! - `zeus-prometheus` — Planner decomposes goals into DAGs, Executor runs them
//! - `zeus-nous` — complexity assessment for auto-delegation decisions
//! - `zeus-mnemosyne` — team-scoped shared memory
//! - `zeus-orchestra::MessageBus` — internal agent-to-agent routing

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use crate::state::AgentStatus;
use crate::{GlobalStateManager, MessageBus, OrchestraError};

// ---------------------------------------------------------------------------
// Mission State Machine
// ---------------------------------------------------------------------------

/// Mission lifecycle states.
///
/// ```text
/// Created → Assembling → Executing → Reviewing → Completed
///                ↓            ↓           ↓
///              Failed       Failed      Failed
///                            ↓
///                          Paused → Executing (resume)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    /// Mission created, not yet started.
    Created,
    /// Assembling team (spawning agents, assigning roles).
    Assembling,
    /// Plan assembled, awaiting supervisor approval before execution.
    AwaitingApproval,
    /// Team assembled, tasks being executed.
    Executing,
    /// Execution paused by user intervention.
    Paused,
    /// All tasks done, under review.
    Reviewing,
    /// Mission completed successfully.
    Completed,
    /// Mission failed.
    Failed,
    /// Mission cancelled by user.
    Cancelled,
}

impl std::fmt::Display for MissionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Assembling => write!(f, "assembling"),
            Self::AwaitingApproval => write!(f, "awaiting_approval"),
            Self::Executing => write!(f, "executing"),
            Self::Paused => write!(f, "paused"),
            Self::Reviewing => write!(f, "reviewing"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

// ---------------------------------------------------------------------------
// Agent Roles
// ---------------------------------------------------------------------------

/// Role an agent plays within a mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Decomposes goals, creates teams, assigns work, makes final decisions.
    Coordinator,
    /// Coordinates a sub-team, reports to coordinator.
    Manager,
    /// Executes assigned tasks, reports results.
    Worker,
    /// Validates worker output, approves/rejects.
    Reviewer,
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Coordinator => write!(f, "coordinator"),
            Self::Manager => write!(f, "manager"),
            Self::Worker => write!(f, "worker"),
            Self::Reviewer => write!(f, "reviewer"),
        }
    }
}

// ---------------------------------------------------------------------------
// Mission Task
// ---------------------------------------------------------------------------

/// State of an individual task within a mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Assigned,
    InProgress,
    Review,
    RevisionNeeded,
    Approved,
    Completed,
    Failed,
}

/// A decomposed subtask within a mission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTask {
    pub id: String,
    pub mission_id: String,
    pub description: String,
    pub state: TaskState,
    pub assigned_to: Option<String>,
    pub reviewer: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub dependencies: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub tokens_used: u64,
}

impl MissionTask {
    pub fn new(mission_id: &str, description: impl Into<String>) -> Self {
        Self {
            id: format!("t-{}", &Uuid::new_v4().to_string()[..8]),
            mission_id: mission_id.to_string(),
            description: description.into(),
            state: TaskState::Pending,
            assigned_to: None,
            reviewer: None,
            result: None,
            error: None,
            dependencies: Vec::new(),
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            tokens_used: 0,
        }
    }

    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    pub fn assign(&mut self, agent_id: &str) {
        self.assigned_to = Some(agent_id.to_string());
        self.state = TaskState::Assigned;
    }

    pub fn start(&mut self) {
        self.state = TaskState::InProgress;
        self.started_at = Some(Utc::now());
    }

    pub fn complete(&mut self, result: String) {
        self.state = TaskState::Completed;
        self.result = Some(result);
        self.completed_at = Some(Utc::now());
    }

    pub fn fail(&mut self, error: String) {
        self.state = TaskState::Failed;
        self.error = Some(error);
        self.completed_at = Some(Utc::now());
    }

    pub fn submit_for_review(&mut self, reviewer_id: &str) {
        self.state = TaskState::Review;
        self.reviewer = Some(reviewer_id.to_string());
    }

    pub fn approve(&mut self) {
        self.state = TaskState::Approved;
        self.completed_at = Some(Utc::now());
    }

    pub fn request_revision(&mut self) {
        self.state = TaskState::RevisionNeeded;
    }

    /// Check if all dependencies are in a completed/approved state.
    pub fn dependencies_met(&self, tasks: &[MissionTask]) -> bool {
        self.dependencies.iter().all(|dep_id| {
            tasks.iter().any(|t| {
                &t.id == dep_id && matches!(t.state, TaskState::Completed | TaskState::Approved)
            })
        })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TaskState::Completed | TaskState::Approved | TaskState::Failed
        )
    }
}

// ---------------------------------------------------------------------------
// Mission Artifact
// ---------------------------------------------------------------------------

/// An output artifact produced during a mission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionArtifact {
    pub id: String,
    pub mission_id: String,
    pub task_id: String,
    pub name: String,
    pub path: Option<String>,
    pub artifact_type: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Team Member
// ---------------------------------------------------------------------------

/// An agent participating in a mission with an assigned role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub agent_id: String,
    pub name: String,
    pub role: AgentRole,
    pub capabilities: Vec<String>,
    pub joined_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Mission Constraints
// ---------------------------------------------------------------------------

/// User-defined constraints for a mission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionConstraints {
    /// Maximum token budget across all agents.
    #[serde(default = "default_budget")]
    pub budget_tokens: u64,
    /// Maximum wall-clock seconds.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Maximum number of agents to spawn.
    #[serde(default = "default_max_agents")]
    pub max_agents: u32,
    /// Whether human review is required before completion.
    #[serde(default)]
    pub require_review: bool,
}

fn default_budget() -> u64 {
    50_000
}
fn default_timeout() -> u64 {
    600
}
fn default_max_agents() -> u32 {
    5
}

impl Default for MissionConstraints {
    fn default() -> Self {
        Self {
            budget_tokens: default_budget(),
            timeout_seconds: default_timeout(),
            max_agents: default_max_agents(),
            require_review: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Mission
// ---------------------------------------------------------------------------

/// A Pantheon mission — the top-level unit of collaborative work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub goal: String,
    pub state: MissionState,
    pub constraints: MissionConstraints,
    pub team: Vec<TeamMember>,
    pub tasks: Vec<MissionTask>,
    pub artifacts: Vec<MissionArtifact>,
    pub tokens_used: u64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub error: Option<String>,
}

impl Mission {
    pub fn new(goal: impl Into<String>, constraints: MissionConstraints) -> Self {
        Self {
            id: format!("m-{}", &Uuid::new_v4().to_string()[..8]),
            goal: goal.into(),
            state: MissionState::Created,
            constraints,
            team: Vec::new(),
            tasks: Vec::new(),
            artifacts: Vec::new(),
            tokens_used: 0,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            summary: None,
            error: None,
        }
    }

    /// Progress as a percentage (completed tasks / total tasks).
    pub fn progress_pct(&self) -> f64 {
        if self.tasks.is_empty() {
            return 0.0;
        }
        let done = self.tasks.iter().filter(|t| t.is_terminal()).count();
        (done as f64 / self.tasks.len() as f64) * 100.0
    }

    pub fn tasks_done(&self) -> usize {
        self.tasks.iter().filter(|t| t.is_terminal()).count()
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            MissionState::Completed | MissionState::Failed | MissionState::Cancelled
        )
    }

    /// Check if budget has been exceeded.
    pub fn over_budget(&self) -> bool {
        self.tokens_used > self.constraints.budget_tokens
    }

    /// Check if timeout has been exceeded.
    pub fn timed_out(&self) -> bool {
        if let Some(started) = self.started_at {
            let elapsed = (Utc::now() - started).num_seconds() as u64;
            elapsed > self.constraints.timeout_seconds
        } else {
            false
        }
    }

    /// Transition the mission state, returning error if transition is invalid.
    pub fn transition(&mut self, new_state: MissionState) -> Result<(), OrchestraError> {
        let valid = match (&self.state, &new_state) {
            (MissionState::Created, MissionState::Assembling) => true,
            (MissionState::Assembling, MissionState::Executing) => true,
            (MissionState::Assembling, MissionState::AwaitingApproval) => true,
            (MissionState::Assembling, MissionState::Failed) => true,
            (MissionState::AwaitingApproval, MissionState::Executing) => true,
            (MissionState::AwaitingApproval, MissionState::Cancelled) => true,
            (MissionState::Executing, MissionState::Reviewing) => true,
            (MissionState::Executing, MissionState::Paused) => true,
            (MissionState::Executing, MissionState::Failed) => true,
            (MissionState::Executing, MissionState::Cancelled) => true,
            (MissionState::Paused, MissionState::Executing) => true,
            (MissionState::Paused, MissionState::Cancelled) => true,
            (MissionState::Reviewing, MissionState::Completed) => true,
            (MissionState::Reviewing, MissionState::Failed) => true,
            // Allow direct completion from executing (no review required)
            (MissionState::Executing, MissionState::Completed) => true,
            _ => false,
        };

        if !valid {
            return Err(OrchestraError::PolicyViolation(format!(
                "invalid mission transition: {} → {}",
                self.state, new_state
            )));
        }

        // Update timestamps
        match new_state {
            MissionState::Executing if self.started_at.is_none() => {
                self.started_at = Some(Utc::now());
            }
            MissionState::Completed | MissionState::Failed | MissionState::Cancelled => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }

        self.state = new_state;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mission Events (for WebSocket broadcast)
// ---------------------------------------------------------------------------

/// Events emitted during mission execution, broadcast to all WS clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MissionEvent {
    /// Mission created.
    MissionCreated { mission_id: String, goal: String },
    /// Team has been assembled.
    TeamAssembled {
        mission_id: String,
        agents: Vec<TeamMember>,
    },
    /// A task was assigned to an agent.
    TaskAssigned {
        mission_id: String,
        task_id: String,
        agent_id: String,
        description: String,
    },
    /// An agent is performing an activity (tool call, thinking, etc).
    AgentActivity {
        mission_id: String,
        agent_id: String,
        activity: String,
        detail: serde_json::Value,
    },
    /// A task was completed.
    TaskCompleted {
        mission_id: String,
        task_id: String,
        result: String,
    },
    /// A task failed.
    TaskFailed {
        mission_id: String,
        task_id: String,
        error: String,
    },
    /// Review was requested for a task.
    ReviewRequested {
        mission_id: String,
        task_id: String,
        reviewer: String,
    },
    /// Mission progress update.
    MissionProgress {
        mission_id: String,
        progress_pct: f64,
        tasks_done: usize,
        tasks_total: usize,
        tokens_used: u64,
    },
    /// An artifact was produced.
    ArtifactCreated {
        mission_id: String,
        task_id: String,
        name: String,
        artifact_type: String,
    },
    /// Mission completed.
    MissionComplete {
        mission_id: String,
        status: String,
        summary: String,
        artifacts: Vec<MissionArtifact>,
    },
    /// Mission failed.
    MissionFailed { mission_id: String, error: String },
    /// User intervention applied.
    Intervention {
        mission_id: String,
        action: String,
        message: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// User Intervention
// ---------------------------------------------------------------------------

/// Actions a user can take on an active mission.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionAction {
    Pause,
    Resume,
    Cancel,
    Redirect { message: String },
    ApproveTask { task_id: String },
    RejectTask { task_id: String, reason: String },
}

// ---------------------------------------------------------------------------
// Pantheon Orchestrator
// ---------------------------------------------------------------------------

/// The Pantheon orchestrator — manages mission lifecycle, team assembly,
/// and coordination between agents.
pub struct PantheonOrchestrator {
    /// Active missions keyed by mission ID.
    missions: Arc<RwLock<HashMap<String, Mission>>>,
    /// Agent state manager for finding and assigning agents.
    state_manager: Arc<GlobalStateManager>,
    /// Internal message bus for agent-to-agent communication.
    _message_bus: Arc<MessageBus>,
    /// Event broadcast channel for WebSocket clients.
    event_tx: broadcast::Sender<MissionEvent>,
}

impl PantheonOrchestrator {
    pub fn new(state_manager: Arc<GlobalStateManager>, message_bus: Arc<MessageBus>) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            missions: Arc::new(RwLock::new(HashMap::new())),
            state_manager,
            _message_bus: message_bus,
            event_tx,
        }
    }

    /// Subscribe to mission events (for WebSocket broadcast).
    pub fn subscribe_events(&self) -> broadcast::Receiver<MissionEvent> {
        self.event_tx.subscribe()
    }

    /// Create a new mission from a user goal.
    pub async fn create_mission(
        &self,
        goal: impl Into<String>,
        constraints: MissionConstraints,
    ) -> Result<Mission, OrchestraError> {
        let mission = Mission::new(goal, constraints);
        let id = mission.id.clone();

        self.emit(MissionEvent::MissionCreated {
            mission_id: id.clone(),
            goal: mission.goal.clone(),
        });

        self.missions
            .write()
            .await
            .insert(id.clone(), mission.clone());
        Ok(mission)
    }

    /// Get a mission by ID.
    pub async fn get_mission(&self, mission_id: &str) -> Option<Mission> {
        self.missions.read().await.get(mission_id).cloned()
    }

    /// List all missions.
    pub async fn list_missions(&self) -> Vec<Mission> {
        self.missions.read().await.values().cloned().collect()
    }

    /// Assemble a team for a mission based on required capabilities.
    ///
    /// Finds available agents, assigns roles (one coordinator, workers,
    /// optionally a reviewer), and transitions to Executing state.
    pub async fn assemble_team(
        &self,
        mission_id: &str,
        required_capabilities: Vec<String>,
    ) -> Result<Vec<TeamMember>, OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        mission.transition(MissionState::Assembling)?;

        let max_agents = mission.constraints.max_agents as usize;
        let mut team = Vec::new();

        // Find the best coordinator (highest health, has broadest capabilities)
        if let Some(coord) = self.state_manager.best_for_task("coordinate").await {
            self.state_manager
                .update_status(
                    &coord.id,
                    AgentStatus::Busy(format!("mission:{}", mission_id)),
                )
                .await?;
            team.push(TeamMember {
                agent_id: coord.id.clone(),
                name: coord.name.clone(),
                role: AgentRole::Coordinator,
                capabilities: coord.capabilities.clone(),
                joined_at: Utc::now(),
            });
        }

        // Find workers for each required capability
        for cap in &required_capabilities {
            if team.len() >= max_agents {
                break;
            }
            // Skip if we already have an agent with this capability
            if team.iter().any(|m| m.capabilities.contains(cap)) {
                continue;
            }
            if let Some(agent) = self.state_manager.best_for_task(cap).await {
                // Don't assign same agent twice
                if team.iter().any(|m| m.agent_id == agent.id) {
                    continue;
                }
                self.state_manager
                    .update_status(
                        &agent.id,
                        AgentStatus::Busy(format!("mission:{}", mission_id)),
                    )
                    .await?;
                team.push(TeamMember {
                    agent_id: agent.id.clone(),
                    name: agent.name.clone(),
                    role: AgentRole::Worker,
                    capabilities: agent.capabilities.clone(),
                    joined_at: Utc::now(),
                });
            }
        }

        // Optionally assign a reviewer if review is required
        if mission.constraints.require_review
            && team.len() < max_agents
            && let Some(reviewer) = self.state_manager.best_for_task("review").await
            && !team.iter().any(|m| m.agent_id == reviewer.id)
        {
            self.state_manager
                .update_status(
                    &reviewer.id,
                    AgentStatus::Busy(format!("mission:{}", mission_id)),
                )
                .await?;
            team.push(TeamMember {
                agent_id: reviewer.id.clone(),
                name: reviewer.name.clone(),
                role: AgentRole::Reviewer,
                capabilities: reviewer.capabilities.clone(),
                joined_at: Utc::now(),
            });
        }

        mission.team = team.clone();

        self.emit(MissionEvent::TeamAssembled {
            mission_id: mission_id.to_string(),
            agents: team.clone(),
        });

        Ok(team)
    }

    /// Add decomposed tasks to a mission.
    pub async fn add_tasks(
        &self,
        mission_id: &str,
        tasks: Vec<MissionTask>,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        mission.tasks.extend(tasks);
        Ok(())
    }

    /// Assign a task to an agent and emit the event.
    pub async fn assign_task(
        &self,
        mission_id: &str,
        task_id: &str,
        agent_id: &str,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        let task = mission
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| {
                OrchestraError::PolicyViolation(format!(
                    "task {} not found in mission {}",
                    task_id, mission_id
                ))
            })?;

        task.assign(agent_id);

        self.emit(MissionEvent::TaskAssigned {
            mission_id: mission_id.to_string(),
            task_id: task_id.to_string(),
            agent_id: agent_id.to_string(),
            description: task.description.clone(),
        });

        Ok(())
    }

    /// Mark a task as in-progress.
    pub async fn start_task(&self, mission_id: &str, task_id: &str) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        // Transition to executing if still assembling
        if mission.state == MissionState::Assembling {
            mission.transition(MissionState::Executing)?;
        }

        let task = mission
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| {
                OrchestraError::PolicyViolation(format!("task {} not found", task_id))
            })?;

        task.start();
        Ok(())
    }

    /// Record an agent activity event (tool call, thinking, etc).
    pub fn record_activity(
        &self,
        mission_id: &str,
        agent_id: &str,
        activity: &str,
        detail: serde_json::Value,
    ) {
        self.emit(MissionEvent::AgentActivity {
            mission_id: mission_id.to_string(),
            agent_id: agent_id.to_string(),
            activity: activity.to_string(),
            detail,
        });
    }

    /// Complete a task and emit progress.
    pub async fn complete_task(
        &self,
        mission_id: &str,
        task_id: &str,
        result: String,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        let task = mission
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| {
                OrchestraError::PolicyViolation(format!("task {} not found", task_id))
            })?;

        task.complete(result.clone());

        // Release the agent
        if let Some(agent_id) = &task.assigned_to {
            let _ = self
                .state_manager
                .update_status(agent_id, AgentStatus::Idle)
                .await;
        }

        self.emit(MissionEvent::TaskCompleted {
            mission_id: mission_id.to_string(),
            task_id: task_id.to_string(),
            result,
        });

        // Emit progress
        self.emit(MissionEvent::MissionProgress {
            mission_id: mission_id.to_string(),
            progress_pct: mission.progress_pct(),
            tasks_done: mission.tasks_done(),
            tasks_total: mission.tasks.len(),
            tokens_used: mission.tokens_used,
        });

        // Check if all tasks are done
        if mission.tasks.iter().all(|t| t.is_terminal()) {
            self.finalize_mission(mission)?;
        }

        Ok(())
    }

    /// Fail a task.
    pub async fn fail_task(
        &self,
        mission_id: &str,
        task_id: &str,
        error: String,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        let task = mission
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| {
                OrchestraError::PolicyViolation(format!("task {} not found", task_id))
            })?;

        task.fail(error.clone());

        // Release the agent
        if let Some(agent_id) = &task.assigned_to {
            let _ = self
                .state_manager
                .update_status(agent_id, AgentStatus::Idle)
                .await;
        }

        self.emit(MissionEvent::TaskFailed {
            mission_id: mission_id.to_string(),
            task_id: task_id.to_string(),
            error,
        });

        Ok(())
    }

    /// Apply user intervention to a mission.
    pub async fn intervene(
        &self,
        mission_id: &str,
        action: InterventionAction,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        let (action_str, message) = match &action {
            InterventionAction::Pause => {
                mission.transition(MissionState::Paused)?;
                ("pause".to_string(), None)
            }
            InterventionAction::Resume => {
                mission.transition(MissionState::Executing)?;
                ("resume".to_string(), None)
            }
            InterventionAction::Cancel => {
                mission.transition(MissionState::Cancelled)?;
                // Release all team agents
                for member in &mission.team {
                    let _ = self
                        .state_manager
                        .update_status(&member.agent_id, AgentStatus::Idle)
                        .await;
                }
                ("cancel".to_string(), None)
            }
            InterventionAction::Redirect { message } => {
                ("redirect".to_string(), Some(message.clone()))
            }
            InterventionAction::ApproveTask { task_id } => {
                if let Some(task) = mission.tasks.iter_mut().find(|t| t.id == *task_id) {
                    task.approve();
                }
                ("approve_task".to_string(), Some(task_id.clone()))
            }
            InterventionAction::RejectTask { task_id, reason } => {
                if let Some(task) = mission.tasks.iter_mut().find(|t| t.id == *task_id) {
                    task.request_revision();
                }
                (
                    "reject_task".to_string(),
                    Some(format!("{}: {}", task_id, reason)),
                )
            }
        };

        self.emit(MissionEvent::Intervention {
            mission_id: mission_id.to_string(),
            action: action_str,
            message,
        });

        Ok(())
    }

    /// Add an artifact to a mission.
    pub async fn add_artifact(
        &self,
        mission_id: &str,
        task_id: &str,
        name: String,
        path: Option<String>,
        artifact_type: String,
    ) -> Result<(), OrchestraError> {
        let mut missions = self.missions.write().await;
        let mission = missions.get_mut(mission_id).ok_or_else(|| {
            OrchestraError::PolicyViolation(format!("mission {} not found", mission_id))
        })?;

        let artifact = MissionArtifact {
            id: format!("a-{}", &Uuid::new_v4().to_string()[..8]),
            mission_id: mission_id.to_string(),
            task_id: task_id.to_string(),
            name: name.clone(),
            path,
            artifact_type: artifact_type.clone(),
            created_at: Utc::now(),
        };

        mission.artifacts.push(artifact);

        self.emit(MissionEvent::ArtifactCreated {
            mission_id: mission_id.to_string(),
            task_id: task_id.to_string(),
            name,
            artifact_type,
        });

        Ok(())
    }

    /// Get the activity feed for a mission (all events).
    pub async fn mission_count(&self) -> usize {
        self.missions.read().await.len()
    }

    /// Get active (non-terminal) missions.
    pub async fn active_missions(&self) -> Vec<Mission> {
        self.missions
            .read()
            .await
            .values()
            .filter(|m| !m.is_terminal())
            .cloned()
            .collect()
    }

    // -- Internal helpers ---------------------------------------------------

    fn finalize_mission(&self, mission: &mut Mission) -> Result<(), OrchestraError> {
        let all_succeeded = mission
            .tasks
            .iter()
            .all(|t| matches!(t.state, TaskState::Completed | TaskState::Approved));

        if all_succeeded {
            if mission.constraints.require_review {
                mission.transition(MissionState::Reviewing)?;
            } else {
                mission.transition(MissionState::Completed)?;
                let summary = format!(
                    "Mission '{}' completed: {}/{} tasks successful",
                    mission.goal,
                    mission.tasks_done(),
                    mission.tasks.len()
                );
                mission.summary = Some(summary.clone());

                self.emit(MissionEvent::MissionComplete {
                    mission_id: mission.id.clone(),
                    status: "success".to_string(),
                    summary,
                    artifacts: mission.artifacts.clone(),
                });
            }
        } else {
            mission.transition(MissionState::Failed)?;
            let failed_tasks: Vec<_> = mission
                .tasks
                .iter()
                .filter(|t| matches!(t.state, TaskState::Failed))
                .map(|t| t.id.clone())
                .collect();
            let error = format!("Tasks failed: {}", failed_tasks.join(", "));
            mission.error = Some(error.clone());

            self.emit(MissionEvent::MissionFailed {
                mission_id: mission.id.clone(),
                error,
            });
        }

        Ok(())
    }

    fn emit(&self, event: MissionEvent) {
        // broadcast::send returns Err only if there are no receivers,
        // which is fine — just means no WS clients are connected.
        let _ = self.event_tx.send(event);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentState;

    async fn setup() -> PantheonOrchestrator {
        let state_mgr = Arc::new(GlobalStateManager::new());
        let bus = Arc::new(MessageBus::new(64));

        // Register some test agents
        state_mgr
            .register_agent(
                AgentState::new("agent-1", "Zeus-C")
                    .with_capabilities(vec!["coordinate".into(), "code".into()]),
            )
            .await
            .unwrap();
        state_mgr
            .register_agent(
                AgentState::new("agent-2", "Zeus-W1")
                    .with_capabilities(vec!["code".into(), "review".into()]),
            )
            .await
            .unwrap();
        state_mgr
            .register_agent(
                AgentState::new("agent-3", "Zeus-W2")
                    .with_capabilities(vec!["code".into(), "test".into()]),
            )
            .await
            .unwrap();

        PantheonOrchestrator::new(state_mgr, bus)
    }

    #[tokio::test]
    async fn test_create_mission() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build a REST API", MissionConstraints::default())
            .await
            .unwrap();
        assert_eq!(mission.state, MissionState::Created);
        assert!(mission.id.starts_with("m-"));
    }

    #[tokio::test]
    async fn test_assemble_team() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build a REST API", MissionConstraints::default())
            .await
            .unwrap();
        let team = orch
            .assemble_team(&mission.id, vec!["code".into(), "test".into()])
            .await
            .unwrap();
        assert!(!team.is_empty());
        // Should have a coordinator
        assert!(team.iter().any(|m| m.role == AgentRole::Coordinator));
    }

    #[tokio::test]
    async fn test_mission_state_transitions() {
        let mut mission = Mission::new("test", MissionConstraints::default());
        assert!(mission.transition(MissionState::Assembling).is_ok());
        assert!(mission.transition(MissionState::Executing).is_ok());
        assert!(mission.transition(MissionState::Paused).is_ok());
        assert!(mission.transition(MissionState::Executing).is_ok());
        assert!(mission.transition(MissionState::Completed).is_ok());
        // Can't go back from completed
        assert!(mission.transition(MissionState::Executing).is_err());
    }

    #[tokio::test]
    async fn test_invalid_transition() {
        let mut mission = Mission::new("test", MissionConstraints::default());
        // Can't go directly to executing
        assert!(mission.transition(MissionState::Executing).is_err());
    }

    #[tokio::test]
    async fn test_task_lifecycle() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build API", MissionConstraints::default())
            .await
            .unwrap();
        orch.assemble_team(&mission.id, vec!["code".into()])
            .await
            .unwrap();

        // Add a task
        let task = MissionTask::new(&mission.id, "Implement endpoints");
        let task_id = task.id.clone();
        orch.add_tasks(&mission.id, vec![task]).await.unwrap();

        // Assign and start
        orch.assign_task(&mission.id, &task_id, "agent-2")
            .await
            .unwrap();
        orch.start_task(&mission.id, &task_id).await.unwrap();

        // Complete
        orch.complete_task(&mission.id, &task_id, "Done".to_string())
            .await
            .unwrap();

        let mission = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(mission.state, MissionState::Completed);
        assert!(mission.summary.is_some());
    }

    #[tokio::test]
    async fn test_intervention_pause_resume() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build API", MissionConstraints::default())
            .await
            .unwrap();
        orch.assemble_team(&mission.id, vec!["code".into()])
            .await
            .unwrap();

        let task = MissionTask::new(&mission.id, "Work");
        let task_id = task.id.clone();
        orch.add_tasks(&mission.id, vec![task]).await.unwrap();
        orch.start_task(&mission.id, &task_id).await.unwrap();

        // Pause
        orch.intervene(&mission.id, InterventionAction::Pause)
            .await
            .unwrap();
        let m = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(m.state, MissionState::Paused);

        // Resume
        orch.intervene(&mission.id, InterventionAction::Resume)
            .await
            .unwrap();
        let m = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(m.state, MissionState::Executing);
    }

    #[tokio::test]
    async fn test_intervention_cancel() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build API", MissionConstraints::default())
            .await
            .unwrap();
        orch.assemble_team(&mission.id, vec!["code".into()])
            .await
            .unwrap();

        let task = MissionTask::new(&mission.id, "Work");
        let task_id = task.id.clone();
        orch.add_tasks(&mission.id, vec![task]).await.unwrap();
        orch.start_task(&mission.id, &task_id).await.unwrap();

        orch.intervene(&mission.id, InterventionAction::Cancel)
            .await
            .unwrap();
        let m = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(m.state, MissionState::Cancelled);
        assert!(m.is_terminal());
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let orch = setup().await;
        let mut rx = orch.subscribe_events();

        orch.create_mission("Test", MissionConstraints::default())
            .await
            .unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, MissionEvent::MissionCreated { .. }));
    }

    #[tokio::test]
    async fn test_progress_tracking() {
        let mission = Mission::new("test", MissionConstraints::default());
        assert_eq!(mission.progress_pct(), 0.0);

        let orch = setup().await;
        let mission = orch
            .create_mission("Build API", MissionConstraints::default())
            .await
            .unwrap();
        orch.assemble_team(&mission.id, vec!["code".into()])
            .await
            .unwrap();

        let t1 = MissionTask::new(&mission.id, "Task 1");
        let t2 = MissionTask::new(&mission.id, "Task 2");
        let t1_id = t1.id.clone();
        let _t2_id = t2.id.clone();
        orch.add_tasks(&mission.id, vec![t1, t2]).await.unwrap();

        orch.assign_task(&mission.id, &t1_id, "agent-2")
            .await
            .unwrap();
        orch.start_task(&mission.id, &t1_id).await.unwrap();
        orch.complete_task(&mission.id, &t1_id, "Done".to_string())
            .await
            .unwrap();

        let m = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(m.progress_pct(), 50.0);
        assert_eq!(m.tasks_done(), 1);
    }

    #[tokio::test]
    async fn test_task_dependencies() {
        let mut t1 = MissionTask::new("m-1", "Schema");
        let t2 = MissionTask::new("m-1", "Endpoints").with_dependencies(vec![t1.id.clone()]);

        // t2 depends on t1, which isn't done yet
        assert!(!t2.dependencies_met(&[t1.clone(), t2.clone()]));

        // Complete t1
        t1.complete("Schema done".to_string());
        assert!(t2.dependencies_met(&[t1.clone(), t2.clone()]));
    }

    #[tokio::test]
    async fn test_artifact_creation() {
        let orch = setup().await;
        let mission = orch
            .create_mission("Build API", MissionConstraints::default())
            .await
            .unwrap();

        let task = MissionTask::new(&mission.id, "Generate code");
        let task_id = task.id.clone();
        orch.add_tasks(&mission.id, vec![task]).await.unwrap();

        orch.add_artifact(
            &mission.id,
            &task_id,
            "api.rs".to_string(),
            Some("/tmp/api.rs".to_string()),
            "code".to_string(),
        )
        .await
        .unwrap();

        let m = orch.get_mission(&mission.id).await.unwrap();
        assert_eq!(m.artifacts.len(), 1);
        assert_eq!(m.artifacts[0].name, "api.rs");
    }

    #[tokio::test]
    async fn test_mission_budget_timeout() {
        let mut mission = Mission::new(
            "test",
            MissionConstraints {
                budget_tokens: 100,
                timeout_seconds: 1,
                ..Default::default()
            },
        );
        mission.tokens_used = 200;
        assert!(mission.over_budget());

        mission.started_at = Some(Utc::now() - chrono::Duration::seconds(5));
        assert!(mission.timed_out());
    }
}
