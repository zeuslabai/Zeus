//! AgentToolExecutor - bridges Agent's ToolRegistry to Prometheus's ToolExecutor trait
//!
//! This lives in the binary crate because it depends on both zeus-agent and zeus-prometheus,
//! which cannot depend on each other.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use zeus_aegis::Aegis;
use zeus_agent::{AgentTarget, SubagentConfig, SubagentResult, ToolRegistry};
use zeus_channels::ChannelManager;
use zeus_core::{ToolCall, ToolResult};
use zeus_llm::LlmClient;
use zeus_prometheus::ToolExecutor;

/// Shared registry of background subagent handles.
/// Arc<Mutex<...>> so it can be cloned across AgentToolExecutor instances and
/// mutated inside `execute_tool(&self)` which takes a shared reference.
type SpawnRegistry = Arc<Mutex<HashMap<String, JoinHandle<SubagentResult>>>>;

/// Implements Prometheus's ToolExecutor by delegating to Agent's ToolRegistry.
///
/// Holds an optional `ChannelManager` so that `send_file` calls issued from
/// the cooking loop (which has no Agent context) can be routed through the
/// shared standalone `send_file_to_channel` function extracted by zeus107.
pub struct AgentToolExecutor {
    registry: ToolRegistry,
    aegis: Option<Arc<Aegis>>,
    channels: Option<Arc<ChannelManager>>,
    llm: Option<Arc<LlmClient>>,
    /// Tracks background subagent handles for collect_spawns.
    spawn_registry: SpawnRegistry,
}

impl AgentToolExecutor {
    pub fn new(registry: ToolRegistry, aegis: Option<Arc<Aegis>>) -> Self {
        Self {
            registry,
            aegis,
            channels: None,
            llm: None,
            spawn_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Attach a ChannelManager so `send_file` works from the cooking loop.
    pub fn with_channels(mut self, channels: Arc<ChannelManager>) -> Self {
        self.channels = Some(channels);
        self
    }

    /// Attach an LlmClient so `deep_research` and `spawn` work from the cooking loop.
    pub fn with_llm(mut self, llm: Arc<LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }
}

#[async_trait]
impl ToolExecutor for AgentToolExecutor {
    async fn execute_tool(&self, call: &ToolCall) -> ToolResult {
        // Aegis security check
        if let Some(ref aegis) = self.aegis
            && !aegis.is_permitted(&call.name)
        {
            return ToolResult {
                call_id: call.id.clone(),
                success: false,
                output: format!(
                    "BLOCKED by security policy: tool '{}' is not permitted under the current \
                     sandbox level. Do NOT retry this tool — choose an alternative approach or \
                     inform the user that this action requires elevated permissions.",
                    call.name
                ),
            };
        }

        // ── spawn ─────────────────────────────────────────────────────────────
        if call.name == "spawn" {
            let llm = match &self.llm {
                Some(l) => l.clone(),
                None => {
                    return ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: "spawn: no LLM context available. This is a configuration error."
                            .to_string(),
                    };
                }
            };

            let args = &call.arguments;

            let task = match args.get("task").and_then(|t| t.as_str()) {
                Some(t) => t.to_string(),
                None => {
                    return ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: "Missing 'task' argument".to_string(),
                    };
                }
            };

            let context = args
                .get("context")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            let max_iterations = args
                .get("max_iterations")
                .and_then(|m| m.as_u64())
                .unwrap_or(15) as usize;

            let wait = args.get("wait").and_then(|w| w.as_bool()).unwrap_or(false);

            let target = if let Some(gw) = args.get("gateway_url").and_then(|g| g.as_str()) {
                let auth_token = args
                    .get("auth_token")
                    .and_then(|a| a.as_str())
                    .map(String::from);
                AgentTarget::Remote {
                    gateway_url: gw.to_string(),
                    auth_token,
                }
            } else {
                AgentTarget::Local
            };

            let mission_id = args
                .get("mission_id")
                .and_then(|m| m.as_str())
                .map(String::from);

            let model = llm.model().to_string();

            let config = SubagentConfig {
                max_iterations,
                can_spawn: false,
                task: task.clone(),
                context,
                target,
                model: Some(model),
                mission_id,
                parent_system_prompt: None,
            };

            if wait {
                // Run synchronously and return result inline
                let subagent =
                    zeus_agent::Subagent::new(config, (*llm).clone(), zeus_memory::Workspace::new("/tmp"), None);
                let result = subagent.run().await;
                return ToolResult {
                    call_id: call.id.clone(),
                    success: result.success,
                    output: format!(
                        "Subagent completed in {} iterations:\n{}",
                        result.iterations, result.output
                    ),
                };
            } else {
                let subagent_id = format!(
                    "cooking-{}",
                    uuid::Uuid::new_v4()
                        .to_string()
                        .split('-')
                        .next()
                        .unwrap_or("sub")
                        .to_string()
                );
                let handle =
                    zeus_agent::spawn_subagent(config, (*llm).clone(), zeus_memory::Workspace::new("/tmp"), None);
                self.spawn_registry
                    .lock()
                    .await
                    .insert(subagent_id.clone(), handle);
                return ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: format!(
                        "Subagent '{}' spawned in background for task: {}. Use collect_spawns to retrieve results.",
                        subagent_id, task
                    ),
                };
            }
        }

