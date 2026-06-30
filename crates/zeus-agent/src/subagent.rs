//! Subagent - Background workers with constraints
//!
//! Supports local subagents and remote agents via `AgentTarget::Remote`
//! which POSTs tasks to a remote Zeus gateway's `/v1/agents/run-task` endpoint.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;
use zeus_aegis::Aegis;
use zeus_core::{Message, Result};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;

use crate::AgentEvent;
use crate::tools::ToolRegistry;

// ============================================================================
// Agent Target — local vs remote
// ============================================================================

/// Where to execute the agent task.
#[derive(Debug, Clone, Default)]
pub enum AgentTarget {
    /// Run locally as a tokio task
    #[default]
    Local,
    /// POST to a remote Zeus gateway
    Remote {
        /// Base URL of the remote gateway (e.g., "http://192.168.1.100:8080")
        gateway_url: String,
        /// Optional auth token for the remote gateway
        auth_token: Option<String>,
    },
}

// ============================================================================
// Subagent Config & Result
// ============================================================================

#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Maximum iterations for this subagent
    pub max_iterations: usize,
    /// Whether this subagent can spawn other subagents
    pub can_spawn: bool,
    /// Task description
    pub task: String,
    /// Additional context
    pub context: String,
    /// Where to execute: local or remote gateway
    pub target: AgentTarget,
    /// LLM model string to use (e.g. "anthropic/claude-sonnet-4-20250514").
    /// When set, remote gateways will honour this instead of their own config.
    pub model: Option<String>,
    /// GAP #4b: Reasoning-effort hint from the selected persona's frontmatter
    /// (e.g. "low"/"medium"/"high"). `None` when the persona is unset or omits it.
    pub effort: Option<String>,
    /// GAP #4b: Tool allow-list from the selected persona's frontmatter.
    /// Empty when the persona is unset or omits `tools:` — no restriction applied.
    pub tools: Vec<String>,
    /// Optional mission ID for result aggregation.
    /// When set, the subagent's final output will be tagged with this ID
    /// so the parent/coordinator can collect results from all spawned agents.
    pub mission_id: Option<String>,
    /// Parent's system prompt prefix for prompt cache sharing.
    /// When set, the subagent uses this as its system prompt base,
    /// enabling API-level prompt caching (same prefix = cache hit).
    pub parent_system_prompt: Option<String>,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 15,
            can_spawn: false,
            task: String::new(),
            context: String::new(),
            target: AgentTarget::Local,
            model: None,
            effort: None,
            tools: Vec::new(),
            mission_id: None,
            parent_system_prompt: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubagentResult {
    pub id: String,
    pub success: bool,
    pub output: String,
    pub iterations: usize,
    /// Mission ID this result belongs to (if spawned as part of a Pantheon mission).
    pub mission_id: Option<String>,
}

pub struct Subagent {
    id: String,
    config: SubagentConfig,
    llm: LlmClient,
    tools: ToolRegistry,
    workspace: Workspace,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    aegis: Option<Arc<Aegis>>,
}

impl Subagent {
    /// Create a new subagent
    pub fn new(
        subagent_config: SubagentConfig,
        llm: LlmClient,
        workspace: Workspace,
        aegis: Option<Arc<Aegis>>,
    ) -> Self {
        let id = format!(
            "sub-{}",
            Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("unknown")
        );

        // Use the standard tool registry (can_spawn is handled at execute time)
        let tools = ToolRegistry::new();

        Self {
            id,
            config: subagent_config,
            llm,
            tools,
            workspace,
            event_tx: None,
            aegis,
        }
    }

