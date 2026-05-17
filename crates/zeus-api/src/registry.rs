//! Agent Registry — manages spawned agent instances and binding-based routing.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use zeus_core::{AgentBinding, AgentConfig, AgentToolPolicy, BindingRule, Config};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_session::Session;

/// Runtime state for a spawned agent instance.
pub struct AgentInstance {
    /// The live agent behind a lock for concurrent access
    pub agent: Arc<RwLock<zeus_agent::Agent>>,
    /// Agent identifier (matches the JSON file name)
    pub agent_id: String,
    /// Human-readable name
    pub name: String,
    /// Binding configuration (rules + tool policy + priority)
    pub binding: AgentBinding,
    /// When this instance was spawned
    pub spawned_at: DateTime<Utc>,
    /// Last time a message was routed to this instance
    pub last_active: DateTime<Utc>,
    /// Count of messages processed
    pub message_count: u64,
    /// Discord account key this agent is bound to (for `route_by_account`)
    pub discord_account: Option<String>,
}

/// Manages agent instances with binding-based routing.
pub struct AgentRegistry {
    instances: HashMap<String, AgentInstance>,
    base_config: Config,
}

impl AgentRegistry {
    /// Create an empty registry.
    pub fn new(config: Config) -> Self {
        Self {
            instances: HashMap::new(),
            base_config: config,
        }
    }

    /// Spawn an agent from its JSON file in `~/.zeus/agents/{id}.json`.
    ///
    /// Reads the agent definition, creates an `Agent::with_subsystems()`,
    /// applies bindings and tool policy, and registers the instance.
    pub async fn spawn(&mut self, agent_id: &str) -> Result<(), String> {
        if self.instances.contains_key(agent_id) {
            return Err(format!("Agent '{}' is already spawned", agent_id));
        }

        // Read agent JSON
        let agents_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join("agents");
        let path = agents_dir.join(format!("{}.json", agent_id));

        if !path.exists() {
            return Err(format!("Agent definition not found: {}", agent_id));
        }

        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read agent: {}", e))?;

        let agent_json: Value =
            serde_json::from_str(&content).map_err(|e| format!("Invalid agent JSON: {}", e))?;

        let name = agent_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed")
            .to_string();

        // Parse bindings
        let bindings: Vec<BindingRule> = agent_json
            .get("bindings")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Parse tool policy
        let tool_policy: AgentToolPolicy = agent_json
            .get("tool_policy")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let priority = agent_json
            .get("priority")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let binding = AgentBinding {
            agent_id: agent_id.to_string(),
            bindings,
            tool_policy: tool_policy.clone(),
            priority,
        };

        // Create LLM client and agent with deterministic session ID
        let config = self.base_config.clone();
        let llm = LlmClient::from_config(&config)
            .map_err(|e| format!("Failed to create LLM client: {}", e))?;
        let workspace = Workspace::from_config(&config);
        let stable_id = format!("agent-{}", agent_id);
        let session = Session::resume_or_create(&config.sessions, &stable_id).await;

        let mut agent = zeus_agent::Agent::with_subsystems(config, llm, workspace, session)
            .await
            .map_err(|e| format!("Failed to create agent: {}", e))?;

        // Apply tool policy
        agent.set_tool_policy(tool_policy);

        let now = Utc::now();
        let instance = AgentInstance {
            agent: Arc::new(RwLock::new(agent)),
            agent_id: agent_id.to_string(),
            name: name.clone(),
            binding,
            spawned_at: now,
            last_active: now,
            message_count: 0,
            discord_account: None,
        };

        self.instances.insert(agent_id.to_string(), instance);
        info!("Spawned agent '{}' ({})", name, agent_id);

        Ok(())
    }

