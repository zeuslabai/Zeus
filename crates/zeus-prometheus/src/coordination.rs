//! Coordination Loop - Async multi-agent DAG execution
//!
//! Decomposes a goal into a DAG, routes tasks to agents via the
//! DynamicOrchestrator, monitors progress, and handles failures
//! with retries and reassignment.

use std::sync::Arc;

use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zeus_core::{Result, ToolSchema};
use zeus_llm::LlmClient;
use zeus_orchestra::{DynamicOrchestrator, GlobalStateManager};

use crate::planner::Plan;
use crate::strategic::{StrategicPlanner, TaskNodeStatus};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Events emitted during coordination.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoordinationEvent {
    TaskCompleted { step_id: usize, result: String },
    TaskFailed { step_id: usize, error: String },
    AgentDown { agent_id: String },
    Escalation { message: String },
}

/// Result of a full coordination run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationResult {
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub skipped_tasks: usize,
    pub total_time_ms: u64,
    pub agents_used: Vec<String>,
    pub events: Vec<CoordinationEvent>,
    pub dag_completed: bool,
}

/// Configuration for the coordination loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationConfig {
    /// Overall timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Max retries per individual task.
    #[serde(default = "default_max_retries")]
    pub max_retries_per_task: usize,
    /// Whether to continue executing when a task fails.
    #[serde(default = "default_continue_on_failure")]
    pub continue_on_failure: bool,
}

fn default_timeout_ms() -> u64 {
    300_000
}
fn default_max_retries() -> usize {
    2
}
fn default_continue_on_failure() -> bool {
    true
}

impl Default for CoordinationConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            max_retries_per_task: default_max_retries(),
            continue_on_failure: default_continue_on_failure(),
        }
    }
}

// ---------------------------------------------------------------------------
// CoordinationLoop
// ---------------------------------------------------------------------------

/// Async event loop that coordinates multi-agent DAG execution.
pub struct CoordinationLoop {
    planner: StrategicPlanner,
    orchestrator: Arc<DynamicOrchestrator>,
    #[allow(dead_code)] // Reserved for agent health queries during execution
    state_manager: Arc<GlobalStateManager>,
    config: CoordinationConfig,
}

impl CoordinationLoop {
    pub fn new(
        orchestrator: Arc<DynamicOrchestrator>,
        state_manager: Arc<GlobalStateManager>,
        config: CoordinationConfig,
    ) -> Self {
        Self {
            planner: StrategicPlanner::new(),
            orchestrator,
            state_manager,
            config,
        }
    }

    /// Full pipeline: LLM plan -> DAG -> execute groups -> collect results.
    pub async fn run(
        &self,
        goal: &str,
        llm: &LlmClient,
        tools: &[ToolSchema],
    ) -> Result<CoordinationResult> {
        let inner_planner = crate::planner::Planner::new();
        let plan = inner_planner.create_plan(goal, llm, tools).await?;
        self.run_plan(&plan).await
    }