    /// Get the subagent's ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Set the event channel for streaming updates
    pub fn with_events(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Run the subagent
    pub async fn run(self) -> SubagentResult {
        info!("Subagent {} starting: {}", self.id, self.config.task);

        let mission_id = self.config.mission_id.clone();

        let system_prompt = match self.build_system_prompt().await {
            Ok(p) => p,
            Err(e) => {
                return SubagentResult {
                    id: self.id,
                    success: false,
                    output: format!("Failed to build system prompt: {}", e),
                    iterations: 0,
                    mission_id,
                };
            }
        };

        let tool_schemas = self.tools.schemas();
        let mut messages = vec![Message::user(&self.config.task)];
        let mut iterations = 0;
        let mut last_output = String::new();

        loop {
            iterations += 1;
            if iterations > self.config.max_iterations {
                warn!(
                    "Subagent {} max iterations ({}) reached",
                    self.id, self.config.max_iterations
                );
                break;
            }

            debug!(
                "Subagent {} iteration {}/{}",
                self.id, iterations, self.config.max_iterations
            );

            // Get LLM response
            let response = match self
                .llm
                .complete(&messages, &tool_schemas, Some(&system_prompt))
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return SubagentResult {
                        id: self.id,
                        success: false,
                        output: format!("LLM error: {}", e),
                        iterations,
                        mission_id,
                    };
                }
            };

            last_output = response.content.clone();

            // Add assistant message
            messages.push(
                Message::assistant(&response.content).with_tool_calls(response.tool_calls.clone()),
            );

            // Check if done
            match response.stop_reason {
                zeus_llm::StopReason::EndTurn => {
                    info!(
                        "Subagent {} finished after {} iterations",
                        self.id, iterations
                    );
                    return SubagentResult {
                        id: self.id,
                        success: true,
                        output: last_output,
                        iterations,
                        mission_id,
                    };
                }
                zeus_llm::StopReason::ToolUse => {
                    // Execute tools
                    let mut tool_results = Vec::new();
                    for call in &response.tool_calls {
                        // Block spawn if not allowed
                        if call.name == "spawn" && !self.config.can_spawn {
                            tool_results.push(zeus_core::ToolResult {
                                call_id: call.id.clone(),
                                success: false,
                                output: "Subagents cannot spawn other subagents".to_string(),
                            });
                            continue;
                        }

                        // Aegis security checks (mirrors parent agent checks)
                        if let Some(ref aegis) = self.aegis {
                            if !aegis.is_permitted(&call.name) {
                                warn!(
                                    "Subagent {}: tool '{}' blocked by Aegis",
                                    self.id, call.name
                                );
                                tool_results.push(zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: false,
                                    output: format!(
                                        "Security: tool '{}' is not permitted",
                                        call.name
                                    ),
                                });
                                continue;
                            }

                            if call.name == "shell"
                                && let Some(cmd) =
                                    call.arguments.get("command").and_then(|v| v.as_str())
                                && let Err(e) = aegis.validate_shell_command(cmd)
                            {
                                warn!(
                                    "Subagent {}: shell command blocked by Aegis: {}",
                                    self.id, e
                                );
                                tool_results.push(zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: false,
                                    output: format!("Security: {}", e),
                                });
                                continue;
                            }

                            if call.name == "web_fetch"
                                && let Some(url) =
                                    call.arguments.get("url").and_then(|v| v.as_str())
                                && let Err(e) = aegis.check_network_url(url)
                            {
                                warn!("Subagent {}: web_fetch blocked by Aegis: {}", self.id, e);
                                tool_results.push(zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: false,
                                    output: format!("Security: {}", e),
                                });
                                continue;
                            }

                            if matches!(
                                call.name.as_str(),
                                "read_file" | "write_file" | "edit_file"
                            ) && let Some(path) =
                                call.arguments.get("path").and_then(|v| v.as_str())
                                && aegis.restricts_filesystem()
                                && !aegis.is_path_allowed(path)
                            {
                                warn!(
                                    "Subagent {}: file access to '{}' blocked by Aegis",
                                    self.id, path
                                );
                                tool_results.push(zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: false,
                                    output: format!("Security: file access denied for '{}'", path),
                                });
                                continue;
                            }
                        }

                        let result =
                            match self.tools.execute(&call.name, call.arguments.clone()).await {
                                Ok(output) => zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: true,
                                    output,
                                },
                                Err(e) => zeus_core::ToolResult {
                                    call_id: call.id.clone(),
                                    success: false,
                                    output: e.to_string(),
                                },
                            };
                        tool_results.push(result);
                    }

                    // Add tool results
                    messages.push(Message {
                        role: zeus_core::Role::Tool,
                        content: String::new(),
                        tool_calls: vec![],
                        tool_results,
                        timestamp: chrono::Utc::now(),
                        attachments: vec![],
                        message_id: Some(uuid::Uuid::new_v4().to_string()),
                        parent_id: None,
                        thread_id: None,
                        direction: zeus_core::TextDirection::default(), channel_source: None, compaction_hint: Default::default(),
                    });
                }
                zeus_llm::StopReason::MaxTokens => {
                    warn!("Subagent {} response truncated", self.id);
                    // Continue
                }
                zeus_llm::StopReason::Error => {
                    return SubagentResult {
                        id: self.id,
                        success: false,
                        output: "LLM error".to_string(),
                        iterations,
                        mission_id,
                    };
                }
            }
        }

        SubagentResult {
            id: self.id,
            success: true, // Completed, just hit max iterations
            output: last_output,
            iterations,
            mission_id,
        }
    }

    async fn build_system_prompt(&self) -> Result<String> {
        // S102 #23: Use parent's system prompt if available for prompt cache sharing.
        // Same prefix → API cache hit → subagents cost ~same as 1 agent.
        let base = if let Some(ref parent_prompt) = self.config.parent_system_prompt {
            parent_prompt.clone()
        } else {
            self.workspace.get_context().await?
        };

        Ok(format!(
            "{}\n\n# Subagent Context\n\nYou are a subagent with a specific task. Complete the task and report your results.\n\n## Task\n{}\n\n## Additional Context\n{}",
            base, self.config.task, self.config.context
        ))
    }
}

