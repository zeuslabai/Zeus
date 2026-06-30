//! Zeus Orchestra - Multi-agent collaboration and delegation
//!
//! Provides the infrastructure for multiple Zeus agents to work together:
//! - **MessageBus** - tokio broadcast-based inter-agent communication
//! - **AgentTeam** - named groups of agents with supervisor and policies
//! - **DelegationChain** - task delegation with context passing and result slots
//! - **TeamPolicy** - ACL, budgets, max depth, loop detection, timeouts
//! - **SmartRouter** - task complexity → model selection
//! - **ResultMerger** - combine multi-agent outputs (concat, vote, synthesize, pick_best)
//! - **WorkVerification** - automatic output review, scoring, re-delegation
//! - **Scheduler** - cron/scheduler system for recurring tasks

pub mod dynamic;
pub mod pantheon;
pub mod peer_review;
pub mod protocol;
pub mod recommend;
pub mod scheduler;
pub mod state;

pub use dynamic::{AgentFactory, DynamicConfig, DynamicOrchestrator, TaskResult};
pub use pantheon::{
    AgentRole, InterventionAction, Mission, MissionArtifact, MissionConstraints, MissionEvent,
    MissionState, MissionTask, PantheonOrchestrator, TaskState, TeamMember,
};
pub use peer_review::{
    ConsensusEngine, ConsensusResult, ConsensusStrategy, PeerReview, PeerReviewSystem, ReviewLog,
    ReviewLogEntry, ReviewPolicy, ReviewVerdict, WorkSubmission,
};
pub use protocol::{BroadcastScope, ProtocolHandler, ProtocolMessage, WorkStatus};
pub use state::{AgentState, AgentStatus, GlobalStateManager};

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum OrchestraError {
    #[error("policy violation: {0}")]
    PolicyViolation(String),

    #[error("budget exceeded")]
    BudgetExceeded,

    #[error("max delegation depth exceeded: {0}")]
    MaxDepthExceeded(u32),

    #[error("delegation loop detected: {0}")]
    LoopDetected(String),

    #[error("delegation failed: {0}")]
    DelegationFailed(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("team not found: {0}")]
    TeamNotFound(String),

    #[error("timeout")]
    Timeout,

    #[error("bus error: {0}")]
    BusError(String),

    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Type of inter-agent message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Direct,
    Broadcast,
    Delegation,
    DelegationResult,
    System,
}

/// An inter-agent message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from_agent: String,
    /// None = broadcast to all.
    pub to_agent: Option<String>,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub message_type: MessageType,
}

impl Message {
    pub fn direct(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from_agent: from.into(),
            to_agent: Some(to.into()),
            content: content.into(),
            timestamp: Utc::now(),
            message_type: MessageType::Direct,
        }
    }

    pub fn broadcast(from: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from_agent: from.into(),
            to_agent: None,
            content: content.into(),
            timestamp: Utc::now(),
            message_type: MessageType::Broadcast,
        }
    }

    pub fn delegation(
        from: impl Into<String>,
        to: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from_agent: from.into(),
            to_agent: Some(to.into()),
            content: task.into(),
            timestamp: Utc::now(),
            message_type: MessageType::Delegation,
        }
    }
}

// ---------------------------------------------------------------------------
// MessageBus
// ---------------------------------------------------------------------------

/// Broadcast-based message bus for inter-agent communication.
pub struct MessageBus {
    sender: broadcast::Sender<Message>,
    history: Arc<Mutex<Vec<Message>>>,
    max_history: usize,
}

impl MessageBus {
    /// Create a new message bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            history: Arc::new(Mutex::new(Vec::new())),
            max_history: 1000,
        }
    }

    /// Send a message on the bus.
    pub async fn send(&self, message: Message) -> Result<(), OrchestraError> {
        // Store in history
        let mut history = self.history.lock().await;
        history.push(message.clone());
        if history.len() > self.max_history {
            history.remove(0);
        }
        drop(history);

        // Ignore SendError when there are no active receivers — the message
        // is still persisted in history.
        let _ = self.sender.send(message);
        Ok(())
    }

    /// Subscribe to receive messages.
    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.sender.subscribe()
    }

    /// Get message history for a specific agent (sent or received).
    pub async fn messages_for(&self, agent_id: &str) -> Vec<Message> {
        let history = self.history.lock().await;
        history
            .iter()
            .filter(|m| {
                m.from_agent == agent_id
                    || m.to_agent.as_deref() == Some(agent_id)
                    || m.to_agent.is_none()
            })
            .cloned()
            .collect()
    }

    /// Get all message history.
    pub async fn all_messages(&self) -> Vec<Message> {
        self.history.lock().await.clone()
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Number of messages in history.
    pub async fn history_count(&self) -> usize {
        self.history.lock().await.len()
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new(256)
    }
}

// ---------------------------------------------------------------------------
// Delegation
// ---------------------------------------------------------------------------

/// Status of a delegation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    TimedOut,
    Verified,
    ReDelegated,
}

/// A single delegation in a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delegation {
    pub id: String,
    pub task: String,
    pub context: String,
    pub from_agent: String,
    pub to_agent: String,
    pub status: DelegationStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub depth: u32,
    pub parent_delegation_id: Option<String>,
}

