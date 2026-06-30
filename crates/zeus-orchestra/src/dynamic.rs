//! Dynamic Orchestrator - Agent lifecycle management and capability routing
//!
//! Manages spin-up, teardown, idle reaping, and capability-based routing
//! with load balancing across a pool of agents.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::OrchestraError;
use crate::state::{AgentState, AgentStatus, GlobalStateManager};

// ---------------------------------------------------------------------------
// Task result types
// ---------------------------------------------------------------------------

/// Result submitted by a routed agent after completing its task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub agent_id: String,
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
}

/// Thread-safe map of agent_id → task result.
type TaskResultMap = Arc<RwLock<HashMap<String, TaskResult>>>;

// ---------------------------------------------------------------------------
// AgentFactory trait
// ---------------------------------------------------------------------------

/// Factory for creating and destroying agent instances.
#[async_trait]
pub trait AgentFactory: Send + Sync {
    /// Create a new agent with the given capability.
    async fn create_agent(&self, capability: &str) -> Result<AgentState, OrchestraError>;

    /// Destroy an agent by ID.
    async fn destroy_agent(&self, agent_id: &str) -> Result<(), OrchestraError>;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the dynamic orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConfig {
    /// Maximum number of agents allowed.
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    /// Seconds before an idle agent is eligible for reaping.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Whether to auto-scale (create new agents) when none are available.
    #[serde(default)]
    pub auto_scale: bool,
}

fn default_max_agents() -> usize {
    10
}
fn default_idle_timeout() -> u64 {
    300
}

impl Default for DynamicConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            idle_timeout_secs: default_idle_timeout(),
            auto_scale: false,
        }
    }
}

// ---------------------------------------------------------------------------
// DynamicOrchestrator
// ---------------------------------------------------------------------------

/// Manages agent lifecycle and capability-based routing.
pub struct DynamicOrchestrator {
    state_manager: Arc<GlobalStateManager>,
    factory: Arc<dyn AgentFactory>,
    config: DynamicConfig,
    /// Results submitted by routed agents (agent_id → result).
    task_results: TaskResultMap,
}

