//! Global State Manager - Centralized agent state tracking
//!
//! Single source of truth for all agent states: status, capabilities,
//! health, load, and heartbeat tracking.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Runtime status of an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Busy(String),
    Error(String),
    Offline,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Busy(task) => write!(f, "busy: {task}"),
            Self::Error(msg) => write!(f, "error: {msg}"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

/// Full state of a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: AgentStatus,
    /// Health score 0.0 (dead) .. 1.0 (perfect).
    pub health_score: f32,
    /// Description of current task (if busy).
    pub current_task: Option<String>,
    /// CPU/memory load percentage 0.0 .. 1.0.
    pub load_pct: f32,
    pub last_heartbeat: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl AgentState {
    /// Create a new agent state with sensible defaults.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name: name.into(),
            capabilities: Vec::new(),
            status: AgentStatus::Idle,
            health_score: 1.0,
            current_task: None,
            load_pct: 0.0,
            last_heartbeat: now,
            registered_at: now,
            metadata: HashMap::new(),
        }
    }

    pub fn with_capabilities(mut self, caps: Vec<String>) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.status, AgentStatus::Idle)
    }

    pub fn is_available(&self) -> bool {
        self.is_idle() && self.health_score > 0.0
    }

    /// Resolve the HTTP gateway URL for this agent.
    ///
    /// Priority:
    /// 1. `metadata["gateway_url"]` — explicit override
    /// 2. `http://{metadata["ip"]}:8080` — derived from IP
    /// 3. `None` — no network address registered
    pub fn gateway_url(&self) -> Option<String> {
        if let Some(url) = self.metadata.get("gateway_url") {
            return Some(url.clone());
        }
        self.metadata
            .get("ip")
            .map(|ip| format!("http://{}:8080", ip))
    }
}

// ---------------------------------------------------------------------------
// GlobalStateManager
// ---------------------------------------------------------------------------

/// Centralized in-memory registry of all agent states.
pub struct GlobalStateManager {
    agents: Arc<RwLock<HashMap<String, AgentState>>>,
}

impl GlobalStateManager {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // -- Registration -------------------------------------------------------

    /// Register a new agent. Returns error if ID already registered.
    pub async fn register_agent(&self, state: AgentState) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        if agents.contains_key(&state.id) {
            return Err(crate::OrchestraError::PolicyViolation(format!(
                "agent {} already registered",
                state.id
            )));
        }
        agents.insert(state.id.clone(), state);
        Ok(())
    }

    /// Remove an agent from the registry.
    pub async fn deregister_agent(&self, agent_id: &str) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        agents
            .remove(agent_id)
            .map(|_| ())
            .ok_or_else(|| crate::OrchestraError::AgentNotFound(agent_id.to_string()))
    }

    // -- State updates ------------------------------------------------------

    pub async fn update_status(
        &self,
        agent_id: &str,
        status: AgentStatus,
    ) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| crate::OrchestraError::AgentNotFound(agent_id.to_string()))?;
        // Sync current_task with status
        agent.current_task = match &status {
            AgentStatus::Busy(task) => Some(task.clone()),
            _ => None,
        };
        agent.status = status;
        Ok(())
    }

    pub async fn update_health(
        &self,
        agent_id: &str,
        health: f32,
    ) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| crate::OrchestraError::AgentNotFound(agent_id.to_string()))?;
        agent.health_score = health.clamp(0.0, 1.0);
        Ok(())
    }

    pub async fn update_load(
        &self,
        agent_id: &str,
        load: f32,
    ) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| crate::OrchestraError::AgentNotFound(agent_id.to_string()))?;
        agent.load_pct = load.clamp(0.0, 1.0);
        Ok(())
    }

    /// Record a heartbeat for an agent (updates last_heartbeat timestamp).
    pub async fn heartbeat(&self, agent_id: &str) -> Result<(), crate::OrchestraError> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| crate::OrchestraError::AgentNotFound(agent_id.to_string()))?;
        agent.last_heartbeat = Utc::now();
        Ok(())
    }

    // -- Queries ------------------------------------------------------------

    pub async fn get_agent(&self, agent_id: &str) -> Option<AgentState> {
        self.agents.read().await.get(agent_id).cloned()
    }

    pub async fn list_agents(&self) -> Vec<AgentState> {
        self.agents.read().await.values().cloned().collect()
    }

    pub async fn list_idle(&self) -> Vec<AgentState> {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.is_idle())
            .cloned()
            .collect()
    }

    pub async fn list_by_capability(&self, capability: &str) -> Vec<AgentState> {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.has_capability(capability))
            .cloned()
            .collect()
    }

    pub async fn agent_count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Find the best idle agent for a given capability requirement.
    /// Prefers: idle + has capability + highest health + lowest load.
    pub async fn best_for_task(&self, capability: &str) -> Option<AgentState> {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.is_available() && a.has_capability(capability))
            .max_by(|a, b| {
                // Higher health is better, lower load is better
                let score_a = a.health_score - a.load_pct;
                let score_b = b.health_score - b.load_pct;
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }

    /// Find agents whose last heartbeat is older than `max_age`.
    pub async fn stale_agents(&self, max_age: Duration) -> Vec<AgentState> {
        let cutoff = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default();
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.last_heartbeat < cutoff)
            .cloned()
            .collect()
    }
}