impl Delegation {
    pub fn new(
        task: impl Into<String>,
        context: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            task: task.into(),
            context: context.into(),
            from_agent: from.into(),
            to_agent: to.into(),
            status: DelegationStatus::Pending,
            result: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            depth: 0,
            parent_delegation_id: None,
        }
    }

    /// Mark this delegation as completed with a result.
    pub fn complete(&mut self, result: String) {
        self.status = DelegationStatus::Completed;
        self.result = Some(result);
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds().max(0) as u64);
    }

    /// Mark this delegation as failed with an error.
    pub fn fail(&mut self, error: String) {
        self.status = DelegationStatus::Failed;
        self.error = Some(error);
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds().max(0) as u64);
    }
}

// ---------------------------------------------------------------------------
// Team policy
// ---------------------------------------------------------------------------

/// Policy governing a team's behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamPolicy {
    /// Maximum delegation depth (default: 3).
    pub max_depth: u32,
    /// Maximum total tokens budget (0 = unlimited).
    pub budget_tokens: u64,
    /// Timeout per delegation in seconds (default: 300).
    pub timeout_seconds: u64,
    /// Whether to enable loop detection.
    pub loop_detection: bool,
    /// Quality threshold for auto-verification (0.0-1.0).
    pub quality_threshold: f64,
    /// Whether work must be verified before completion.
    pub require_verification: bool,
}

impl Default for TeamPolicy {
    fn default() -> Self {
        Self {
            max_depth: 3,
            budget_tokens: 0,
            timeout_seconds: 300,
            loop_detection: true,
            quality_threshold: 0.8,
            require_verification: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Agent team
// ---------------------------------------------------------------------------

/// A team of agents that can collaborate on tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    pub id: String,
    pub name: String,
    pub agent_ids: Vec<String>,
    pub supervisor_id: Option<String>,
    pub policy: TeamPolicy,
    /// Peer review policy for work produced by this team's agents.
    #[serde(default)]
    pub review_policy: peer_review::ReviewPolicy,
    pub created_at: DateTime<Utc>,
}

impl AgentTeam {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            agent_ids: Vec::new(),
            supervisor_id: None,
            policy: TeamPolicy::default(),
            review_policy: peer_review::ReviewPolicy::default(),
            created_at: Utc::now(),
        }
    }

    pub fn with_agents(mut self, agents: Vec<String>) -> Self {
        self.agent_ids = agents;
        self
    }

    pub fn with_supervisor(mut self, supervisor: String) -> Self {
        self.supervisor_id = Some(supervisor);
        self
    }

    pub fn with_policy(mut self, policy: TeamPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_review_policy(mut self, review_policy: peer_review::ReviewPolicy) -> Self {
        self.review_policy = review_policy;
        self
    }

    pub fn has_agent(&self, agent_id: &str) -> bool {
        self.agent_ids.iter().any(|id| id == agent_id)
    }

    pub fn agent_count(&self) -> usize {
        self.agent_ids.len()
    }
}

// ---------------------------------------------------------------------------
// Work verification
// ---------------------------------------------------------------------------

/// Status of a verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationStatus {
    Pending,
    Pass,
    Fail,
}

/// A verification result for a delegation's output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub delegation_id: String,
    pub agent_id: String,
    pub status: VerificationStatus,
    pub score: f64,
    pub issues: Vec<String>,
    pub verified_by: String,
    pub timestamp: DateTime<Utc>,
}

impl Verification {
    pub fn new(
        delegation_id: impl Into<String>,
        agent_id: impl Into<String>,
        verified_by: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            delegation_id: delegation_id.into(),
            agent_id: agent_id.into(),
            status: VerificationStatus::Pending,
            score: 0.0,
            issues: Vec::new(),
            verified_by: verified_by.into(),
            timestamp: Utc::now(),
        }
    }

    pub fn pass(mut self, score: f64) -> Self {
        self.status = VerificationStatus::Pass;
        self.score = score;
        self
    }

    pub fn fail(mut self, score: f64, issues: Vec<String>) -> Self {
        self.status = VerificationStatus::Fail;
        self.score = score;
        self.issues = issues;
        self
    }
}

/// Policy for agent work verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPolicy {
    pub auto_verify: bool,
    pub verify_with: VerifyWith,
    pub quality_threshold: f64,
    pub require_tests: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyWith {
    Supervisor,
    Peer,
    Human,
}

