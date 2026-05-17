//! MCP Agent Manager — spawn, track, and query subagents via MCP tools
//!
//! Provides `list_agents`, `spawn_agent`, and `agent_status` tools that
//! delegate to `zeus_agent::spawn_subagent()` for real background execution.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tracing::info;
use zeus_agent::{SubagentConfig, SubagentResult, spawn_subagent};
use zeus_core::{Config, ToolSchema};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;

/// Runtime state of a tracked agent
#[derive(Debug, Clone)]
pub enum AgentState {
    Running,
    Completed { output: String, iterations: usize },
    Failed { error: String },
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Running => write!(f, "running"),
            AgentState::Completed { iterations, .. } => {
                write!(f, "completed ({} iterations)", iterations)
            }
            AgentState::Failed { error } => write!(f, "failed: {}", error),
        }
    }
}

/// A subagent tracked by the MCP server
pub struct TrackedAgent {
    pub id: String,
    pub task: String,
    pub state: AgentState,
    pub spawned_at: DateTime<Utc>,
    handle: Option<JoinHandle<SubagentResult>>,
}

/// Manages MCP-spawned subagents
pub struct McpAgentManager {
    agents: HashMap<String, TrackedAgent>,
    config: Config,
}

impl McpAgentManager {
    /// Create a new agent manager
    pub fn new(config: Config) -> Self {
        Self {
            agents: HashMap::new(),
            config,
        }
    }

    /// Spawn a new subagent in the background
    pub async fn spawn(
        &mut self,
        task: String,
        context: String,
        max_iterations: Option<usize>,
    ) -> Result<String, String> {
        let llm =
            LlmClient::from_config(&self.config).map_err(|e| format!("LLM init error: {}", e))?;
        let workspace = Workspace::from_config(&self.config);

        let subagent_config = SubagentConfig {
            max_iterations: max_iterations.unwrap_or(self.config.max_subagent_iterations),
            can_spawn: false, // MCP-spawned agents cannot spawn children
            task: task.clone(),
            context,
            model: Some(self.config.model.clone()),
            ..Default::default()
        };

        let handle = spawn_subagent(subagent_config, llm, workspace, None);

        let id = format!(
            "mcp-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("unknown")
        );

        let tracked = TrackedAgent {
            id: id.clone(),
            task,
            state: AgentState::Running,
            spawned_at: Utc::now(),
            handle: Some(handle),
        };

        info!("Spawned MCP agent {}", id);
        self.agents.insert(id.clone(), tracked);
        Ok(id)
    }

    /// Poll a JoinHandle and update state if finished
    pub async fn refresh_status(&mut self, id: &str) -> Option<AgentState> {
        let agent = self.agents.get_mut(id)?;

        if let Some(handle) = &agent.handle
            && handle.is_finished()
        {
            let handle = agent.handle.take().unwrap();
            match handle.await {
                Ok(result) => {
                    agent.state = if result.success {
                        AgentState::Completed {
                            output: result.output,
                            iterations: result.iterations,
                        }
                    } else {
                        AgentState::Failed {
                            error: result.output,
                        }
                    };
                }
                Err(e) => {
                    agent.state = AgentState::Failed {
                        error: format!("Task panicked: {}", e),
                    };
                }
            }
        }

        Some(agent.state.clone())
    }

    /// Refresh all agents and return JSON array
    pub async fn list(&mut self) -> Value {
        let ids: Vec<String> = self.agents.keys().cloned().collect();
        for id in &ids {
            self.refresh_status(id).await;
        }

        let entries: Vec<Value> = self
            .agents
            .values()
            .map(|a| {
                json!({
                    "id": a.id,
                    "task": a.task,
                    "state": a.state.to_string(),
                    "spawned_at": a.spawned_at.to_rfc3339(),
                })
            })
            .collect();

        json!(entries)
    }

    /// Refresh and return a single agent's status
    pub async fn status(&mut self, id: &str) -> Result<Value, String> {
        self.refresh_status(id).await;

        let agent = self
            .agents
            .get(id)
            .ok_or_else(|| format!("Agent '{}' not found", id))?;

        let mut obj = json!({
            "id": agent.id,
            "task": agent.task,
            "state": agent.state.to_string(),
            "spawned_at": agent.spawned_at.to_rfc3339(),
        });

        // Include output for completed/failed agents
        match &agent.state {
            AgentState::Completed { output, iterations } => {
                obj["output"] = json!(output);
                obj["iterations"] = json!(iterations);
            }
            AgentState::Failed { error } => {
                obj["error"] = json!(error);
            }
            AgentState::Running => {}
        }

        Ok(obj)
    }