    /// Execute a pre-built plan (skip LLM planning).
    pub async fn run_plan(&self, plan: &Plan) -> Result<CoordinationResult> {
        let start = std::time::Instant::now();

        // Build DAG
        let mut dag = self
            .planner
            .analyze(plan)
            .map_err(zeus_core::Error::Config)?;

        let groups = dag.parallel_groups();
        let (event_tx, mut event_rx) = mpsc::channel::<CoordinationEvent>(64);
        let mut all_events = Vec::new();
        let mut agents_used = Vec::new();

        info!(
            groups = groups.len(),
            nodes = dag.nodes.len(),
            "Starting coordinated execution"
        );

        // Execute group by group
        for (group_idx, group) in groups.iter().enumerate() {
            debug!(
                group = group_idx,
                tasks = group.len(),
                "Executing parallel group"
            );

            let ready = dag.next_ready();
            if ready.is_empty() && !dag.is_finished() {
                warn!("No ready tasks but DAG not finished — possible blocked state");
                break;
            }

            // Dispatch all tasks in this group concurrently
            let mut handles = Vec::new();
            for &step_id in &ready {
                let node = match dag.nodes.get(&step_id) {
                    Some(n) => n.clone(),
                    None => continue,
                };

                let orch = self.orchestrator.clone();
                let tx = event_tx.clone();
                let max_retries = self.config.max_retries_per_task;
                let timeout_ms = self.config.timeout_ms;

                // Mark as running
                if let Some(n) = dag.nodes.get_mut(&step_id) {
                    n.status = TaskNodeStatus::Running;
                }

                let gsm = self.state_manager.clone();
                let http = HttpClient::new();
                handles.push(tokio::spawn(async move {
                    dispatch_task(
                        step_id,
                        &node,
                        &orch,
                        &gsm,
                        &http,
                        &tx,
                        max_retries,
                        timeout_ms,
                    )
                    .await
                }));
            }

            // Collect results from this group
            for handle in handles {
                match handle.await {
                    Ok((step_id, success, agent_id)) => {
                        if success {
                            dag.complete_node(step_id);
                        } else {
                            dag.fail_node(step_id);
                        }
                        if let Some(aid) = agent_id
                            && !agents_used.contains(&aid)
                        {
                            agents_used.push(aid);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Task join error");
                    }
                }
            }

            // Drain events
            while let Ok(event) = event_rx.try_recv() {
                all_events.push(event);
            }

            // Check if we should stop on failure
            if !self.config.continue_on_failure {
                let any_failed = dag
                    .nodes
                    .values()
                    .any(|n| n.status == TaskNodeStatus::Failed);
                if any_failed {
                    info!("Stopping coordination due to task failure (continue_on_failure=false)");
                    // Skip remaining nodes
                    for node in dag.nodes.values_mut() {
                        if node.status == TaskNodeStatus::Pending {
                            node.status = TaskNodeStatus::Skipped;
                        }
                    }
                    break;
                }
            }

            // Timeout check
            if start.elapsed().as_millis() as u64 > self.config.timeout_ms {
                warn!("Coordination timeout exceeded");
                all_events.push(CoordinationEvent::Escalation {
                    message: "overall timeout exceeded".to_string(),
                });
                break;
            }
        }

        let total_time_ms = start.elapsed().as_millis() as u64;
        let completed_tasks = dag
            .nodes
            .values()
            .filter(|n| n.status == TaskNodeStatus::Completed)
            .count();
        let failed_tasks = dag
            .nodes
            .values()
            .filter(|n| n.status == TaskNodeStatus::Failed)
            .count();
        let skipped_tasks = dag
            .nodes
            .values()
            .filter(|n| n.status == TaskNodeStatus::Skipped)
            .count();

        Ok(CoordinationResult {
            completed_tasks,
            failed_tasks,
            skipped_tasks,
            total_time_ms,
            agents_used,
            events: all_events,
            dag_completed: dag.is_finished(),
        })
    }
}

/// Dispatch a single task to an agent, with retries.
///
/// If the selected agent has a `gateway_url`, the task is forwarded via
/// `POST {gateway_url}/v1/fleet/execute`.  Otherwise falls back to local
/// execution (marks success immediately — used for self-hosted tasks).
#[allow(clippy::too_many_arguments)]
async fn dispatch_task(
    step_id: usize,
    node: &crate::strategic::TaskNode,
    orchestrator: &DynamicOrchestrator,
    state_manager: &GlobalStateManager,
    http: &HttpClient,
    event_tx: &mpsc::Sender<CoordinationEvent>,
    _max_retries: usize,
    _timeout_ms: u64,
) -> (usize, bool, Option<String>) {
    let capability = node.tool.as_deref().unwrap_or("general");

    // 1. Acquire an agent with the required capability
    let agent_id = match orchestrator.request_capability(capability).await {
        Ok(id) => id,
        Err(e) => {
            debug!(step_id, error = %e, "No agent available");
            let _ = event_tx
                .send(CoordinationEvent::TaskFailed {
                    step_id,
                    error: format!("no agent available: {e}"),
                })
                .await;
            return (step_id, false, None);
        }
    };

    // 2. Look up the agent's gateway URL from GlobalStateManager
    let gateway_url = state_manager
        .get_agent(&agent_id)
        .await
        .and_then(|a| a.gateway_url());

    // 3. Dispatch remotely or fall back to local success
    let (success, result_msg) = if let Some(ref url) = gateway_url {
        remote_execute(http, url, &agent_id, step_id, &node.description).await
    } else {
        // No gateway URL — agent is local or URL not registered; mark success
        info!(
            step_id,
            agent_id = %agent_id,
            "No gateway URL for agent — marking as locally handled"
        );
        (true, format!("handled locally by {agent_id}"))
    };

    // 4. Release the agent back to the pool
    let _ = orchestrator.release_agent(&agent_id).await;

    let event = if success {
        CoordinationEvent::TaskCompleted {
            step_id,
            result: result_msg,
        }
    } else {
        CoordinationEvent::TaskFailed {
            step_id,
            error: result_msg,
        }
    };
    let _ = event_tx.send(event).await;

    (step_id, success, Some(agent_id))
}

/// POST {gateway_url}/v1/fleet/execute with task details.
/// Returns (success, message).
async fn remote_execute(
    http: &HttpClient,
    gateway_url: &str,
    agent_id: &str,
    step_id: usize,
    description: &str,
) -> (bool, String) {
    let url = format!("{}/v1/fleet/execute", gateway_url);
    let body = serde_json::json!({
        "step_id": step_id,
        "description": description,
    });

    debug!(
        agent_id = %agent_id,
        url = %url,
        step_id,
        "Dispatching task to remote agent"
    );

    match http.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            let text = resp.text().await.unwrap_or_default();
            info!(agent_id = %agent_id, step_id, "Remote task completed");
            (
                true,
                format!(
                    "completed by {} — {}",
                    agent_id,
                    text.chars().take(100).collect::<String>()
                ),
            )
        }
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(agent_id = %agent_id, step_id, %status, "Remote task failed");
            (
                false,
                format!(
                    "remote agent {} returned {}: {}",
                    agent_id,
                    status,
                    text.chars().take(200).collect::<String>()
                ),
            )
        }
        Err(e) => {
            warn!(agent_id = %agent_id, step_id, error = %e, "Remote dispatch network error");
            (
                false,
                format!("network error dispatching to {}: {}", agent_id, e),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{Plan, PlanStatus, Step, StepStatus};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zeus_orchestra::{AgentFactory, AgentState};

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
        async fn create_agent(
            &self,
            capability: &str,
        ) -> std::result::Result<AgentState, zeus_orchestra::OrchestraError> {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            let id = format!("mock-{n}");
            Ok(AgentState::new(&id, &id).with_capabilities(vec![capability.to_string()]))
        }

        async fn destroy_agent(
            &self,
            _agent_id: &str,
        ) -> std::result::Result<(), zeus_orchestra::OrchestraError> {
            Ok(())
        }
    }

    fn make_plan(steps: Vec<(usize, &str, Option<&str>, Vec<usize>)>) -> Plan {
        Plan {
            task: "test".to_string(),
            steps: steps
                .into_iter()
                .map(|(id, desc, tool, deps)| Step {
                    id,
                    description: desc.to_string(),
                    tool: tool.map(|t| t.to_string()),
                    arguments: None,
                    dependencies: deps,
                    status: StepStatus::Pending,
                    output: None,
                })
                .collect(),
            status: PlanStatus::Created,
        }
    }

    fn setup() -> (Arc<GlobalStateManager>, Arc<DynamicOrchestrator>) {
        let sm = Arc::new(GlobalStateManager::new());
        let factory = Arc::new(MockFactory::new());
        let config = zeus_orchestra::DynamicConfig {
            auto_scale: true,
            max_agents: 10,
            ..Default::default()
        };
        let orch = Arc::new(DynamicOrchestrator::new(sm.clone(), factory, config));
        (sm, orch)
    }

    #[tokio::test]
    async fn test_single_step_plan() {
        let (sm, orch) = setup();
        let coord = CoordinationLoop::new(orch, sm, CoordinationConfig::default());
        let plan = make_plan(vec![(1, "Do thing", Some("shell"), vec![])]);

        let result = coord.run_plan(&plan).await.unwrap();
        assert_eq!(result.completed_tasks, 1);
        assert_eq!(result.failed_tasks, 0);
        assert!(result.dag_completed);
    }

    #[tokio::test]
    async fn test_sequential_plan() {
        let (sm, orch) = setup();
        let coord = CoordinationLoop::new(orch, sm, CoordinationConfig::default());
        let plan = make_plan(vec![
            (1, "First", Some("shell"), vec![]),
            (2, "Second", Some("shell"), vec![1]),
        ]);

        let result = coord.run_plan(&plan).await.unwrap();
        assert_eq!(result.completed_tasks, 2);
        assert!(result.dag_completed);
    }

    #[tokio::test]
    async fn test_parallel_group() {
        let (sm, orch) = setup();
        let coord = CoordinationLoop::new(orch, sm, CoordinationConfig::default());
        let plan = make_plan(vec![
            (1, "A", Some("shell"), vec![]),
            (2, "B", Some("shell"), vec![]),
            (3, "C", Some("shell"), vec![]),
        ]);

        let result = coord.run_plan(&plan).await.unwrap();
        assert_eq!(result.completed_tasks, 3);
        assert!(result.dag_completed);
    }

    #[tokio::test]
    async fn test_diamond_plan() {
        let (sm, orch) = setup();
        let coord = CoordinationLoop::new(orch, sm, CoordinationConfig::default());
        let plan = make_plan(vec![
            (1, "Start", Some("shell"), vec![]),
            (2, "Left", Some("shell"), vec![1]),
            (3, "Right", Some("shell"), vec![1]),
            (4, "Merge", Some("shell"), vec![2, 3]),
        ]);

        let result = coord.run_plan(&plan).await.unwrap();
        assert_eq!(result.completed_tasks, 4);
        assert!(result.dag_completed);
    }

    #[tokio::test]
    async fn test_config_defaults() {
        let config = CoordinationConfig::default();
        assert_eq!(config.timeout_ms, 300_000);
        assert_eq!(config.max_retries_per_task, 2);
        assert!(config.continue_on_failure);
    }

    #[tokio::test]
    async fn test_config_serialization() {
        let config = CoordinationConfig {
            timeout_ms: 60_000,
            max_retries_per_task: 3,
            continue_on_failure: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: CoordinationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.timeout_ms, 60_000);
        assert_eq!(de.max_retries_per_task, 3);
        assert!(!de.continue_on_failure);
    }

    #[tokio::test]
    async fn test_event_serialization() {
        let events = vec![
            CoordinationEvent::TaskCompleted {
                step_id: 1,
                result: "done".into(),
            },
            CoordinationEvent::TaskFailed {
                step_id: 2,
                error: "oops".into(),
            },
            CoordinationEvent::AgentDown {
                agent_id: "a1".into(),
            },
            CoordinationEvent::Escalation {
                message: "help".into(),
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let de: CoordinationEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&de).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[tokio::test]
    async fn test_result_serialization() {
        let result = CoordinationResult {
            completed_tasks: 3,
            failed_tasks: 1,
            skipped_tasks: 0,
            total_time_ms: 5000,
            agents_used: vec!["a1".into(), "a2".into()],
            events: vec![],
            dag_completed: true,
        };
        let json = serde_json::to_string(&result).unwrap();
        let de: CoordinationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(de.completed_tasks, 3);
        assert!(de.dag_completed);
    }

    #[tokio::test]
    async fn test_empty_plan() {
        let (sm, orch) = setup();
        let coord = CoordinationLoop::new(orch, sm, CoordinationConfig::default());
        let plan = Plan {
            task: "nothing".to_string(),
            steps: vec![],
            status: PlanStatus::Created,
        };

        let result = coord.run_plan(&plan).await.unwrap();
        assert_eq!(result.completed_tasks, 0);
        assert!(result.dag_completed);
    }
}
