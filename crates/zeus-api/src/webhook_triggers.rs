//! Webhook Trigger-Action Pipelines
//!
//! Inbound webhook automations that match incoming webhook events against
//! configurable triggers and dispatch actions (route to agents, execute tools,
//! forward to channels, or call outbound webhooks).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::{debug, info};

// ============================================================================
// Trigger Matching
// ============================================================================

/// A condition that must be satisfied for a trigger to fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerCondition {
    /// Match when `source` equals the given value (case-insensitive).
    SourceEquals { value: String },
    /// Match when `channel` equals the given value (case-insensitive).
    ChannelEquals { value: String },
    /// Match when `sender` equals the given value (case-insensitive).
    SenderEquals { value: String },
    /// Match when `message` contains the given substring (case-insensitive).
    MessageContains { value: String },
    /// Match when `message` starts with the given prefix (case-insensitive).
    MessageStartsWith { value: String },
    /// Match when a metadata key exists and optionally equals a value.
    MetadataField { key: String, value: Option<String> },
    /// Match when all sub-conditions are true.
    All { conditions: Vec<TriggerCondition> },
    /// Match when any sub-condition is true.
    Any { conditions: Vec<TriggerCondition> },
    /// Always matches (catch-all).
    Always,
}

impl TriggerCondition {
    /// Evaluate this condition against a webhook event.
    pub fn matches(&self, event: &WebhookEvent) -> bool {
        match self {
            Self::SourceEquals { value } => event
                .source
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(value))
                .unwrap_or(false),
            Self::ChannelEquals { value } => event
                .channel
                .as_deref()
                .map(|c| c.eq_ignore_ascii_case(value))
                .unwrap_or(false),
            Self::SenderEquals { value } => event
                .sender
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(value))
                .unwrap_or(false),
            Self::MessageContains { value } => {
                event.message.to_lowercase().contains(&value.to_lowercase())
            }
            Self::MessageStartsWith { value } => event
                .message
                .to_lowercase()
                .starts_with(&value.to_lowercase()),
            Self::MetadataField { key, value } => match &event.metadata {
                Some(meta) => match meta.get(key) {
                    Some(v) => match value {
                        Some(expected) => {
                            v.as_str().map(|s| s == expected).unwrap_or(false)
                                || v.to_string().trim_matches('"') == expected
                        }
                        None => true, // key exists, no value check
                    },
                    None => false,
                },
                None => false,
            },
            Self::All { conditions } => conditions.iter().all(|c| c.matches(event)),
            Self::Any { conditions } => conditions.iter().any(|c| c.matches(event)),
            Self::Always => true,
        }
    }
}

// ============================================================================
// Actions
// ============================================================================

/// Action to execute when a trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerAction {
    /// Route the message to a specific agent by ID.
    RouteToAgent {
        agent_id: String,
        /// Optional prompt template. `{message}` is replaced with the webhook message.
        #[serde(default)]
        prompt_template: Option<String>,
    },
    /// Execute a named tool with the message as input.
    ExecuteTool {
        tool_name: String,
        /// Static arguments merged with `{"input": message}`.
        #[serde(default)]
        arguments: Option<Value>,
    },
    /// Forward the message to a channel (Telegram, Discord, Slack, etc.).
    ForwardToChannel {
        channel_type: String,
        chat_id: String,
        #[serde(default)]
        prefix: Option<String>,
    },
    /// Call an outbound webhook URL with the event payload.
    CallWebhook {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    /// Log the event (no-op action useful for auditing).
    Log {
        #[serde(default)]
        label: Option<String>,
    },
    /// Execute multiple actions in sequence.
    Chain { actions: Vec<TriggerAction> },
}

// ============================================================================
// WebhookTrigger
// ============================================================================

/// A webhook automation trigger: condition + action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTrigger {
    /// Unique trigger ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// When this condition matches, the action fires.
    pub condition: TriggerCondition,
    /// Action(s) to execute.
    pub action: TriggerAction,
    /// Whether this trigger is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Priority (lower = higher priority, evaluated first). Default: 100.
    #[serde(default = "default_priority")]
    pub priority: u32,
    /// When this trigger was created.
    pub created_at: DateTime<Utc>,
    /// How many times this trigger has fired.
    #[serde(default)]
    pub fire_count: u64,
    /// Last time this trigger fired.
    #[serde(default)]
    pub last_fired_at: Option<DateTime<Utc>>,
}