impl Default for VerificationPolicy {
    fn default() -> Self {
        Self {
            auto_verify: true,
            verify_with: VerifyWith::Supervisor,
            quality_threshold: 0.8,
            require_tests: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Smart router
// ---------------------------------------------------------------------------

/// Task complexity level for routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskComplexity {
    Simple,
    Medium,
    Complex,
    Expert,
}

/// Maps task complexity to model selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartRouter {
    /// Mapping from complexity level to model string.
    pub routes: HashMap<String, String>,
    /// Fallback chain when primary model is unavailable.
    pub fallback_chain: Vec<String>,
}

impl SmartRouter {
    pub fn new() -> Self {
        let mut routes = HashMap::new();
        routes.insert("simple".to_string(), "ollama/llama3.2".to_string());
        routes.insert(
            "medium".to_string(),
            "anthropic/claude-sonnet-4-20250514".to_string(),
        );
        routes.insert(
            "complex".to_string(),
            "anthropic/claude-sonnet-4-20250514".to_string(),
        );
        routes.insert(
            "expert".to_string(),
            "anthropic/claude-sonnet-4-20250514".to_string(),
        );

        Self {
            routes,
            fallback_chain: vec![
                "anthropic/claude-sonnet-4-20250514".to_string(),
                "openai/gpt-4o".to_string(),
                "ollama/llama3.2".to_string(),
            ],
        }
    }

    /// Select a model for the given complexity.
    pub fn route(&self, complexity: TaskComplexity) -> &str {
        let key = match complexity {
            TaskComplexity::Simple => "simple",
            TaskComplexity::Medium => "medium",
            TaskComplexity::Complex => "complex",
            TaskComplexity::Expert => "expert",
        };
        self.routes.get(key).map(|s| s.as_str()).unwrap_or_else(|| {
            self.fallback_chain
                .first()
                .map(|s| s.as_str())
                .unwrap_or("ollama/llama3.2")
        })
    }

    /// Set a route for a complexity level.
    pub fn set_route(&mut self, complexity: &str, model: impl Into<String>) {
        self.routes.insert(complexity.to_string(), model.into());
    }
}

impl Default for SmartRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Result merger
// ---------------------------------------------------------------------------

/// Strategy for merging results from multiple agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Concatenate all results.
    Concat,
    /// Vote on the best result (majority wins).
    Vote,
    /// LLM synthesizes a combined answer.
    Synthesize,
    /// Pick the single best result (by score).
    PickBest,
}

/// Merges results from multiple delegations.
pub struct ResultMerger;