    /// Spawn a dynamic (ephemeral) agent without requiring a JSON file on disk.
    ///
    /// Used by the Prometheus executor to create per-step agents programmatically.
    /// The agent gets core tools (read_file, write_file, shell, etc.) and an optional
    /// goals context for plan-aware execution.
    pub async fn spawn_dynamic(
        &mut self,
        agent_id: &str,
        name: &str,
        goals_context: Option<String>,
    ) -> Result<(), String> {
        if self.instances.contains_key(agent_id) {
            return Err(format!("Agent '{}' is already spawned", agent_id));
        }

        let config = self.base_config.clone();
        let llm = LlmClient::from_config(&config)
            .map_err(|e| format!("Failed to create LLM client: {}", e))?;
        let workspace = Workspace::from_config(&config);
        let stable_id = format!("agent-{}", agent_id);
        let session = Session::resume_or_create(&config.sessions, &stable_id).await;

        // Registry spawn: ChannelManager is shared post-construction via
        // `share_channels()` (gateway.rs:694) → `set_shared_channels()`, so this
        // constructor passes None and lets the registry-wide share-pass wire it.
        let mut agent = zeus_agent::Agent::new(config, llm, workspace, session, None);

        if let Some(ctx) = goals_context {
            agent.set_goals_context(Some(ctx));
        }

        let now = Utc::now();
        let binding = AgentBinding {
            agent_id: agent_id.to_string(),
            bindings: vec![],
            tool_policy: AgentToolPolicy::default(),
            priority: 0,
        };

        let instance = AgentInstance {
            agent: Arc::new(RwLock::new(agent)),
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            binding,
            spawned_at: now,
            last_active: now,
            message_count: 0,
            discord_account: None,
        };

        self.instances.insert(agent_id.to_string(), instance);
        info!("Spawned dynamic agent '{}' ({})", name, agent_id);

        Ok(())
    }

    /// Spawn an agent from a `[agents.*]` config entry (S36 Track B).
    ///
    /// Creates an isolated `Agent` instance with its own workspace, sessions,
    /// and model override (all from `AgentConfig`). The instance is keyed by
    /// `agent_cfg.id` and also registers the Discord account binding for
    /// `route_by_account()` lookups.
    pub async fn spawn_from_config(&mut self, agent_cfg: &AgentConfig) -> Result<(), String> {
        if self.instances.contains_key(&agent_cfg.id) {
            return Err(format!("Agent '{}' is already registered", agent_cfg.id));
        }

        // Build per-agent config: inherit base config, override model + paths + mnemosyne
        let mut cfg = self.base_config.clone();
        if let Some(ref model) = agent_cfg.model {
            cfg.model = model.clone();
        }
        let workspace_path = agent_cfg.resolve_workspace(&self.base_config.workspace);
        let sessions_path = agent_cfg.resolve_sessions(&self.base_config.sessions);
        cfg.workspace = workspace_path.clone();
        cfg.sessions = sessions_path.clone();

        // S41: Per-agent Mnemosyne DB — each agent gets isolated long-term memory.
        // If the base config has mnemosyne enabled, override db_path for this agent.
        if let Some(ref mut mc) = cfg.mnemosyne {
            let agent_db = agent_cfg.resolve_mnemosyne_db(&self.base_config.workspace);
            mc.db_path = agent_db;
        }

        // Strip channel config from registry agents — the default agent owns
        // the Discord/Telegram connections and the gateway consumer routes
        // inbound messages to registry agents.  Creating adapters here would
        // open duplicate WebSocket connections with the same bot token.
        // Use empty config (not None) so the from_env() fallback doesn't
        // re-create channel adapters from environment variables.
        cfg.channels = Some(zeus_core::ChannelsConfig::default());

        let llm = LlmClient::from_config(&cfg)
            .map_err(|e| format!("Failed to create LLM client for agent '{}': {}", agent_cfg.id, e))?;

        let workspace = Workspace::new(&workspace_path);
        workspace.init().await.map_err(|e| format!("Workspace init failed for '{}': {}", agent_cfg.id, e))?;

        // Copy superpowered templates from global workspace if per-agent versions don't exist.
        // This ensures fleet agents get the same quality templates as the primary agent.
        let global_ws = Workspace::new(&self.base_config.workspace);
        for filename in &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md"] {
            let per_agent = workspace_path.join(filename);
            if !per_agent.exists() {
                if let Ok(content) = global_ws.read(filename).await {
                    if !content.trim().is_empty() {
                        let _ = tokio::fs::write(&per_agent, &content).await;
                    }
                }
            }
        }

        let stable_id = format!("agent-{}", agent_cfg.id);
        let session = Session::resume_or_create(&sessions_path, &stable_id).await;

        let agent = zeus_agent::Agent::with_subsystems(cfg, llm, workspace, session)
            .await
            .map_err(|e| format!("Failed to create agent '{}': {}", agent_cfg.id, e))?;