fn default_true() -> bool {
    true
}
fn default_priority() -> u32 {
    100
}

// ============================================================================
// WebhookEvent (normalized inbound event)
// ============================================================================

/// A normalized inbound webhook event for trigger evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub source: Option<String>,
    pub message: String,
    pub sender: Option<String>,
    pub channel: Option<String>,
    pub metadata: Option<Value>,
    pub timestamp: DateTime<Utc>,
}

impl WebhookEvent {
    /// Create from the raw webhook payload fields.
    pub fn new(
        source: Option<String>,
        message: String,
        sender: Option<String>,
        channel: Option<String>,
        metadata: Option<Value>,
    ) -> Self {
        Self {
            source,
            message,
            sender,
            channel,
            metadata,
            timestamp: Utc::now(),
        }
    }
}

// ============================================================================
// ActionResult
// ============================================================================

/// Result of dispatching a trigger action.
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub trigger_id: String,
    pub trigger_name: String,
    pub action_type: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

// ============================================================================
// TriggerEngine
// ============================================================================

/// Manages webhook triggers and evaluates incoming events against them.
pub struct TriggerEngine {
    store_path: PathBuf,
    triggers: RwLock<Vec<WebhookTrigger>>,
}

impl TriggerEngine {
    /// Create a new TriggerEngine persisting triggers to `workspace_dir/webhook_triggers.json`.
    pub fn new(workspace_dir: &Path) -> Self {
        let store_path = workspace_dir.join("webhook_triggers.json");
        let triggers = if store_path.exists() {
            match std::fs::read_to_string(&store_path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Self {
            store_path,
            triggers: RwLock::new(triggers),
        }
    }

    /// Persist triggers to disk.
    async fn persist(&self) -> Result<(), String> {
        let triggers = self.triggers.read().await;
        let data = serde_json::to_string_pretty(&*triggers)
            .map_err(|e| format!("serialize error: {}", e))?;
        if let Some(parent) = self.store_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir error: {}", e))?;
        }
        tokio::fs::write(&self.store_path, data)
            .await
            .map_err(|e| format!("write error: {}", e))?;
        Ok(())
    }

    /// List all triggers (sorted by priority ascending).
    pub async fn list(&self) -> Vec<WebhookTrigger> {
        let mut triggers = self.triggers.read().await.clone();
        triggers.sort_by_key(|t| t.priority);
        triggers
    }

    /// Get a trigger by ID.
    pub async fn get(&self, id: &str) -> Option<WebhookTrigger> {
        self.triggers
            .read()
            .await
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    /// Create a new trigger.
    pub async fn create(
        &self,
        name: String,
        description: Option<String>,
        condition: TriggerCondition,
        action: TriggerAction,
        priority: Option<u32>,
    ) -> Result<WebhookTrigger, String> {
        if name.is_empty() {
            return Err("trigger name is required".to_string());
        }

        let trigger = WebhookTrigger {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description,
            condition,
            action,
            enabled: true,
            priority: priority.unwrap_or(100),
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };

        {
            let mut triggers = self.triggers.write().await;
            triggers.push(trigger.clone());
        }
        self.persist().await?;

        info!(
            "Created webhook trigger '{}' ({})",
            trigger.name, trigger.id
        );
        Ok(trigger)
    }

    /// Update a trigger's enabled state.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        {
            let mut triggers = self.triggers.write().await;
            let trigger = triggers
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("trigger '{}' not found", id))?;
            trigger.enabled = enabled;
        }
        self.persist().await
    }

    /// Delete a trigger by ID.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        {
            let mut triggers = self.triggers.write().await;
            let before = triggers.len();
            triggers.retain(|t| t.id != id);
            if triggers.len() == before {
                return Err(format!("trigger '{}' not found", id));
            }
        }
        self.persist().await?;
        info!("Deleted webhook trigger {}", id);
        Ok(())
    }

    /// Evaluate an event against all enabled triggers.
    /// Returns matching triggers sorted by priority (lowest first).
    pub async fn evaluate(&self, event: &WebhookEvent) -> Vec<WebhookTrigger> {
        let triggers = self.triggers.read().await;
        let mut matched: Vec<WebhookTrigger> = triggers
            .iter()
            .filter(|t| t.enabled && t.condition.matches(event))
            .cloned()
            .collect();
        matched.sort_by_key(|t| t.priority);
        matched
    }

    /// Record that a trigger fired (bump count + timestamp).
    pub async fn record_fire(&self, id: &str) {
        {
            let mut triggers = self.triggers.write().await;
            if let Some(t) = triggers.iter_mut().find(|t| t.id == id) {
                t.fire_count += 1;
                t.last_fired_at = Some(Utc::now());
            }
        }
        let _ = self.persist().await;
    }

    /// Dispatch a single action and return the result.
    /// This handles Log actions directly; other actions return metadata
    /// for the caller (e.g. the webhook handler) to execute.
    pub fn dispatch_action(trigger: &WebhookTrigger, event: &WebhookEvent) -> ActionResult {
        let action_type = match &trigger.action {
            TriggerAction::RouteToAgent {
                agent_id,
                prompt_template,
            } => {
                let prompt = match prompt_template {
                    Some(tpl) => tpl.replace("{message}", &event.message),
                    None => format!(
                        "[Webhook from {}/{}] {}: {}",
                        event.source.as_deref().unwrap_or("unknown"),
                        event.channel.as_deref().unwrap_or("default"),
                        event.sender.as_deref().unwrap_or("anonymous"),
                        event.message
                    ),
                };
                return ActionResult {
                    trigger_id: trigger.id.clone(),
                    trigger_name: trigger.name.clone(),
                    action_type: "route_to_agent".to_string(),
                    success: true,
                    output: Some(
                        serde_json::json!({
                            "agent_id": agent_id,
                            "prompt": prompt,
                        })
                        .to_string(),
                    ),
                    error: None,
                };
            }
            TriggerAction::ExecuteTool {
                tool_name,
                arguments,
            } => {
                let mut args = arguments.clone().unwrap_or(serde_json::json!({}));
                if let Some(obj) = args.as_object_mut() {
                    obj.insert("input".to_string(), serde_json::json!(event.message));
                }
                return ActionResult {
                    trigger_id: trigger.id.clone(),
                    trigger_name: trigger.name.clone(),
                    action_type: "execute_tool".to_string(),
                    success: true,
                    output: Some(
                        serde_json::json!({
                            "tool_name": tool_name,
                            "arguments": args,
                        })
                        .to_string(),
                    ),
                    error: None,
                };
            }
            TriggerAction::ForwardToChannel {
                channel_type,
                chat_id,
                prefix,
            } => {
                let content = match prefix {
                    Some(p) => format!("{} {}", p, event.message),
                    None => event.message.clone(),
                };
                return ActionResult {
                    trigger_id: trigger.id.clone(),
                    trigger_name: trigger.name.clone(),
                    action_type: "forward_to_channel".to_string(),
                    success: true,
                    output: Some(
                        serde_json::json!({
                            "channel_type": channel_type,
                            "chat_id": chat_id,
                            "content": content,
                        })
                        .to_string(),
                    ),
                    error: None,
                };
            }
            TriggerAction::CallWebhook { url, headers } => {
                return ActionResult {
                    trigger_id: trigger.id.clone(),
                    trigger_name: trigger.name.clone(),
                    action_type: "call_webhook".to_string(),
                    success: true,
                    output: Some(
                        serde_json::json!({
                            "url": url,
                            "headers": headers,
                            "payload": event,
                        })
                        .to_string(),
                    ),
                    error: None,
                };
            }
            TriggerAction::Log { label } => {
                let tag = label.as_deref().unwrap_or("webhook-trigger");
                debug!(
                    "[{}] Trigger '{}' matched: {} from {:?}",
                    tag, trigger.name, event.message, event.source
                );
                "log"
            }
            TriggerAction::Chain { actions } => {
                // Return chain metadata; caller iterates sub-actions
                let action_types: Vec<String> = actions
                    .iter()
                    .map(|a| match a {
                        TriggerAction::RouteToAgent { .. } => "route_to_agent".to_string(),
                        TriggerAction::ExecuteTool { .. } => "execute_tool".to_string(),
                        TriggerAction::ForwardToChannel { .. } => "forward_to_channel".to_string(),
                        TriggerAction::CallWebhook { .. } => "call_webhook".to_string(),
                        TriggerAction::Log { .. } => "log".to_string(),
                        TriggerAction::Chain { .. } => "chain".to_string(),
                    })
                    .collect();
                return ActionResult {
                    trigger_id: trigger.id.clone(),
                    trigger_name: trigger.name.clone(),
                    action_type: "chain".to_string(),
                    success: true,
                    output: Some(
                        serde_json::json!({
                            "action_count": actions.len(),
                            "action_types": action_types,
                        })
                        .to_string(),
                    ),
                    error: None,
                };
            }
        };

        ActionResult {
            trigger_id: trigger.id.clone(),
            trigger_name: trigger.name.clone(),
            action_type: action_type.to_string(),
            success: true,
            output: None,
            error: None,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_event(source: &str, message: &str) -> WebhookEvent {
        WebhookEvent::new(
            Some(source.to_string()),
            message.to_string(),
            Some("test-user".to_string()),
            Some("general".to_string()),
            None,
        )
    }

    fn test_event_with_metadata(source: &str, message: &str, meta: Value) -> WebhookEvent {
        WebhookEvent::new(
            Some(source.to_string()),
            message.to_string(),
            Some("test-user".to_string()),
            Some("general".to_string()),
            Some(meta),
        )
    }

    // -- Condition tests --

    #[test]
    fn test_source_equals() {
        let cond = TriggerCondition::SourceEquals {
            value: "github".to_string(),
        };
        assert!(cond.matches(&test_event("github", "push event")));
        assert!(cond.matches(&test_event("GitHub", "push event"))); // case-insensitive
        assert!(!cond.matches(&test_event("gitlab", "push event")));
    }

    #[test]
    fn test_channel_equals() {
        let cond = TriggerCondition::ChannelEquals {
            value: "general".to_string(),
        };
        assert!(cond.matches(&test_event("github", "hello")));

        let cond2 = TriggerCondition::ChannelEquals {
            value: "alerts".to_string(),
        };
        assert!(!cond2.matches(&test_event("github", "hello")));
    }

    #[test]
    fn test_sender_equals() {
        let cond = TriggerCondition::SenderEquals {
            value: "test-user".to_string(),
        };
        assert!(cond.matches(&test_event("github", "hello")));

        let cond2 = TriggerCondition::SenderEquals {
            value: "other-user".to_string(),
        };
        assert!(!cond2.matches(&test_event("github", "hello")));
    }

    #[test]
    fn test_message_contains() {
        let cond = TriggerCondition::MessageContains {
            value: "deploy".to_string(),
        };
        assert!(cond.matches(&test_event("ci", "Starting deploy to production")));
        assert!(cond.matches(&test_event("ci", "DEPLOY complete")));
        assert!(!cond.matches(&test_event("ci", "Build succeeded")));
    }

    #[test]
    fn test_message_starts_with() {
        let cond = TriggerCondition::MessageStartsWith {
            value: "/alert".to_string(),
        };
        assert!(cond.matches(&test_event("monitor", "/alert CPU high")));
        assert!(!cond.matches(&test_event("monitor", "CPU /alert high")));
    }

    #[test]
    fn test_metadata_field_exists() {
        let cond = TriggerCondition::MetadataField {
            key: "priority".to_string(),
            value: None,
        };
        let event = test_event_with_metadata("ci", "test", json!({"priority": "high"}));
        assert!(cond.matches(&event));

        let event_no_meta = test_event("ci", "test");
        assert!(!cond.matches(&event_no_meta));
    }

    #[test]
    fn test_metadata_field_equals() {
        let cond = TriggerCondition::MetadataField {
            key: "priority".to_string(),
            value: Some("high".to_string()),
        };
        let event = test_event_with_metadata("ci", "test", json!({"priority": "high"}));
        assert!(cond.matches(&event));

        let event_low = test_event_with_metadata("ci", "test", json!({"priority": "low"}));
        assert!(!cond.matches(&event_low));
    }

    #[test]
    fn test_all_condition() {
        let cond = TriggerCondition::All {
            conditions: vec![
                TriggerCondition::SourceEquals {
                    value: "github".to_string(),
                },
                TriggerCondition::MessageContains {
                    value: "push".to_string(),
                },
            ],
        };
        assert!(cond.matches(&test_event("github", "push to main")));
        assert!(!cond.matches(&test_event("github", "PR opened")));
        assert!(!cond.matches(&test_event("gitlab", "push to main")));
    }

    #[test]
    fn test_any_condition() {
        let cond = TriggerCondition::Any {
            conditions: vec![
                TriggerCondition::SourceEquals {
                    value: "github".to_string(),
                },
                TriggerCondition::SourceEquals {
                    value: "gitlab".to_string(),
                },
            ],
        };
        assert!(cond.matches(&test_event("github", "event")));
        assert!(cond.matches(&test_event("gitlab", "event")));
        assert!(!cond.matches(&test_event("bitbucket", "event")));
    }

    #[test]
    fn test_always_condition() {
        let cond = TriggerCondition::Always;
        assert!(cond.matches(&test_event("any", "anything")));
    }

    #[test]
    fn test_source_equals_none_source() {
        let cond = TriggerCondition::SourceEquals {
            value: "github".to_string(),
        };
        let event = WebhookEvent::new(None, "msg".to_string(), None, None, None);
        assert!(!cond.matches(&event));
    }

    // -- Action dispatch tests --

    #[test]
    fn test_dispatch_route_to_agent() {
        let trigger = WebhookTrigger {
            id: "t1".to_string(),
            name: "Route CI".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::RouteToAgent {
                agent_id: "agent-1".to_string(),
                prompt_template: None,
            },
            enabled: true,
            priority: 100,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "build done");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        assert_eq!(result.action_type, "route_to_agent");
        assert!(result.output.unwrap().contains("agent-1"));
    }

    #[test]
    fn test_dispatch_route_with_template() {
        let trigger = WebhookTrigger {
            id: "t2".to_string(),
            name: "Template".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::RouteToAgent {
                agent_id: "a1".to_string(),
                prompt_template: Some("Handle this: {message}".to_string()),
            },
            enabled: true,
            priority: 50,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "deploy failed");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(
            result
                .output
                .unwrap()
                .contains("Handle this: deploy failed")
        );
    }

    #[test]
    fn test_dispatch_execute_tool() {
        let trigger = WebhookTrigger {
            id: "t3".to_string(),
            name: "Run tool".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::ExecuteTool {
                tool_name: "shell".to_string(),
                arguments: Some(json!({"cwd": "/tmp"})),
            },
            enabled: true,
            priority: 100,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "ls -la");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        assert_eq!(result.action_type, "execute_tool");
        let output = result.output.unwrap();
        assert!(output.contains("shell"));
        assert!(output.contains("ls -la"));
    }

    #[test]
    fn test_dispatch_forward_to_channel() {
        let trigger = WebhookTrigger {
            id: "t4".to_string(),
            name: "Forward".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::ForwardToChannel {
                channel_type: "telegram".to_string(),
                chat_id: "-100123".to_string(),
                prefix: Some("[ALERT]".to_string()),
            },
            enabled: true,
            priority: 100,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("monitor", "CPU 95%");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        let output = result.output.unwrap();
        assert!(output.contains("[ALERT] CPU 95%"));
        assert!(output.contains("telegram"));
    }

    #[test]
    fn test_dispatch_call_webhook() {
        let mut headers = HashMap::new();
        headers.insert("X-Token".to_string(), "secret".to_string());
        let trigger = WebhookTrigger {
            id: "t5".to_string(),
            name: "Webhook".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::CallWebhook {
                url: "https://example.com/hook".to_string(),
                headers,
            },
            enabled: true,
            priority: 100,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "done");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        assert_eq!(result.action_type, "call_webhook");
    }

    #[test]
    fn test_dispatch_log() {
        let trigger = WebhookTrigger {
            id: "t6".to_string(),
            name: "Logger".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::Log {
                label: Some("audit".to_string()),
            },
            enabled: true,
            priority: 200,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "test");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        assert_eq!(result.action_type, "log");
    }

    #[test]
    fn test_dispatch_chain() {
        let trigger = WebhookTrigger {
            id: "t7".to_string(),
            name: "Chain".to_string(),
            description: None,
            condition: TriggerCondition::Always,
            action: TriggerAction::Chain {
                actions: vec![
                    TriggerAction::Log { label: None },
                    TriggerAction::RouteToAgent {
                        agent_id: "a1".to_string(),
                        prompt_template: None,
                    },
                ],
            },
            enabled: true,
            priority: 100,
            created_at: Utc::now(),
            fire_count: 0,
            last_fired_at: None,
        };
        let event = test_event("ci", "chain test");
        let result = TriggerEngine::dispatch_action(&trigger, &event);
        assert!(result.success);
        assert_eq!(result.action_type, "chain");
        let output = result.output.unwrap();
        assert!(output.contains("\"action_count\":2"));
    }

    // -- TriggerEngine tests --

    #[tokio::test]
    async fn test_engine_create_and_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let trigger = engine
            .create(
                "Test Trigger".to_string(),
                Some("A test".to_string()),
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await
            .unwrap();

        assert!(!trigger.id.is_empty());
        assert_eq!(trigger.name, "Test Trigger");
        assert!(trigger.enabled);
        assert_eq!(trigger.priority, 100);

        let list = engine.list().await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_engine_create_requires_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let result = engine
            .create(
                String::new(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_engine_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let trigger = engine
            .create(
                "Delete me".to_string(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await
            .unwrap();

        engine.delete(&trigger.id).await.unwrap();
        assert!(engine.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_engine_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());
        assert!(engine.delete("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_engine_set_enabled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let trigger = engine
            .create(
                "Toggle".to_string(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await
            .unwrap();

        engine.set_enabled(&trigger.id, false).await.unwrap();
        let t = engine.get(&trigger.id).await.unwrap();
        assert!(!t.enabled);

        engine.set_enabled(&trigger.id, true).await.unwrap();
        let t = engine.get(&trigger.id).await.unwrap();
        assert!(t.enabled);
    }

    #[tokio::test]
    async fn test_engine_evaluate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        engine
            .create(
                "GitHub only".to_string(),
                None,
                TriggerCondition::SourceEquals {
                    value: "github".to_string(),
                },
                TriggerAction::Log { label: None },
                Some(10),
            )
            .await
            .unwrap();

        engine
            .create(
                "Catch-all".to_string(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                Some(200),
            )
            .await
            .unwrap();

        // GitHub event: both match
        let github_event = test_event("github", "push");
        let matched = engine.evaluate(&github_event).await;
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].name, "GitHub only"); // lower priority = first

        // GitLab event: only catch-all
        let gitlab_event = test_event("gitlab", "push");
        let matched = engine.evaluate(&gitlab_event).await;
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "Catch-all");
    }

    #[tokio::test]
    async fn test_engine_evaluate_skips_disabled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let trigger = engine
            .create(
                "Disabled".to_string(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await
            .unwrap();

        engine.set_enabled(&trigger.id, false).await.unwrap();

        let event = test_event("any", "any");
        let matched = engine.evaluate(&event).await;
        assert!(matched.is_empty());
    }

    #[tokio::test]
    async fn test_engine_record_fire() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = TriggerEngine::new(tmp.path());

        let trigger = engine
            .create(
                "Counter".to_string(),
                None,
                TriggerCondition::Always,
                TriggerAction::Log { label: None },
                None,
            )
            .await
            .unwrap();

        assert_eq!(trigger.fire_count, 0);
        assert!(trigger.last_fired_at.is_none());

        engine.record_fire(&trigger.id).await;
        engine.record_fire(&trigger.id).await;

        let t = engine.get(&trigger.id).await.unwrap();
        assert_eq!(t.fire_count, 2);
        assert!(t.last_fired_at.is_some());
    }

    #[tokio::test]
    async fn test_engine_persistence() {
        let tmp = tempfile::TempDir::new().unwrap();

        {
            let engine = TriggerEngine::new(tmp.path());
            engine
                .create(
                    "Persist me".to_string(),
                    None,
                    TriggerCondition::Always,
                    TriggerAction::Log { label: None },
                    None,
                )
                .await
                .unwrap();
        }

        // Reload
        let engine2 = TriggerEngine::new(tmp.path());
        let list = engine2.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Persist me");
    }

    // -- Serialization tests --

    #[test]
    fn test_trigger_condition_serialization() {
        let cond = TriggerCondition::All {
            conditions: vec![
                TriggerCondition::SourceEquals {
                    value: "github".to_string(),
                },
                TriggerCondition::MessageContains {
                    value: "deploy".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&cond).unwrap();
        let back: TriggerCondition = serde_json::from_str(&json).unwrap();
        let event = test_event("github", "deploy to prod");
        assert!(back.matches(&event));
    }

    #[test]
    fn test_trigger_action_serialization() {
        let action = TriggerAction::RouteToAgent {
            agent_id: "agent-1".to_string(),
            prompt_template: Some("Process: {message}".to_string()),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("route_to_agent"));
        let back: TriggerAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, TriggerAction::RouteToAgent { .. }));
    }

    #[test]
    fn test_webhook_event_serialization() {
        let event = test_event("github", "push event");
        let json = serde_json::to_string(&event).unwrap();
        let back: WebhookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message, "push event");
        assert_eq!(back.source.as_deref(), Some("github"));
    }
}
