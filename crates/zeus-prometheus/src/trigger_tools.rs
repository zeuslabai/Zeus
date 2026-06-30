//! S103 #34: Background agent triggers
//!
//! Exposes `create_trigger`, `list_triggers`, and `remove_trigger` as agent
//! tool calls. Each trigger runs a shell command on a cron schedule and injects
//! the output as a system message into the agent's next turn.

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use zeus_core::{Result, ToolSchema, TriggerExecutor};

use crate::scheduler::{CronScheduler, TaskConfig, TaskType};

// ─── Tool schemas ─────────────────────────────────────────────────────────────

/// Return the tool schemas for trigger management.
pub fn trigger_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "create_trigger".to_string(),
            description: "Create a background trigger that runs a shell command on a cron \
                schedule and injects the output as a system message before your next turn. \
                Use this to set up recurring checks, monitors, or data feeds. \
                For one-shot timed orders ('do X at 9pm'), pass run_at (RFC3339 timestamp) \
                instead of schedule — the trigger fires once at that time and is then \
                automatically removed."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Human-readable name for this trigger."
                    },
                    "schedule": {
                        "type": "string",
                        "description": "Cron expression (e.g. '*/5 * * * *') or human schedule \
                            (e.g. 'every 5 minutes', 'daily at 9am'). Optional if run_at is given."
                    },
                    "command": {
                        "type": "string",
                        "description": "Shell command to run. Its stdout will be injected as a \
                            system message."
                    },
                    "run_at": {
                        "type": "string",
                        "description": "One-shot: RFC3339 timestamp (e.g. '2026-06-10T21:00:00Z') \
                            to fire exactly once, then auto-remove. Overrides schedule."
                    },
                    "run_once": {
                        "type": "boolean",
                        "description": "If true, remove the trigger after its first firing \
                            (default: false)."
                    }
                },
                "required": ["name", "command"]
            }),
        },
        ToolSchema {
            name: "list_triggers".to_string(),
            description: "List all active background triggers with their schedules and last \
                run times."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSchema {
            name: "remove_trigger".to_string(),
            description: "Remove a background trigger by its ID (from list_triggers).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Trigger ID returned by create_trigger or list_triggers."
                    }
                },
                "required": ["id"]
            }),
        },
    ]
}

// ─── Handle ───────────────────────────────────────────────────────────────────

/// Shared handle passed to the tool executor so it can reach the scheduler.
#[derive(Clone)]
pub struct TriggerHandle {
    pub scheduler: Arc<RwLock<CronScheduler>>,
}

impl TriggerHandle {
    pub fn new(scheduler: Arc<RwLock<CronScheduler>>) -> Self {
        Self { scheduler }
    }

    /// Dispatch a trigger tool call. Returns a string result.
    pub async fn execute(&self, tool_name: &str, input: &Value) -> Result<String> {
        match tool_name {
            "create_trigger" => self.create_trigger(input).await,
            "list_triggers" => self.list_triggers().await,
            "remove_trigger" => self.remove_trigger(input).await,
            _ => Ok(format!("Unknown trigger tool: {tool_name}")),
        }
    }

    async fn create_trigger(&self, input: &Value) -> Result<String> {
        let name = input["name"].as_str().unwrap_or("unnamed").to_string();
        let schedule = input["cron"]
            .as_str()
            .or_else(|| input["schedule"].as_str())
            .unwrap_or("0 * * * *")
            .to_string();
        let command = input["command"].as_str().unwrap_or("").to_string();

        if command.is_empty() {
            return Ok("Error: command is required".to_string());
        }

        // One-shot support: `run_at` (RFC3339) fires once at the timestamp and
        // is implicitly run-once. Without it, agents could only create
        // recurring cron triggers — "do X at 9pm tonight" was impossible.
        let run_at = match input["run_at"].as_str() {
            Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                Ok(dt) => {
                    let dt = dt.with_timezone(&chrono::Utc);
                    if dt <= chrono::Utc::now() {
                        return Ok(format!(
                            "Error: run_at '{ts}' is in the past (now: {})",
                            chrono::Utc::now().to_rfc3339()
                        ));
                    }
                    Some(dt)
                }
                Err(e) => {
                    return Ok(format!(
                        "Error: run_at '{ts}' is not a valid RFC3339 timestamp ({e}). \
                         Example: 2026-06-10T21:00:00Z"
                    ))
                }
            },
            None => None,
        };
        let run_once = input["run_once"].as_bool().unwrap_or(false);

        if run_at.is_none() && input["schedule"].is_null() && input["cron"].is_null() {
            return Ok(
                "Error: either schedule (cron) or run_at (one-shot) is required".to_string(),
            );
        }

        let config = TaskConfig {
            name: name.clone(),
            cron: schedule.clone(),
            task_type: TaskType::Shell {
                command: command.clone(),
            },
            enabled: true,
            run_at,
            run_once,
            wake_mode: crate::scheduler::WakeMode::Now,
            delivery_mode: crate::scheduler::DeliveryMode::Channel,
        };

        let id = self.scheduler.read().await.add_task(config).await?;

        let when = match run_at {
            Some(at) => format!("One-shot at: {}", at.to_rfc3339()),
            None => format!(
                "Schedule: {schedule}{}",
                if run_once { " (run once)" } else { "" }
            ),
        };
        Ok(format!(
            "Trigger created.\nID: {id}\nName: {name}\n{when}\nCommand: {command}\n\
             Output will be injected as a system message before each agent turn."
        ))
    }

    async fn list_triggers(&self) -> Result<String> {
        let tasks = self.scheduler.read().await.list_tasks().await;

        let shell_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| matches!(t.task_type, TaskType::Shell { .. }))
            .collect();

        if shell_tasks.is_empty() {
            return Ok("No active triggers.".to_string());
        }

        let mut out = format!("{} active trigger(s):\n\n", shell_tasks.len());
        for t in shell_tasks {
            let command = match &t.task_type {
                TaskType::Shell { command } => command.as_str(),
                _ => "",
            };
            let last = t
                .last_run
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "never".to_string());
            let next = t
                .next_run
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "N/A".to_string());

            out.push_str(&format!(
                "ID: {}\nName: {}\nSchedule: {}\nCommand: {}\nLast: {}\nNext: {}\n\n",
                t.id, t.name, t.cron_expr, command, last, next
            ));
        }

        Ok(out.trim_end().to_string())
    }

    async fn remove_trigger(&self, input: &Value) -> Result<String> {
        let id = match input["id"].as_str() {
            Some(id) => id,
            None => return Ok("Error: id is required".to_string()),
        };

        self.scheduler.read().await.remove_task(id).await?;
        Ok(format!("Trigger '{id}' removed."))
    }
}

// Implement the zeus-core trait so the gateway can pass TriggerHandle as
// Arc<dyn TriggerExecutor> to zeus-agent's ToolRegistry without a direct
// dependency from zeus-agent → zeus-prometheus.
#[async_trait::async_trait]
impl TriggerExecutor for TriggerHandle {
    async fn execute(&self, tool_name: &str, input: &serde_json::Value) -> Result<String> {
        // Delegate to the inherent method.
        TriggerHandle::execute(self, tool_name, input).await
    }
}