        // ── collect_spawns ────────────────────────────────────────────────────
        if call.name == "collect_spawns" {
            let timeout_secs = call
                .arguments
                .get("timeout_seconds")
                .and_then(|t| t.as_u64())
                .unwrap_or(300);

            let mut registry = self.spawn_registry.lock().await;

            if registry.is_empty() {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: "No subagents running. Nothing to collect.".to_string(),
                };
            }

            let ids: Vec<String> = registry.keys().cloned().collect();
            let handles: Vec<(String, JoinHandle<SubagentResult>)> = ids
                .into_iter()
                .filter_map(|id| registry.remove(&id).map(|h| (id, h)))
                .collect();
            drop(registry); // release lock before awaiting

            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

            let mut results: Vec<serde_json::Value> = Vec::new();
            for (id, handle) in handles {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                let r = match tokio::time::timeout(remaining, handle).await {
                    Ok(Ok(r)) => serde_json::json!({
                        "subagent_id": id,
                        "success": r.success,
                        "output": r.output,
                        "iterations": r.iterations,
                        "mission_id": r.mission_id,
                    }),
                    Ok(Err(e)) => serde_json::json!({
                        "subagent_id": id,
                        "success": false,
                        "output": format!("JoinError: {}", e),
                    }),
                    Err(_) => serde_json::json!({
                        "subagent_id": id,
                        "success": false,
                        "output": "Timed out waiting for subagent",
                    }),
                };
                results.push(r);
            }

            let succeeded = results.iter().filter(|r| r["success"].as_bool().unwrap_or(false)).count();
            let failed = results.len() - succeeded;

            return ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: serde_json::to_string_pretty(&serde_json::json!({
                    "collected": results.len(),
                    "succeeded": succeeded,
                    "failed": failed,
                    "results": results,
                }))
                .unwrap_or_else(|_| "Failed to serialize results".to_string()),
            };
        }

        // ── deep_research ─────────────────────────────────────────────────────
        if call.name == "deep_research" {
            let mut result = match &self.llm {
                Some(llm) => zeus_agent::execute_deep_research(&call.arguments, llm).await,
                None => ToolResult {
                    call_id: String::new(),
                    success: false,
                    output: "deep_research: no LLM context available (LlmClient not attached \
                             to executor). This is a configuration error."
                        .to_string(),
                },
            };
            result.call_id = call.id.clone();
            return result;
        }

        // ── send_file ─────────────────────────────────────────────────────────
        if call.name == "send_file" {
            let mut result = match &self.channels {
                Some(channels) => {
                    zeus_agent::send_file_to_channel(&call.arguments, channels).await
                }
                None => ToolResult {
                    call_id: String::new(),
                    success: false,
                    output: "send_file: no channel context available (ChannelManager not \
                             attached to executor). This is a configuration error."
                        .to_string(),
                },
            };
            result.call_id = call.id.clone();
            return result;
        }

        // Route deep_research through the standalone function that takes an LlmClient.
        if call.name == "deep_research" {
            let mut result = match &self.llm {
                Some(llm) => {
                    zeus_agent::execute_deep_research(&call.arguments, llm).await
                }
                None => ToolResult {
                    call_id: String::new(),
                    success: false,
                    output: "deep_research: no LLM context available (LlmClient not \
                             attached to executor). This is a configuration error."
                        .to_string(),
                },
            };
            result.call_id = call.id.clone();
            return result;
        }

        // ── everything else → registry ────────────────────────────────────────
        match self
            .registry
            .execute(&call.name, call.arguments.clone())
            .await
        {
            Ok(output) => ToolResult {
                call_id: call.id.clone(),
                success: true,
                output,
            },
            Err(e) => {
                let error_str = e.to_string();
                let guidance = if error_str.contains("blocked by security policy")
                    || error_str.contains("not permitted")
                    || error_str.contains("denied")
                {
                    " [RECOVERY: This path/command is restricted. Try an alternative \
                     path, a different tool, or inform the user about the restriction.]"
                } else if error_str.contains("Unknown tool") {
                    " [RECOVERY: This tool is not available. Check available_tools or \
                     use a different approach.]"
                } else if error_str.contains("timed out") || error_str.contains("Timeout") {
                    " [RECOVERY: The operation timed out. Retry with a simpler command \
                     or break the task into smaller steps.]"
                } else if error_str.contains("not found") || error_str.contains("No such file") {
                    " [RECOVERY: The target was not found. Verify the path/name exists \
                     before retrying.]"
                } else {
                    " [RECOVERY: Tool execution failed. Review the error, adjust \
                     parameters, and retry or use an alternative approach.]"
                };
                ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: format!("ERROR: {}{}", error_str, guidance),
                }
            }
        }
    }

    fn has_tool(&self, name: &str) -> bool {
        self.registry.schemas().iter().any(|s| s.name == name)
    }

    fn available_tools(&self) -> Vec<String> {
        self.registry
            .schemas()
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }
}