impl Default for GlobalStateManager {
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

    fn make_agent(id: &str, caps: Vec<&str>) -> AgentState {
        AgentState::new(id, id).with_capabilities(caps.into_iter().map(String::from).collect())
    }

    #[tokio::test]
    async fn test_register_and_get() {
        let mgr = GlobalStateManager::new();
        let agent = make_agent("a1", vec!["code"]);
        mgr.register_agent(agent)
            .await
            .expect("async operation should succeed");

        let got = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert_eq!(got.name, "a1");
        assert!(got.has_capability("code"));
    }

    #[tokio::test]
    async fn test_register_duplicate_fails() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        let err = mgr
            .register_agent(make_agent("a1", vec![]))
            .await
            .unwrap_err();
        assert!(matches!(err, crate::OrchestraError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn test_deregister() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.deregister_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(mgr.get_agent("a1").await.is_none());
        assert_eq!(mgr.agent_count().await, 0);
    }

    #[tokio::test]
    async fn test_deregister_missing() {
        let mgr = GlobalStateManager::new();
        let err = mgr.deregister_agent("nope").await.unwrap_err();
        assert!(matches!(err, crate::OrchestraError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn test_update_status() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");

        mgr.update_status("a1", AgentStatus::Busy("building".into()))
            .await
            .expect("async operation should succeed");
        let a = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(matches!(a.status, AgentStatus::Busy(_)));
        assert_eq!(a.current_task.as_deref(), Some("building"));

        mgr.update_status("a1", AgentStatus::Idle)
            .await
            .expect("async operation should succeed");
        let a = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(a.is_idle());
        assert!(a.current_task.is_none());
    }

    #[tokio::test]
    async fn test_update_health() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.update_health("a1", 0.5)
            .await
            .expect("async operation should succeed");
        let a = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!((a.health_score - 0.5).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_update_health_clamped() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.update_health("a1", 2.0)
            .await
            .expect("async operation should succeed");
        let a = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!((a.health_score - 1.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_update_load() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.update_load("a1", 0.75)
            .await
            .expect("async operation should succeed");
        let a = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!((a.load_pct - 0.75).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_heartbeat() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        let before = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed")
            .last_heartbeat;
        tokio::time::sleep(Duration::from_millis(10)).await;
        mgr.heartbeat("a1")
            .await
            .expect("async operation should succeed");
        let after = mgr
            .get_agent("a1")
            .await
            .expect("async operation should succeed")
            .last_heartbeat;
        assert!(after > before);
    }

    #[tokio::test]
    async fn test_list_agents() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.register_agent(make_agent("a2", vec![]))
            .await
            .expect("async operation should succeed");
        assert_eq!(mgr.list_agents().await.len(), 2);
    }

    #[tokio::test]
    async fn test_list_idle() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.register_agent(make_agent("a2", vec![]))
            .await
            .expect("async operation should succeed");
        mgr.update_status("a1", AgentStatus::Busy("work".into()))
            .await
            .expect("async operation should succeed");

        let idle = mgr.list_idle().await;
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].id, "a2");
    }

    #[tokio::test]
    async fn test_list_by_capability() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec!["code", "review"]))
            .await
            .expect("async operation should succeed");
        mgr.register_agent(make_agent("a2", vec!["code"]))
            .await
            .expect("async operation should succeed");
        mgr.register_agent(make_agent("a3", vec!["deploy"]))
            .await
            .expect("async operation should succeed");