/// Spawn a subagent — local or remote based on `config.target`.
pub fn spawn_subagent(
    config: SubagentConfig,
    llm: LlmClient,
    workspace: Workspace,
    aegis: Option<Arc<Aegis>>,
) -> tokio::task::JoinHandle<SubagentResult> {
    match config.target.clone() {
        AgentTarget::Local => tokio::spawn(async move {
            let subagent = Subagent::new(config, llm, workspace, aegis);
            subagent.run().await
        }),
        AgentTarget::Remote {
            gateway_url,
            auth_token,
        } => {
            tokio::spawn(
                async move { spawn_remote(&gateway_url, auth_token.as_deref(), &config).await },
            )
        }
    }
}

/// POST a task to a remote Zeus gateway's `/v1/agents/run-task` endpoint.
///
/// The remote gateway will create a local agent and run the task.
/// Returns the result once the remote agent completes.
async fn spawn_remote(
    gateway_url: &str,
    auth_token: Option<&str>,
    config: &SubagentConfig,
) -> SubagentResult {
    let id = format!(
        "remote-{}",
        Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("unknown")
    );
    let mission_id = config.mission_id.clone();

    let url = format!("{}/v1/agents/run-task", gateway_url.trim_end_matches('/'));
    info!(
        "Spawning remote agent {} at {} for task: {}",
        id, url, config.task
    );

    let mut body = serde_json::json!({
        "task": config.task,
        "context": config.context,
        "max_iterations": config.max_iterations,
        "wait": true,
    });
    // Forward parent's LLM model so remote gateway uses the same provider
    if let Some(ref model) = config.model {
        body["model"] = serde_json::Value::String(model.clone());
    }

    let client = reqwest::Client::new();
    let mut req = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(300));

    if let Some(token) = auth_token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.text().await {
                Ok(text) => {
                    if status.is_success() {
                        // Try to parse the response as JSON
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            let output = json["output"]
                                .as_str()
                                .or_else(|| json["message"].as_str())
                                .unwrap_or(&text)
                                .to_string();
                            let iterations = json["iterations"].as_u64().unwrap_or(0) as usize;
                            SubagentResult {
                                id,
                                success: json["success"].as_bool().unwrap_or(true),
                                output,
                                iterations,
                                mission_id,
                            }
                        } else {
                            SubagentResult {
                                id,
                                success: true,
                                output: text,
                                iterations: 0,
                                mission_id,
                            }
                        }
                    } else {
                        SubagentResult {
                            id,
                            success: false,
                            output: format!("Remote gateway returned {}: {}", status, text),
                            iterations: 0,
                            mission_id,
                        }
                    }
                }
                Err(e) => SubagentResult {
                    id,
                    success: false,
                    output: format!("Failed to read remote response: {}", e),
                    iterations: 0,
                    mission_id,
                },
            }
        }
        Err(e) => SubagentResult {
            id,
            success: false,
            output: format!("Failed to reach remote gateway at {}: {}", gateway_url, e),
            iterations: 0,
            mission_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_config_defaults() {
        let config = SubagentConfig::default();
        assert_eq!(config.max_iterations, 15);
        assert!(!config.can_spawn);
        assert!(config.task.is_empty());
        assert!(config.context.is_empty());
    }

    #[test]
    fn test_subagent_config_custom() {
        let config = SubagentConfig {
            max_iterations: 30,
            can_spawn: true,
            task: "Write a report".to_string(),
            context: "Use formal language".to_string(),
            ..Default::default()
        };
        assert_eq!(config.max_iterations, 30);
        assert!(config.can_spawn);
        assert_eq!(config.task, "Write a report");
        assert_eq!(config.context, "Use formal language");
    }

    #[test]
    fn test_subagent_config_clone() {
        let config = SubagentConfig {
            max_iterations: 10,
            can_spawn: true,
            task: "task".to_string(),
            context: "ctx".to_string(),
            ..Default::default()
        };
        let cloned = config.clone();
        assert_eq!(cloned.max_iterations, config.max_iterations);
        assert_eq!(cloned.can_spawn, config.can_spawn);
        assert_eq!(cloned.task, config.task);
        assert_eq!(cloned.context, config.context);
    }

    #[test]
    fn test_subagent_result_success() {
        let result = SubagentResult {
            id: "sub-abc123".to_string(),
            success: true,
            output: "Task completed".to_string(),
            iterations: 3,
            mission_id: None,
        };
        assert!(result.success);
        assert_eq!(result.iterations, 3);
        assert_eq!(result.output, "Task completed");
        assert!(result.id.starts_with("sub-"));
    }

    #[test]
    fn test_subagent_result_failure() {
        let result = SubagentResult {
            id: "sub-def456".to_string(),
            success: false,
            output: "LLM error: timeout".to_string(),
            iterations: 1,
            mission_id: None,
        };
        assert!(!result.success);
        assert_eq!(result.iterations, 1);
        assert!(result.output.contains("error"));
    }

    #[test]
    fn test_subagent_result_clone() {
        let result = SubagentResult {
            id: "sub-xxx".to_string(),
            success: true,
            output: "done".to_string(),
            iterations: 5,
            mission_id: None,
        };
        let cloned = result.clone();
        assert_eq!(cloned.id, result.id);
        assert_eq!(cloned.success, result.success);
        assert_eq!(cloned.output, result.output);
        assert_eq!(cloned.iterations, result.iterations);
    }

    #[test]
    fn test_subagent_config_debug() {
        let config = SubagentConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("SubagentConfig"));
        assert!(debug_str.contains("max_iterations"));
        assert!(debug_str.contains("can_spawn"));
    }

    #[test]
    fn test_subagent_result_debug() {
        let result = SubagentResult {
            id: "sub-test".to_string(),
            success: true,
            output: "output".to_string(),
            iterations: 2,
            mission_id: None,
        };
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("SubagentResult"));
        assert!(debug_str.contains("sub-test"));
    }

    #[test]
    fn test_subagent_config_zero_iterations() {
        let config = SubagentConfig {
            max_iterations: 0,
            can_spawn: false,
            task: "impossible task".to_string(),
            context: String::new(),
            ..Default::default()
        };
        assert_eq!(config.max_iterations, 0);
    }

    #[test]
    fn test_subagent_config_large_iterations() {
        let config = SubagentConfig {
            max_iterations: 1000,
            can_spawn: true,
            task: "long running task".to_string(),
            context: "extra context".to_string(),
            ..Default::default()
        };
        assert_eq!(config.max_iterations, 1000);
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_subagent_config_max_iterations() {
        let config = SubagentConfig {
            max_iterations: usize::MAX,
            can_spawn: true,
            task: "stress test".to_string(),
            context: "very high iteration count".to_string(),
            ..Default::default()
        };
        assert_eq!(config.max_iterations, usize::MAX);
        assert!(config.can_spawn);
    }

    #[test]
    fn test_subagent_result_with_empty_output() {
        let result = SubagentResult {
            id: "sub-empty".to_string(),
            success: true,
            output: String::new(),
            iterations: 1,
            mission_id: None,
        };
        assert!(result.success);
        assert!(result.output.is_empty());
        assert_eq!(result.iterations, 1);
    }

    #[test]
    fn test_subagent_result_with_long_output() {
        let long_output = "x".repeat(1_000_000);
        let result = SubagentResult {
            id: "sub-long".to_string(),
            success: true,
            output: long_output.clone(),
            iterations: 5,
            mission_id: None,
        };
        assert_eq!(result.output.len(), 1_000_000);
        assert_eq!(result.output, long_output);
    }

    #[test]
    fn test_subagent_config_equality() {
        let config_a = SubagentConfig {
            max_iterations: 10,
            can_spawn: false,
            task: "same task".to_string(),
            context: "same context".to_string(),
            ..Default::default()
        };
        let config_b = SubagentConfig {
            max_iterations: 10,
            can_spawn: false,
            task: "same task".to_string(),
            context: "same context".to_string(),
            ..Default::default()
        };
        // SubagentConfig derives Clone and Debug but not PartialEq,
        // so compare fields directly
        assert_eq!(config_a.max_iterations, config_b.max_iterations);
        assert_eq!(config_a.can_spawn, config_b.can_spawn);
        assert_eq!(config_a.task, config_b.task);
        assert_eq!(config_a.context, config_b.context);
    }

    #[test]
    fn test_subagent_result_serialization() {
        // SubagentResult derives Debug and Clone
        // Test that it can be cloned and the clone is independent
        let original = SubagentResult {
            id: "sub-ser".to_string(),
            success: true,
            output: "serialized output".to_string(),
            iterations: 7,
            mission_id: None,
        };
        let mut cloned = original.clone();
        cloned.output = "modified".to_string();
        cloned.iterations = 99;

        // Original should be unchanged
        assert_eq!(original.output, "serialized output");
        assert_eq!(original.iterations, 7);
        // Clone should reflect changes
        assert_eq!(cloned.output, "modified");
        assert_eq!(cloned.iterations, 99);
    }

    #[test]
    fn test_subagent_config_display() {
        let config = SubagentConfig {
            max_iterations: 42,
            can_spawn: true,
            task: "display test task".to_string(),
            context: "display context".to_string(),
            ..Default::default()
        };
        let debug_output = format!("{:?}", config);
        assert!(debug_output.contains("42"));
        assert!(debug_output.contains("true"));
        assert!(debug_output.contains("display test task"));
        assert!(debug_output.contains("display context"));
    }

    // ================================================================
    // AgentTarget tests
    // ================================================================

    #[test]
    fn test_agent_target_default_is_local() {
        let target = AgentTarget::default();
        assert!(matches!(target, AgentTarget::Local));
    }

    #[test]
    fn test_agent_target_local_debug() {
        let target = AgentTarget::Local;
        let debug_str = format!("{:?}", target);
        assert!(debug_str.contains("Local"));
    }

    #[test]
    fn test_agent_target_remote_debug() {
        let target = AgentTarget::Remote {
            gateway_url: "http://192.168.1.100:8080".to_string(),
            auth_token: Some("secret".to_string()),
        };
        let debug_str = format!("{:?}", target);
        assert!(debug_str.contains("Remote"));
        assert!(debug_str.contains("192.168.1.100"));
    }

    #[test]
    fn test_agent_target_remote_without_auth() {
        let target = AgentTarget::Remote {
            gateway_url: "http://10.0.0.1:3000".to_string(),
            auth_token: None,
        };
        assert!(matches!(
            target,
            AgentTarget::Remote {
                auth_token: None,
                ..
            }
        ));
    }

    #[test]
    fn test_agent_target_clone() {
        let target = AgentTarget::Remote {
            gateway_url: "http://host:8080".to_string(),
            auth_token: Some("token123".to_string()),
        };
        let cloned = target.clone();
        if let AgentTarget::Remote {
            gateway_url,
            auth_token,
        } = cloned
        {
            assert_eq!(gateway_url, "http://host:8080");
            assert_eq!(auth_token, Some("token123".to_string()));
        } else {
            panic!("Expected Remote variant");
        }
    }

    #[test]
    fn test_subagent_config_with_remote_target() {
        let config = SubagentConfig {
            max_iterations: 10,
            can_spawn: false,
            task: "remote task".to_string(),
            context: "remote context".to_string(),
            target: AgentTarget::Remote {
                gateway_url: "http://192.168.1.200:8080".to_string(),
                auth_token: Some("bearer-xyz".to_string()),
            },
            model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            effort: None,
            tools: Vec::new(),
            mission_id: None, parent_system_prompt: None,
        };
        assert!(matches!(config.target, AgentTarget::Remote { .. }));
        if let AgentTarget::Remote {
            gateway_url,
            auth_token,
        } = &config.target
        {
            assert_eq!(gateway_url, "http://192.168.1.200:8080");
            assert_eq!(auth_token.as_deref(), Some("bearer-xyz"));
        }
    }

    #[test]
    fn test_subagent_config_default_target_is_local() {
        let config = SubagentConfig::default();
        assert!(matches!(config.target, AgentTarget::Local));
    }

    #[test]
    fn test_spawn_remote_result_format() {
        // Verify SubagentResult can represent remote outcomes
        let result = SubagentResult {
            id: "remote-abc123".to_string(),
            success: true,
            output: "Remote task completed successfully".to_string(),
            iterations: 5,
            mission_id: None,
        };
        assert!(result.id.starts_with("remote-"));
        assert!(result.success);
        assert_eq!(result.iterations, 5);
    }

    #[test]
    fn test_spawn_remote_failure_result() {
        let result = SubagentResult {
            id: "remote-fail".to_string(),
            success: false,
            output: "Failed to reach remote gateway at http://10.0.0.1:8080: connection refused"
                .to_string(),
            iterations: 0,
            mission_id: None,
        };
        assert!(!result.success);
        assert!(result.output.contains("remote gateway"));
        assert_eq!(result.iterations, 0);
    }
}