    /// Return tool schemas for the 3 agent MCP tools
    pub fn agent_tool_schemas() -> Vec<ToolSchema> {
        vec![
            ToolSchema::new(
                "list_agents",
                "List all MCP-spawned agents and their status",
            ),
            ToolSchema::new(
                "spawn_agent",
                "Spawn a background subagent to execute a task",
            )
            .with_param("task", "string", "Task description for the agent", true)
            .with_param(
                "context",
                "string",
                "Additional context for the agent",
                false,
            )
            .with_param(
                "max_iterations",
                "integer",
                "Maximum iterations (default: config value)",
                false,
            ),
            ToolSchema::new(
                "agent_status",
                "Get the status and output of a spawned agent",
            )
            .with_param("id", "string", "Agent ID returned by spawn_agent", true),
        ]
    }

    /// Dispatch a tool call to the appropriate agent method
    pub async fn execute_tool(&mut self, name: &str, args: Value) -> Result<String, String> {
        match name {
            "list_agents" => {
                let result = self.list().await;
                Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
            }
            "spawn_agent" => {
                let task = args
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing required 'task' parameter")?
                    .to_string();
                let context = args
                    .get("context")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let max_iterations = args
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);

                let id = self.spawn(task, context, max_iterations).await?;
                Ok(json!({ "id": id, "status": "spawned" }).to_string())
            }
            "agent_status" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing required 'id' parameter")?;
                let result = self.status(id).await?;
                Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
            }
            _ => Err(format!("Unknown agent tool: {}", name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_state_display_running() {
        let state = AgentState::Running;
        assert_eq!(state.to_string(), "running");
    }

    #[test]
    fn test_agent_state_display_completed() {
        let state = AgentState::Completed {
            output: "done".to_string(),
            iterations: 5,
        };
        assert_eq!(state.to_string(), "completed (5 iterations)");
    }

    #[test]
    fn test_agent_state_display_failed() {
        let state = AgentState::Failed {
            error: "timeout".to_string(),
        };
        assert_eq!(state.to_string(), "failed: timeout");
    }

    #[test]
    fn test_agent_state_clone() {
        let state = AgentState::Completed {
            output: "result".to_string(),
            iterations: 3,
        };
        let cloned = state.clone();
        assert_eq!(cloned.to_string(), state.to_string());
    }

    #[test]
    fn test_agent_tool_schemas_count() {
        let schemas = McpAgentManager::agent_tool_schemas();
        assert_eq!(schemas.len(), 3);
    }

    #[test]
    fn test_agent_tool_schemas_names() {
        let schemas = McpAgentManager::agent_tool_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"list_agents"));
        assert!(names.contains(&"spawn_agent"));
        assert!(names.contains(&"agent_status"));
    }

    #[test]
    fn test_spawn_agent_schema_has_task_param() {
        let schemas = McpAgentManager::agent_tool_schemas();
        let spawn = schemas.iter().find(|s| s.name == "spawn_agent").unwrap();
        let props = spawn.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("task"));
        assert!(props.contains_key("context"));
        assert!(props.contains_key("max_iterations"));
    }

    #[test]
    fn test_agent_status_schema_has_id_param() {
        let schemas = McpAgentManager::agent_tool_schemas();
        let status = schemas.iter().find(|s| s.name == "agent_status").unwrap();
        let required = status.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("id")));
    }

    #[test]
    fn test_manager_new() {
        let config = Config::default();
        let mgr = McpAgentManager::new(config);
        assert!(mgr.agents.is_empty());
    }

    #[tokio::test]
    async fn test_manager_list_empty() {
        let config = Config::default();
        let mut mgr = McpAgentManager::new(config);
        let result = mgr.list().await;
        assert_eq!(result, json!([]));
    }

    #[tokio::test]
    async fn test_manager_status_not_found() {
        let config = Config::default();
        let mut mgr = McpAgentManager::new(config);
        let result = mgr.status("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn test_execute_unknown_tool() {
        let config = Config::default();
        let mut mgr = McpAgentManager::new(config);
        let result = mgr.execute_tool("unknown_tool", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown agent tool"));
    }
}