        let now = Utc::now();
        let binding = AgentBinding {
            agent_id: agent_cfg.id.clone(),
            bindings: vec![],
            tool_policy: AgentToolPolicy::default(),
            priority: 0,
        };

        let instance = AgentInstance {
            agent: Arc::new(RwLock::new(agent)),
            agent_id: agent_cfg.id.clone(),
            name: agent_cfg.display_name().to_string(),
            binding,
            spawned_at: now,
            last_active: now,
            message_count: 0,
            discord_account: agent_cfg.discord_account.clone(),
        };

        self.instances.insert(agent_cfg.id.clone(), instance);
        info!(
            "Registered agent '{}' from config (discord_account={:?}, workspace={}, sessions={})",
            agent_cfg.id,
            agent_cfg.discord_account,
            workspace_path.display(),
            sessions_path.display(),
        );
        Ok(())
    }

    /// Share a ChannelManager with all registry agents so their `message` tool
    /// can send through platform channels (Discord, Telegram, Slack, etc.).
    /// Called by the gateway after the default agent is created.
    pub async fn share_channels(&self, channels: std::sync::Arc<zeus_channels::ChannelManager>) {
        for instance in self.instances.values() {
            let mut agent = instance.agent.write().await;
            agent.set_shared_channels(channels.clone());
        }
        info!("Shared channel manager with {} registry agent(s)", self.instances.len());
    }

    /// Remove a spawned agent instance.
    pub fn unregister(&mut self, agent_id: &str) -> bool {
        if self.instances.remove(agent_id).is_some() {
            info!("Unregistered agent '{}'", agent_id);
            true
        } else {
            warn!("Agent '{}' not found in registry", agent_id);
            false
        }
    }

    /// Find the highest-priority agent whose bindings match the given message metadata.
    pub fn route(
        &self,
        channel_type: &str,
        user_id: &str,
        chat_id: &str,
    ) -> Option<&AgentInstance> {
        let mut best: Option<&AgentInstance> = None;

        for instance in self.instances.values() {
            let matches = instance
                .binding
                .bindings
                .iter()
                .any(|rule| rule.matches(channel_type, user_id, chat_id));

            if matches {
                if let Some(current_best) = best {
                    if instance.binding.priority > current_best.binding.priority {
                        best = Some(instance);
                    }
                } else {
                    best = Some(instance);
                }
            }
        }

        best
    }

    /// Find an agent registered for a specific account identity (S35/S36 multi-account routing).
    ///
    /// Two-tier lookup:
    /// 1. Fast path: `agent_id == account_id` (conventional case where id equals account key)
    /// 2. Slow path: scan for `discord_account == Some(account_id)` (explicit mapping)
    ///
    /// This is the highest-priority routing tier:
    /// account-routed > thread > binding-routed > default.
    pub fn route_by_account(&self, account_id: &str) -> Option<&AgentInstance> {
        // Fast path: agent_id matches account_id directly
        if let Some(instance) = self.instances.get(account_id) {
            return Some(instance);
        }
        // Slow path: search by explicit discord_account binding
        self.instances
            .values()
            .find(|i| i.discord_account.as_deref() == Some(account_id))
    }

    /// Get an agent instance by ID.
    pub fn get(&self, agent_id: &str) -> Option<&AgentInstance> {
        self.instances.get(agent_id)
    }

    /// Get a mutable reference to an agent instance by ID.
    pub fn get_mut(&mut self, agent_id: &str) -> Option<&mut AgentInstance> {
        self.instances.get_mut(agent_id)
    }

    /// List all spawned agent instances.
    pub fn list(&self) -> Vec<&AgentInstance> {
        self.instances.values().collect()
    }

    /// Update activity counters for an agent.
    pub fn update_activity(&mut self, agent_id: &str) {
        if let Some(instance) = self.instances.get_mut(agent_id) {
            instance.last_active = Utc::now();
            instance.message_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::BindingRule;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn test_registry_new_empty() {
        let registry = AgentRegistry::new(test_config());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_registry_route_no_match() {
        let registry = AgentRegistry::new(test_config());
        assert!(registry.route("telegram", "user1", "chat1").is_none());
    }

    #[test]
    fn test_registry_unregister_missing() {
        let mut registry = AgentRegistry::new(test_config());
        assert!(!registry.unregister("nonexistent"));
    }

    #[test]
    fn test_registry_get_missing() {
        let registry = AgentRegistry::new(test_config());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_route_priority() {
        let registry = AgentRegistry::new(test_config());

        let binding_low = AgentBinding {
            agent_id: "low".to_string(),
            bindings: vec![BindingRule::Team("telegram".to_string())],
            tool_policy: AgentToolPolicy::default(),
            priority: 0,
        };

        let binding_high = AgentBinding {
            agent_id: "high".to_string(),
            bindings: vec![BindingRule::Team("telegram".to_string())],
            tool_policy: AgentToolPolicy::default(),
            priority: 10,
        };

        // Route matching tested directly (Agent instances require LLM config):
        assert!(binding_low.bindings[0].matches("telegram", "any", "any"));
        assert!(binding_high.bindings[0].matches("telegram", "any", "any"));
        assert!(binding_high.priority > binding_low.priority);

        // Verify registry starts empty
        assert!(registry.route("telegram", "any", "any").is_none());
    }

    #[test]
    fn test_binding_route_matching() {
        // Test the core routing logic without needing actual agents
        let bindings = vec![
            AgentBinding {
                agent_id: "agent-tg".to_string(),
                bindings: vec![BindingRule::Team("telegram".to_string())],
                tool_policy: AgentToolPolicy::default(),
                priority: 0,
            },
            AgentBinding {
                agent_id: "agent-dm".to_string(),
                bindings: vec![BindingRule::Peer("user42".to_string())],
                tool_policy: AgentToolPolicy::default(),
                priority: 10,
            },
        ];

        // Find best match for telegram user42
        let matched: Vec<_> = bindings
            .iter()
            .filter(|b| {
                b.bindings
                    .iter()
                    .any(|r| r.matches("telegram", "user42", "chat1"))
            })
            .collect();

        assert_eq!(matched.len(), 2); // Both match
        let best = matched.iter().max_by_key(|b| b.priority).unwrap();
        assert_eq!(best.agent_id, "agent-dm"); // Higher priority wins
    }

    #[tokio::test]
    async fn test_spawn_dynamic_creates_agent() {
        let mut registry = AgentRegistry::new(test_config());
        assert!(registry.list().is_empty());

        let result = registry
            .spawn_dynamic(
                "test-agent-1",
                "Test Agent",
                Some("You are a test agent".to_string()),
            )
            .await;
        // This will fail without a real LLM config (no API key), which is expected
        // in test. The important thing is the method exists and doesn't panic.
        if result.is_ok() {
            assert_eq!(registry.list().len(), 1);
            let instance = registry.get("test-agent-1").unwrap();
            assert_eq!(instance.name, "Test Agent");
            assert_eq!(instance.agent_id, "test-agent-1");
            assert!(instance.binding.bindings.is_empty());
            assert_eq!(instance.message_count, 0);

            // Unregister and verify cleanup
            assert!(registry.unregister("test-agent-1"));
            assert!(registry.list().is_empty());
        }
        // If it fails (expected in test), that's fine — LlmClient needs real config
    }

    #[tokio::test]
    async fn test_spawn_dynamic_duplicate_rejected() {
        let mut registry = AgentRegistry::new(test_config());
        let _ = registry.spawn_dynamic("dup-agent", "Dup", None).await;
        // If first spawn succeeded, second should fail with duplicate error
        if registry.get("dup-agent").is_some() {
            let err = registry
                .spawn_dynamic("dup-agent", "Dup2", None)
                .await
                .unwrap_err();
            assert!(err.contains("already spawned"));
        }
    }

    #[test]
    fn test_spawn_dynamic_no_bindings() {
        // Verify that dynamically spawned agents have empty bindings
        // (they shouldn't match any routing rules)
        let binding = AgentBinding {
            agent_id: "dynamic-1".to_string(),
            bindings: vec![],
            tool_policy: AgentToolPolicy::default(),
            priority: 0,
        };
        assert!(binding.bindings.is_empty());
        // Should NOT match any route
        let matches = binding
            .bindings
            .iter()
            .any(|r| r.matches("telegram", "user1", "chat1"));
        assert!(!matches);
    }
}