        let coders = mgr.list_by_capability("code").await;
        assert_eq!(coders.len(), 2);
        let deployers = mgr.list_by_capability("deploy").await;
        assert_eq!(deployers.len(), 1);
    }

    #[tokio::test]
    async fn test_best_for_task() {
        let mgr = GlobalStateManager::new();
        // a1: idle, health 0.8, load 0.2 => score 0.6
        let mut a1 = make_agent("a1", vec!["code"]);
        a1.health_score = 0.8;
        a1.load_pct = 0.2;
        mgr.register_agent(a1)
            .await
            .expect("async operation should succeed");

        // a2: idle, health 1.0, load 0.1 => score 0.9 (best)
        let mut a2 = make_agent("a2", vec!["code"]);
        a2.health_score = 1.0;
        a2.load_pct = 0.1;
        mgr.register_agent(a2)
            .await
            .expect("async operation should succeed");

        // a3: idle but no "code" capability
        mgr.register_agent(make_agent("a3", vec!["deploy"]))
            .await
            .expect("async operation should succeed");

        let best = mgr
            .best_for_task("code")
            .await
            .expect("async operation should succeed");
        assert_eq!(best.id, "a2");
    }

    #[tokio::test]
    async fn test_best_for_task_no_match() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec!["code"]))
            .await
            .expect("async operation should succeed");
        assert!(mgr.best_for_task("deploy").await.is_none());
    }

    #[tokio::test]
    async fn test_best_for_task_busy_excluded() {
        let mgr = GlobalStateManager::new();
        mgr.register_agent(make_agent("a1", vec!["code"]))
            .await
            .expect("async operation should succeed");
        mgr.update_status("a1", AgentStatus::Busy("working".into()))
            .await
            .expect("async operation should succeed");
        assert!(mgr.best_for_task("code").await.is_none());
    }

    #[tokio::test]
    async fn test_stale_agents() {
        let mgr = GlobalStateManager::new();
        let mut old_agent = make_agent("stale", vec![]);
        old_agent.last_heartbeat = Utc::now() - chrono::Duration::seconds(600);
        mgr.register_agent(old_agent)
            .await
            .expect("async operation should succeed");
        mgr.register_agent(make_agent("fresh", vec![]))
            .await
            .expect("async operation should succeed");

        let stale = mgr.stale_agents(Duration::from_secs(300)).await;
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, "stale");
    }

    #[tokio::test]
    async fn test_agent_status_display() {
        assert_eq!(AgentStatus::Idle.to_string(), "idle");
        assert_eq!(AgentStatus::Busy("task".into()).to_string(), "busy: task");
        assert_eq!(AgentStatus::Error("oops".into()).to_string(), "error: oops");
        assert_eq!(AgentStatus::Offline.to_string(), "offline");
    }

    #[tokio::test]
    async fn test_agent_state_serialization() {
        let agent = make_agent("a1", vec!["code", "review"]);
        let json = serde_json::to_string(&agent).expect("should serialize to JSON");
        let de: AgentState = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "a1");
        assert_eq!(de.capabilities.len(), 2);
        assert!(matches!(de.status, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_agent_status_serialization() {
        let statuses = vec![
            AgentStatus::Idle,
            AgentStatus::Busy("coding".into()),
            AgentStatus::Error("timeout".into()),
            AgentStatus::Offline,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).expect("should serialize to JSON");
            let de: AgentStatus = serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(de, status);
        }
    }
}