impl ResultMerger {
    /// Merge multiple delegation results using the given strategy.
    pub fn merge(results: &[&str], strategy: MergeStrategy) -> String {
        match strategy {
            MergeStrategy::Concat => results.join("\n\n---\n\n"),
            MergeStrategy::PickBest => {
                // Pick the longest result as a heuristic for "best"
                results
                    .iter()
                    .max_by_key(|r| r.len())
                    .copied()
                    .unwrap_or("")
                    .to_string()
            }
            MergeStrategy::Vote => {
                // Simple majority: pick the most common result
                let mut counts: HashMap<&str, usize> = HashMap::new();
                for r in results {
                    *counts.entry(r).or_insert(0) += 1;
                }
                counts
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(result, _)| result.to_string())
                    .unwrap_or_default()
            }
            MergeStrategy::Synthesize => {
                // For real synthesis, this would call an LLM.
                // Fallback: concat with synthesis header.
                format!(
                    "Synthesized from {} sources:\n\n{}",
                    results.len(),
                    results.join("\n\n")
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Orchestra (central coordinator)
// ---------------------------------------------------------------------------

/// Central coordinator managing teams, delegations, and the message bus.
pub struct Orchestra {
    bus: Arc<MessageBus>,
    teams: Arc<RwLock<HashMap<String, AgentTeam>>>,
    delegations: Arc<DashMap<String, Delegation>>,
    verifications: Arc<DashMap<String, Verification>>,
    router: Arc<RwLock<SmartRouter>>,
    verification_policies: Arc<RwLock<HashMap<String, VerificationPolicy>>>,
}

impl Orchestra {
    pub fn new() -> Self {
        Self {
            bus: Arc::new(MessageBus::new(256)),
            teams: Arc::new(RwLock::new(HashMap::new())),
            delegations: Arc::new(DashMap::new()),
            verifications: Arc::new(DashMap::new()),
            router: Arc::new(RwLock::new(SmartRouter::new())),
            verification_policies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Access the message bus.
    pub fn bus(&self) -> &MessageBus {
        &self.bus
    }

    // -- Team management ----------------------------------------------------

    pub async fn create_team(&self, team: AgentTeam) -> Result<AgentTeam, OrchestraError> {
        let mut teams = self.teams.write().await;
        if teams.contains_key(&team.id) {
            return Err(OrchestraError::PolicyViolation(format!(
                "team {} already exists",
                team.id
            )));
        }
        teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    pub async fn get_team(&self, id: &str) -> Result<AgentTeam, OrchestraError> {
        let teams = self.teams.read().await;
        teams
            .get(id)
            .cloned()
            .ok_or_else(|| OrchestraError::TeamNotFound(id.to_string()))
    }

    pub async fn list_teams(&self) -> Vec<AgentTeam> {
        let teams = self.teams.read().await;
        teams.values().cloned().collect()
    }

    pub async fn update_team(&self, team: AgentTeam) -> Result<(), OrchestraError> {
        let mut teams = self.teams.write().await;
        if !teams.contains_key(&team.id) {
            return Err(OrchestraError::TeamNotFound(team.id.clone()));
        }
        teams.insert(team.id.clone(), team);
        Ok(())
    }

    pub async fn delete_team(&self, id: &str) -> Result<(), OrchestraError> {
        let mut teams = self.teams.write().await;
        teams
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| OrchestraError::TeamNotFound(id.to_string()))
    }

    pub async fn team_count(&self) -> usize {
        self.teams.read().await.len()
    }

    // -- Delegation management ----------------------------------------------

    /// Create a new delegation within a team.
    pub async fn delegate(
        &self,
        team_id: &str,
        task: impl Into<String>,
        context: impl Into<String>,
        from_agent: impl Into<String>,
        target_agent: Option<String>,
    ) -> Result<Delegation, OrchestraError> {
        let team = self.get_team(team_id).await?;
        let from = from_agent.into();
        let task = task.into();
        let context = context.into();

        // Determine target agent
        let to = match target_agent {
            Some(id) => {
                if !team.has_agent(&id) {
                    return Err(OrchestraError::AgentNotFound(id));
                }
                id
            }
            None => {
                // Pick first agent that isn't the sender
                team.agent_ids
                    .iter()
                    .find(|id| id.as_str() != from)
                    .cloned()
                    .ok_or_else(|| {
                        OrchestraError::DelegationFailed("no available agents".to_string())
                    })?
            }
        };

        // Check depth
        let current_depth = self.current_delegation_depth(&from).await;
        if current_depth >= team.policy.max_depth {
            return Err(OrchestraError::MaxDepthExceeded(team.policy.max_depth));
        }

        // Loop detection
        if team.policy.loop_detection {
            let has_loop = self.delegations.iter().any(|r| {
                r.from_agent == to && r.to_agent == from && r.status == DelegationStatus::InProgress
            });
            if has_loop {
                return Err(OrchestraError::LoopDetected(format!("{} <-> {}", from, to)));
            }
        }

        let mut delegation = Delegation::new(&task, &context, &from, &to);
        delegation.depth = current_depth + 1;
        delegation.status = DelegationStatus::InProgress;

        // Send delegation message
        let msg = Message::delegation(&from, &to, &task);
        let _ = self.bus.send(msg).await;

        self.delegations
            .insert(delegation.id.clone(), delegation.clone());

        Ok(delegation)
    }

    /// Complete a delegation with a result.
    pub async fn complete_delegation(
        &self,
        delegation_id: &str,
        result: String,
    ) -> Result<(), OrchestraError> {
        let mut delegation = self.delegations.get_mut(delegation_id).ok_or_else(|| {
            OrchestraError::DelegationFailed(format!("delegation {delegation_id} not found"))
        })?;
        delegation.complete(result);
        Ok(())
    }

    /// Fail a delegation with an error.
    pub async fn fail_delegation(
        &self,
        delegation_id: &str,
        error: String,
    ) -> Result<(), OrchestraError> {
        let mut delegation = self.delegations.get_mut(delegation_id).ok_or_else(|| {
            OrchestraError::DelegationFailed(format!("delegation {delegation_id} not found"))
        })?;
        delegation.fail(error);
        Ok(())
    }

    /// Get delegations for a team.
    pub async fn team_delegations(&self, team_id: &str) -> Vec<Delegation> {
        let team = match self.get_team(team_id).await {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        self.delegations
            .iter()
            .filter(|r| team.has_agent(&r.from_agent) || team.has_agent(&r.to_agent))
            .map(|r| r.value().clone())
            .collect()
    }

    /// Get a specific delegation by ID.
    pub async fn get_delegation(&self, id: &str) -> Option<Delegation> {
        self.delegations.get(id).map(|r| r.value().clone())
    }

    /// List all delegations across all teams.
    pub async fn list_all_delegations(&self) -> Vec<Delegation> {
        self.delegations.iter().map(|r| r.value().clone()).collect()
    }

    /// Create a standalone delegation (not scoped to a team).
    ///
    /// Use this when delegating directly between agents without team policy
    /// enforcement. For team-scoped delegations with policy checks, use
    /// [`delegate()`](Self::delegate) instead.
    pub async fn create_delegation(
        &self,
        task: impl Into<String>,
        context: impl Into<String>,
        from_agent: impl Into<String>,
        to_agent: impl Into<String>,
    ) -> Delegation {
        let delegation = Delegation::new(task, context, from_agent, to_agent);

        // Send delegation message on the bus
        let msg = Message::delegation(
            &delegation.from_agent,
            &delegation.to_agent,
            &delegation.task,
        );
        let _ = self.bus.send(msg).await;

        self.delegations
            .insert(delegation.id.clone(), delegation.clone());
        delegation
    }

    /// Get total delegation count.
    pub async fn delegation_count(&self) -> usize {
        self.delegations.len()
    }

    async fn current_delegation_depth(&self, agent_id: &str) -> u32 {
        self.delegations
            .iter()
            .filter(|r| r.to_agent == agent_id && r.status == DelegationStatus::InProgress)
            .map(|r| r.depth)
            .max()
            .unwrap_or(0)
    }

    // -- Verification management --------------------------------------------

    pub async fn add_verification(&self, verification: Verification) {
        self.verifications
            .insert(verification.id.clone(), verification);
    }

    pub async fn list_verifications(&self) -> Vec<Verification> {
        self.verifications
            .iter()
            .map(|r| r.value().clone())
            .collect()
    }

    pub async fn get_verification(&self, id: &str) -> Option<Verification> {
        self.verifications.get(id).map(|r| r.value().clone())
    }

    pub async fn team_verifications(&self, team_id: &str) -> Vec<Verification> {
        let team = match self.get_team(team_id).await {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        self.verifications
            .iter()
            .filter(|r| team.has_agent(&r.agent_id))
            .map(|r| r.value().clone())
            .collect()
    }

    // -- Verification policy management -------------------------------------

    pub async fn set_verification_policy(&self, agent_id: &str, policy: VerificationPolicy) {
        self.verification_policies
            .write()
            .await
            .insert(agent_id.to_string(), policy);
    }

    pub async fn get_verification_policy(&self, agent_id: &str) -> Option<VerificationPolicy> {
        self.verification_policies
            .read()
            .await
            .get(agent_id)
            .cloned()
    }

    // -- Router access ------------------------------------------------------

    pub async fn router(&self) -> SmartRouter {
        self.router.read().await.clone()
    }

    pub async fn set_router(&self, router: SmartRouter) {
        *self.router.write().await = router;
    }
}

impl Default for Orchestra {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Message tests ------------------------------------------------------

    #[test]
    fn test_message_direct() {
        let msg = Message::direct("agent-a", "agent-b", "hello");
        assert_eq!(msg.from_agent, "agent-a");
        assert_eq!(msg.to_agent.as_deref(), Some("agent-b"));
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.message_type, MessageType::Direct);
    }

    #[test]
    fn test_message_broadcast() {
        let msg = Message::broadcast("agent-a", "all hands");
        assert!(msg.to_agent.is_none());
        assert_eq!(msg.message_type, MessageType::Broadcast);
    }

    #[test]
    fn test_message_delegation() {
        let msg = Message::delegation("boss", "worker", "do the thing");
        assert_eq!(msg.from_agent, "boss");
        assert_eq!(msg.to_agent.as_deref(), Some("worker"));
        assert_eq!(msg.message_type, MessageType::Delegation);
    }

    #[test]
    fn test_message_unique_ids() {
        let m1 = Message::direct("a", "b", "1");
        let m2 = Message::direct("a", "b", "2");
        assert_ne!(m1.id, m2.id);
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::direct("a", "b", "test");
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: Message = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.from_agent, "a");
        assert_eq!(de.content, "test");
    }

    // -- MessageBus tests ---------------------------------------------------

    #[tokio::test]
    async fn test_bus_creation() {
        let bus = MessageBus::new(32);
        assert_eq!(bus.history_count().await, 0);
    }

    #[tokio::test]
    async fn test_bus_default() {
        let bus = MessageBus::default();
        assert_eq!(bus.history_count().await, 0);
    }

    #[tokio::test]
    async fn test_bus_send_receive() {
        let bus = MessageBus::new(32);
        let mut rx = bus.subscribe();

        let msg = Message::direct("a", "b", "hello");
        bus.send(msg.clone())
            .await
            .expect("channel send should succeed");

        let received = rx.recv().await.expect("async operation should succeed");
        assert_eq!(received.content, "hello");
    }

    #[tokio::test]
    async fn test_bus_history() {
        let bus = MessageBus::new(32);
        bus.send(Message::direct("a", "b", "msg1"))
            .await
            .expect("channel send should succeed");
        bus.send(Message::direct("b", "a", "msg2"))
            .await
            .expect("channel send should succeed");

        assert_eq!(bus.history_count().await, 2);
        let all = bus.all_messages().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_bus_messages_for_agent() {
        let bus = MessageBus::new(32);
        bus.send(Message::direct("a", "b", "for b"))
            .await
            .expect("channel send should succeed");
        bus.send(Message::direct("c", "d", "not for b"))
            .await
            .expect("channel send should succeed");
        bus.send(Message::broadcast("a", "for all"))
            .await
            .expect("channel send should succeed");

        let msgs = bus.messages_for("b").await;
        assert_eq!(msgs.len(), 2); // direct + broadcast
    }

    #[tokio::test]
    async fn test_bus_subscriber_count() {
        let bus = MessageBus::new(32);
        assert_eq!(bus.subscriber_count(), 0);
        let _rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }

    // -- Delegation tests ---------------------------------------------------

    #[test]
    fn test_delegation_new() {
        let d = Delegation::new("build feature", "context here", "boss", "worker");
        assert_eq!(d.task, "build feature");
        assert_eq!(d.from_agent, "boss");
        assert_eq!(d.to_agent, "worker");
        assert_eq!(d.status, DelegationStatus::Pending);
        assert!(d.result.is_none());
    }

    #[test]
    fn test_delegation_complete() {
        let mut d = Delegation::new("task", "ctx", "a", "b");
        d.complete("done!".to_string());
        assert_eq!(d.status, DelegationStatus::Completed);
        assert_eq!(d.result.as_deref(), Some("done!"));
        assert!(d.completed_at.is_some());
    }

    #[test]
    fn test_delegation_fail() {
        let mut d = Delegation::new("task", "ctx", "a", "b");
        d.fail("oops".to_string());
        assert_eq!(d.status, DelegationStatus::Failed);
        assert_eq!(d.error.as_deref(), Some("oops"));
    }

    #[test]
    fn test_delegation_serialization() {
        let d = Delegation::new("task", "ctx", "a", "b");
        let json = serde_json::to_string(&d).expect("should serialize to JSON");
        let de: Delegation = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.task, "task");
        assert_eq!(de.status, DelegationStatus::Pending);
    }

    // -- AgentTeam tests ----------------------------------------------------

    #[test]
    fn test_team_new() {
        let team = AgentTeam::new("alpha-team");
        assert_eq!(team.name, "alpha-team");
        assert!(team.agent_ids.is_empty());
        assert!(team.supervisor_id.is_none());
        assert_eq!(team.policy.max_depth, 3);
    }

    #[test]
    fn test_team_builder() {
        let team = AgentTeam::new("builders")
            .with_agents(vec!["a".into(), "b".into()])
            .with_supervisor("s".into())
            .with_policy(TeamPolicy {
                max_depth: 5,
                ..Default::default()
            });
        assert_eq!(team.agent_count(), 2);
        assert_eq!(team.supervisor_id.as_deref(), Some("s"));
        assert_eq!(team.policy.max_depth, 5);
    }

    #[test]
    fn test_team_has_agent() {
        let team = AgentTeam::new("t").with_agents(vec!["a".into(), "b".into(), "c".into()]);
        assert!(team.has_agent("a"));
        assert!(team.has_agent("c"));
        assert!(!team.has_agent("d"));
    }

    #[test]
    fn test_team_serialization() {
        let team = AgentTeam::new("ser-test").with_agents(vec!["x".into()]);
        let json = serde_json::to_string(&team).expect("should serialize to JSON");
        let de: AgentTeam = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "ser-test");
        assert_eq!(de.agent_count(), 1);
    }

    // -- TeamPolicy tests ---------------------------------------------------

    #[test]
    fn test_policy_default() {
        let p = TeamPolicy::default();
        assert_eq!(p.max_depth, 3);
        assert_eq!(p.budget_tokens, 0);
        assert_eq!(p.timeout_seconds, 300);
        assert!(p.loop_detection);
        assert!((p.quality_threshold - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_policy_serialization() {
        let p = TeamPolicy {
            max_depth: 5,
            budget_tokens: 10000,
            timeout_seconds: 600,
            loop_detection: false,
            quality_threshold: 0.9,
            require_verification: true,
        };
        let json = serde_json::to_string(&p).expect("should serialize to JSON");
        let de: TeamPolicy = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.max_depth, 5);
        assert!(!de.loop_detection);
        assert!(de.require_verification);
    }

    // -- Verification tests -------------------------------------------------

    #[test]
    fn test_verification_new() {
        let v = Verification::new("del-1", "agent-1", "supervisor");
        assert_eq!(v.delegation_id, "del-1");
        assert_eq!(v.status, VerificationStatus::Pending);
        assert_eq!(v.score, 0.0);
    }

    #[test]
    fn test_verification_pass() {
        let v = Verification::new("del-1", "agent-1", "supervisor").pass(0.95);
        assert_eq!(v.status, VerificationStatus::Pass);
        assert!((v.score - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_verification_fail() {
        let v = Verification::new("del-1", "agent-1", "supervisor").fail(
            0.3,
            vec!["incomplete".to_string(), "wrong format".to_string()],
        );
        assert_eq!(v.status, VerificationStatus::Fail);
        assert_eq!(v.issues.len(), 2);
    }

    #[test]
    fn test_verification_serialization() {
        let v = Verification::new("d", "a", "s").pass(0.8);
        let json = serde_json::to_string(&v).expect("should serialize to JSON");
        let de: Verification = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.status, VerificationStatus::Pass);
    }

    // -- VerificationPolicy tests -------------------------------------------

    #[test]
    fn test_verification_policy_default() {
        let vp = VerificationPolicy::default();
        assert!(vp.auto_verify);
        assert_eq!(vp.verify_with, VerifyWith::Supervisor);
        assert!((vp.quality_threshold - 0.8).abs() < f64::EPSILON);
        assert!(!vp.require_tests);
    }

    // -- SmartRouter tests --------------------------------------------------

    #[test]
    fn test_router_default_routes() {
        let router = SmartRouter::new();
        assert!(router.route(TaskComplexity::Simple).contains("llama"));
        assert!(
            router.route(TaskComplexity::Complex).contains("claude")
                || router.route(TaskComplexity::Complex).contains("sonnet")
        );
    }

    #[test]
    fn test_router_set_route() {
        let mut router = SmartRouter::new();
        router.set_route("simple", "openai/gpt-4o-mini");
        assert_eq!(router.route(TaskComplexity::Simple), "openai/gpt-4o-mini");
    }

    #[test]
    fn test_router_serialization() {
        let router = SmartRouter::new();
        let json = serde_json::to_string(&router).expect("should serialize to JSON");
        let de: SmartRouter = serde_json::from_str(&json).expect("should parse successfully");
        assert!(!de.routes.is_empty());
        assert!(!de.fallback_chain.is_empty());
    }

    // -- ResultMerger tests -------------------------------------------------

    #[test]
    fn test_merge_concat() {
        let results = vec!["result A", "result B"];
        let merged = ResultMerger::merge(&results, MergeStrategy::Concat);
        assert!(merged.contains("result A"));
        assert!(merged.contains("result B"));
        assert!(merged.contains("---"));
    }

    #[test]
    fn test_merge_pick_best() {
        let results = vec!["short", "this is a much longer result"];
        let merged = ResultMerger::merge(&results, MergeStrategy::PickBest);
        assert_eq!(merged, "this is a much longer result");
    }

    #[test]
    fn test_merge_vote() {
        let results = vec!["answer A", "answer B", "answer A"];
        let merged = ResultMerger::merge(&results, MergeStrategy::Vote);
        assert_eq!(merged, "answer A");
    }

    #[test]
    fn test_merge_synthesize() {
        let results = vec!["part 1", "part 2"];
        let merged = ResultMerger::merge(&results, MergeStrategy::Synthesize);
        assert!(merged.contains("Synthesized from 2 sources"));
    }

    #[test]
    fn test_merge_empty() {
        let results: Vec<&str> = vec![];
        let merged = ResultMerger::merge(&results, MergeStrategy::Concat);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_strategy_serialization() {
        let json =
            serde_json::to_string(&MergeStrategy::PickBest).expect("should serialize to JSON");
        assert_eq!(json, "\"pick_best\"");
        let de: MergeStrategy =
            serde_json::from_str("\"concat\"").expect("should parse successfully");
        assert_eq!(de, MergeStrategy::Concat);
    }

    // -- Orchestra tests ----------------------------------------------------

    #[tokio::test]
    async fn test_orchestra_creation() {
        let orch = Orchestra::new();
        assert_eq!(orch.team_count().await, 0);
        assert_eq!(orch.delegation_count().await, 0);
    }

    #[tokio::test]
    async fn test_orchestra_default() {
        let orch = Orchestra::default();
        assert_eq!(orch.team_count().await, 0);
    }

    #[tokio::test]
    async fn test_create_team() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("test-team").with_agents(vec!["a".into(), "b".into()]);
        let created = orch
            .create_team(team)
            .await
            .expect("async operation should succeed");
        assert_eq!(created.name, "test-team");
        assert_eq!(orch.team_count().await, 1);
    }

    #[tokio::test]
    async fn test_get_team() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("get-test");
        let id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let retrieved = orch
            .get_team(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.name, "get-test");
    }

    #[tokio::test]
    async fn test_get_missing_team() {
        let orch = Orchestra::new();
        let err = orch.get_team("nope").await.unwrap_err();
        assert!(matches!(err, OrchestraError::TeamNotFound(_)));
    }

    #[tokio::test]
    async fn test_list_teams() {
        let orch = Orchestra::new();
        orch.create_team(AgentTeam::new("t1"))
            .await
            .expect("AgentTeam::new should succeed");
        orch.create_team(AgentTeam::new("t2"))
            .await
            .expect("AgentTeam::new should succeed");
        assert_eq!(orch.list_teams().await.len(), 2);
    }

    #[tokio::test]
    async fn test_update_team() {
        let orch = Orchestra::new();
        let mut team = AgentTeam::new("original");
        let id = team.id.clone();
        orch.create_team(team.clone())
            .await
            .expect("async operation should succeed");

        team.name = "updated".to_string();
        orch.update_team(team)
            .await
            .expect("async operation should succeed");

        let retrieved = orch
            .get_team(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.name, "updated");
    }

    #[tokio::test]
    async fn test_delete_team() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("doomed");
        let id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");
        orch.delete_team(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(orch.team_count().await, 0);
    }

    #[tokio::test]
    async fn test_delegate_task() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("workers").with_agents(vec!["boss".into(), "worker".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let delegation = orch
            .delegate(
                &team_id,
                "build feature",
                "context",
                "boss",
                Some("worker".into()),
            )
            .await
            .expect("async operation should succeed");
        assert_eq!(delegation.from_agent, "boss");
        assert_eq!(delegation.to_agent, "worker");
        assert_eq!(delegation.status, DelegationStatus::InProgress);
    }

    #[tokio::test]
    async fn test_delegate_auto_target() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("auto").with_agents(vec!["sender".into(), "receiver".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let delegation = orch
            .delegate(&team_id, "task", "ctx", "sender", None)
            .await
            .expect("async operation should succeed");
        assert_eq!(delegation.to_agent, "receiver");
    }

    #[tokio::test]
    async fn test_delegate_agent_not_found() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("t").with_agents(vec!["a".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let err = orch
            .delegate(&team_id, "task", "ctx", "a", Some("nonexistent".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, OrchestraError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn test_complete_delegation() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("t").with_agents(vec!["a".into(), "b".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let delegation = orch
            .delegate(&team_id, "task", "ctx", "a", Some("b".into()))
            .await
            .expect("async operation should succeed");

        orch.complete_delegation(&delegation.id, "done!".into())
            .await
            .expect("async operation should succeed");

        let d = orch
            .get_delegation(&delegation.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(d.status, DelegationStatus::Completed);
        assert_eq!(d.result.as_deref(), Some("done!"));
    }

    #[tokio::test]
    async fn test_fail_delegation() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("t").with_agents(vec!["a".into(), "b".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        let delegation = orch
            .delegate(&team_id, "task", "ctx", "a", Some("b".into()))
            .await
            .expect("async operation should succeed");

        orch.fail_delegation(&delegation.id, "broken".into())
            .await
            .expect("async operation should succeed");

        let d = orch
            .get_delegation(&delegation.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(d.status, DelegationStatus::Failed);
    }

    #[tokio::test]
    async fn test_team_delegations() {
        let orch = Orchestra::new();
        let team = AgentTeam::new("t").with_agents(vec!["a".into(), "b".into()]);
        let team_id = team.id.clone();
        orch.create_team(team)
            .await
            .expect("async operation should succeed");

        orch.delegate(&team_id, "t1", "c1", "a", Some("b".into()))
            .await
            .expect("async operation should succeed");
        orch.delegate(&team_id, "t2", "c2", "a", Some("b".into()))
            .await
            .expect("async operation should succeed");

        let dels = orch.team_delegations(&team_id).await;
        assert_eq!(dels.len(), 2);
    }

    #[tokio::test]
    async fn test_verification_flow() {
        let orch = Orchestra::new();
        let v = Verification::new("del-1", "agent-1", "supervisor").pass(0.9);
        orch.add_verification(v.clone()).await;

        let all = orch.list_verifications().await;
        assert_eq!(all.len(), 1);

        let got = orch
            .get_verification(&v.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(got.status, VerificationStatus::Pass);
    }

    #[tokio::test]
    async fn test_verification_policy_management() {
        let orch = Orchestra::new();
        let policy = VerificationPolicy {
            auto_verify: false,
            verify_with: VerifyWith::Peer,
            quality_threshold: 0.9,
            require_tests: true,
        };
        orch.set_verification_policy("agent-1", policy).await;

        let got = orch
            .get_verification_policy("agent-1")
            .await
            .expect("async operation should succeed");
        assert!(!got.auto_verify);
        assert_eq!(got.verify_with, VerifyWith::Peer);

        assert!(orch.get_verification_policy("missing").await.is_none());
    }

    #[tokio::test]
    async fn test_router_access() {
        let orch = Orchestra::new();
        let router = orch.router().await;
        assert!(!router.routes.is_empty());

        let mut new_router = router.clone();
        new_router.set_route("expert", "openai/o1-preview");
        orch.set_router(new_router).await;

        let updated = orch.router().await;
        assert_eq!(
            updated.routes.get("expert").map(|s| s.as_str()),
            Some("openai/o1-preview")
        );
    }

    // -- Error display tests ------------------------------------------------

    #[test]
    fn test_error_display() {
        assert_eq!(
            OrchestraError::PolicyViolation("x".into()).to_string(),
            "policy violation: x"
        );
        assert_eq!(
            OrchestraError::BudgetExceeded.to_string(),
            "budget exceeded"
        );
        assert_eq!(
            OrchestraError::MaxDepthExceeded(3).to_string(),
            "max delegation depth exceeded: 3"
        );
        assert_eq!(
            OrchestraError::LoopDetected("a <-> b".into()).to_string(),
            "delegation loop detected: a <-> b"
        );
        assert_eq!(OrchestraError::Timeout.to_string(), "timeout");
    }

    // -- Standalone delegation tests ----------------------------------------

    #[tokio::test]
    async fn test_create_standalone_delegation() {
        let orch = Orchestra::new();
        let d = orch
            .create_delegation("write docs", "ctx", "boss", "writer")
            .await;
        assert_eq!(d.task, "write docs");
        assert_eq!(d.from_agent, "boss");
        assert_eq!(d.to_agent, "writer");
        assert_eq!(d.context, "ctx");
        assert_eq!(d.status, DelegationStatus::Pending);
        assert_eq!(orch.delegation_count().await, 1);
    }

    #[tokio::test]
    async fn test_list_all_delegations() {
        let orch = Orchestra::new();
        assert!(orch.list_all_delegations().await.is_empty());

        orch.create_delegation("t1", "", "a", "b").await;
        orch.create_delegation("t2", "", "c", "d").await;

        let all = orch.list_all_delegations().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_standalone_delegation_complete_and_get() {
        let orch = Orchestra::new();
        let d = orch.create_delegation("task", "ctx", "a", "b").await;
        let id = d.id.clone();

        orch.complete_delegation(&id, "result!".into())
            .await
            .expect("async operation should succeed");

        let got = orch
            .get_delegation(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(got.status, DelegationStatus::Completed);
        assert_eq!(got.result.as_deref(), Some("result!"));
        assert!(got.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_standalone_delegation_recorded_in_bus() {
        let orch = Orchestra::new();
        let _d = orch.create_delegation("task", "ctx", "a", "b").await;

        let messages = orch.bus().all_messages().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, MessageType::Delegation);
        assert_eq!(messages[0].from_agent, "a");
        assert_eq!(messages[0].to_agent.as_deref(), Some("b"));
    }
}