impl DynamicOrchestrator {
    pub fn new(
        state_manager: Arc<GlobalStateManager>,
        factory: Arc<dyn AgentFactory>,
        config: DynamicConfig,
    ) -> Self {
        Self {
            state_manager,
            factory,
            config,
            task_results: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Request an agent with a given capability.
    ///
    /// 1. Look for an idle agent with the capability.
    /// 2. If none found and `auto_scale` is enabled and under `max_agents`, create one.
    /// 3. Otherwise, return an error.
    pub async fn request_capability(&self, capability: &str) -> Result<String, OrchestraError> {
        // Try to find an existing idle agent
        if let Some(agent) = self.state_manager.best_for_task(capability).await {
            self.state_manager
                .update_status(
                    &agent.id,
                    AgentStatus::Busy(format!("assigned: {capability}")),
                )
                .await?;
            return Ok(agent.id);
        }

        // Auto-scale if enabled
        if self.config.auto_scale {
            let count = self.state_manager.agent_count().await;
            if count >= self.config.max_agents {
                return Err(OrchestraError::PolicyViolation(format!(
                    "max agents reached ({}/{})",
                    count, self.config.max_agents
                )));
            }
            let agent = self.factory.create_agent(capability).await?;
            let agent_id = agent.id.clone();
            self.state_manager.register_agent(agent).await?;
            self.state_manager
                .update_status(
                    &agent_id,
                    AgentStatus::Busy(format!("assigned: {capability}")),
                )
                .await?;
            return Ok(agent_id);
        }

        Err(OrchestraError::AgentNotFound(format!(
            "no idle agent with capability '{capability}'"
        )))
    }

    /// Release an agent back to idle status.
    pub async fn release_agent(&self, agent_id: &str) -> Result<(), OrchestraError> {
        self.state_manager
            .update_status(agent_id, AgentStatus::Idle)
            .await
    }

    /// Reap agents that have been idle longer than `idle_timeout_secs`.
    /// Returns IDs of destroyed agents.
    pub async fn reap_idle(&self) -> Result<Vec<String>, OrchestraError> {
        let timeout = std::time::Duration::from_secs(self.config.idle_timeout_secs);
        let stale = self.state_manager.stale_agents(timeout).await;
        let mut reaped = Vec::new();

        for agent in stale {
            // Only reap idle agents (don't kill busy ones that just missed a heartbeat)
            if agent.is_idle() {
                if let Err(e) = self.factory.destroy_agent(&agent.id).await {
                    tracing::warn!(agent_id = %agent.id, error = %e, "Failed to destroy agent");
                    continue;
                }
                let _ = self.state_manager.deregister_agent(&agent.id).await;
                reaped.push(agent.id);
            }
        }

        Ok(reaped)
    }

    /// Route a task to an agent that has ALL required capabilities.
    pub async fn route_task(
        &self,
        _task: &str,
        required_capabilities: &[String],
    ) -> Result<String, OrchestraError> {
        // Find an idle agent that has all required capabilities
        let agents = self.state_manager.list_idle().await;
        let matching = agents.iter().find(|a| {
            required_capabilities
                .iter()
                .all(|cap| a.has_capability(cap))
        });

        if let Some(agent) = matching {
            let agent_id = agent.id.clone();
            self.state_manager
                .update_status(
                    &agent_id,
                    AgentStatus::Busy(format!("routed: {} caps", required_capabilities.len())),
                )
                .await?;
            return Ok(agent_id);
        }

        // Try auto-scaling with the first required capability
        if self.config.auto_scale && !required_capabilities.is_empty() {
            let count = self.state_manager.agent_count().await;
            if count < self.config.max_agents {
                let agent = self.factory.create_agent(&required_capabilities[0]).await?;
                let agent_id = agent.id.clone();
                self.state_manager.register_agent(agent).await?;
                self.state_manager
                    .update_status(
                        &agent_id,
                        AgentStatus::Busy(format!("routed: {} caps", required_capabilities.len())),
                    )
                    .await?;
                return Ok(agent_id);
            }
        }

        Err(OrchestraError::AgentNotFound(format!(
            "no agent with capabilities: {:?}",
            required_capabilities
        )))
    }

    /// Re-route a task after a failure. Excludes the failed agent.
    pub async fn reassign_task(
        &self,
        failed_agent_id: &str,
        _task: &str,
        required_capabilities: &[String],
    ) -> Result<String, OrchestraError> {
        // Mark the failed agent as errored
        let _ = self
            .state_manager
            .update_status(
                failed_agent_id,
                AgentStatus::Error("task failed, reassigning".into()),
            )
            .await;

        // Find a different idle agent with the required capabilities
        let agents = self.state_manager.list_idle().await;
        let matching = agents.iter().find(|a| {
            a.id != failed_agent_id
                && required_capabilities
                    .iter()
                    .all(|cap| a.has_capability(cap))
        });

        if let Some(agent) = matching {
            let agent_id = agent.id.clone();
            self.state_manager
                .update_status(&agent_id, AgentStatus::Busy("reassigned".into()))
                .await?;
            return Ok(agent_id);
        }

        Err(OrchestraError::AgentNotFound(format!(
            "no alternate agent for capabilities: {:?}",
            required_capabilities
        )))
    }

    /// Submit a task result for a routed agent.
    ///
    /// Called by the execution layer when a routed agent finishes its task.
    /// Automatically releases the agent back to `Idle` status.
    pub async fn submit_result(&self, result: TaskResult) {
        let agent_id = result.agent_id.clone();
        self.task_results
            .write()
            .await
            .insert(agent_id.clone(), result);
        // Best-effort release — agent may have been reaped already
        let _ = self.release_agent(&agent_id).await;
    }

    /// Collect results for a set of agent IDs, removing them from the map.
    ///
    /// Returns results in the same order as `agent_ids`. Missing results
    /// (agent hasn't reported yet) are omitted.
    pub async fn collect_results(&self, agent_ids: &[String]) -> Vec<TaskResult> {
        let mut map = self.task_results.write().await;
        agent_ids.iter().filter_map(|id| map.remove(id)).collect()
    }

    /// Check how many of the given agents have submitted results.
    pub async fn results_ready(&self, agent_ids: &[String]) -> usize {
        let map = self.task_results.read().await;
        agent_ids.iter().filter(|id| map.contains_key(*id)).count()
    }

    /// Access the underlying state manager.
    pub fn state_manager(&self) -> &GlobalStateManager {
        &self.state_manager
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockFactory {
        counter: AtomicUsize,
    }

    impl MockFactory {
        fn new() -> Self {
            Self {
                counter: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentFactory for MockFactory {
        async fn create_agent(&self, capability: &str) -> Result<AgentState, OrchestraError> {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            let id = format!("mock-{n}");
            Ok(AgentState::new(&id, &id).with_capabilities(vec![capability.to_string()]))
        }

        async fn destroy_agent(&self, _agent_id: &str) -> Result<(), OrchestraError> {
            Ok(())
        }
    }

    fn setup() -> (Arc<GlobalStateManager>, Arc<MockFactory>) {
        (
            Arc::new(GlobalStateManager::new()),
            Arc::new(MockFactory::new()),
        )
    }

    #[tokio::test]
    async fn test_request_existing_agent() {
        let (sm, factory) = setup();
        let agent = AgentState::new("a1", "a1").with_capabilities(vec!["code".into()]);
        sm.register_agent(agent)
            .await
            .expect("async operation should succeed");

        let orch = DynamicOrchestrator::new(sm.clone(), factory, DynamicConfig::default());
        let id = orch
            .request_capability("code")
            .await
            .expect("async operation should succeed");
        assert_eq!(id, "a1");

        let a = sm
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(matches!(a.status, AgentStatus::Busy(_)));
    }

    #[tokio::test]
    async fn test_request_auto_scale() {
        let (sm, factory) = setup();
        let config = DynamicConfig {
            auto_scale: true,
            max_agents: 5,
            ..Default::default()
        };
        let orch = DynamicOrchestrator::new(sm.clone(), factory, config);

        let id = orch
            .request_capability("code")
            .await
            .expect("async operation should succeed");
        assert!(id.starts_with("mock-"));
        assert_eq!(sm.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_request_auto_scale_max_exceeded() {
        let (sm, factory) = setup();
        // Pre-fill to max
        sm.register_agent(AgentState::new("a1", "a1").with_capabilities(vec!["deploy".into()]))
            .await
            .expect("AgentState::new should succeed");
        sm.update_status("a1", AgentStatus::Busy("work".into()))
            .await
            .expect("async operation should succeed");

        let config = DynamicConfig {
            auto_scale: true,
            max_agents: 1,
            ..Default::default()
        };
        let orch = DynamicOrchestrator::new(sm, factory, config);
        let err = orch.request_capability("code").await.unwrap_err();
        assert!(matches!(err, OrchestraError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn test_request_no_auto_scale() {
        let (sm, factory) = setup();
        let config = DynamicConfig {
            auto_scale: false,
            ..Default::default()
        };
        let orch = DynamicOrchestrator::new(sm, factory, config);
        let err = orch.request_capability("code").await.unwrap_err();
        assert!(matches!(err, OrchestraError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn test_release_agent() {
        let (sm, factory) = setup();
        sm.register_agent(AgentState::new("a1", "a1").with_capabilities(vec!["code".into()]))
            .await
            .expect("AgentState::new should succeed");
        sm.update_status("a1", AgentStatus::Busy("working".into()))
            .await
            .expect("async operation should succeed");

        let orch = DynamicOrchestrator::new(sm.clone(), factory, DynamicConfig::default());
        orch.release_agent("a1")
            .await
            .expect("async operation should succeed");

        let a = sm
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(a.is_idle());
    }

    #[tokio::test]
    async fn test_reap_idle() {
        let (sm, factory) = setup();
        let mut old = AgentState::new("old", "old");
        old.last_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(600);
        sm.register_agent(old)
            .await
            .expect("async operation should succeed");
        sm.register_agent(AgentState::new("fresh", "fresh"))
            .await
            .expect("AgentState::new should succeed");

        let config = DynamicConfig {
            idle_timeout_secs: 300,
            ..Default::default()
        };
        let orch = DynamicOrchestrator::new(sm.clone(), factory, config);
        let reaped = orch
            .reap_idle()
            .await
            .expect("async operation should succeed");

        assert_eq!(reaped, vec!["old"]);
        assert_eq!(sm.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_route_single_capability() {
        let (sm, factory) = setup();
        sm.register_agent(AgentState::new("a1", "a1").with_capabilities(vec!["code".into()]))
            .await
            .expect("AgentState::new should succeed");

        let orch = DynamicOrchestrator::new(sm.clone(), factory, DynamicConfig::default());
        let id = orch
            .route_task("build feature", &["code".into()])
            .await
            .expect("async operation should succeed");
        assert_eq!(id, "a1");
    }

    #[tokio::test]
    async fn test_route_multi_capability() {
        let (sm, factory) = setup();
        sm.register_agent(
            AgentState::new("a1", "a1").with_capabilities(vec!["code".into(), "review".into()]),
        )
        .await
        .expect("async operation should succeed");
        sm.register_agent(AgentState::new("a2", "a2").with_capabilities(vec!["code".into()]))
            .await
            .expect("AgentState::new should succeed");

        let orch = DynamicOrchestrator::new(sm, factory, DynamicConfig::default());
        let id = orch
            .route_task("review PR", &["code".into(), "review".into()])
            .await
            .expect("async operation should succeed");
        assert_eq!(id, "a1");
    }

    #[tokio::test]
    async fn test_reassign_task() {
        let (sm, factory) = setup();
        sm.register_agent(AgentState::new("a1", "a1").with_capabilities(vec!["code".into()]))
            .await
            .expect("AgentState::new should succeed");
        sm.register_agent(AgentState::new("a2", "a2").with_capabilities(vec!["code".into()]))
            .await
            .expect("AgentState::new should succeed");
        // a1 was working and failed
        sm.update_status("a1", AgentStatus::Busy("task".into()))
            .await
            .expect("async operation should succeed");

        let orch = DynamicOrchestrator::new(sm.clone(), factory, DynamicConfig::default());
        let new_id = orch
            .reassign_task("a1", "retry task", &["code".into()])
            .await
            .expect("async operation should succeed");
        assert_eq!(new_id, "a2");

        // a1 should be in Error state
        let a1 = sm
            .get_agent("a1")
            .await
            .expect("async operation should succeed");
        assert!(matches!(a1.status, AgentStatus::Error(_)));
    }

    #[tokio::test]
    async fn test_config_defaults() {
        let config = DynamicConfig::default();
        assert_eq!(config.max_agents, 10);
        assert_eq!(config.idle_timeout_secs, 300);
        assert!(!config.auto_scale);
    }

    #[tokio::test]
    async fn test_config_serialization() {
        let config = DynamicConfig {
            max_agents: 20,
            idle_timeout_secs: 600,
            auto_scale: true,
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let de: DynamicConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.max_agents, 20);
        assert!(de.auto_scale);
    }
}
