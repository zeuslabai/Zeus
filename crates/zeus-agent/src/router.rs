//! Multi-agent routing configuration and dispatcher.
//!
//! Routes incoming messages to the appropriate agent profile based on
//! channel type, enabling different models, workspaces, and settings
//! per communication channel.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a named agent profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Optional model override (e.g., "anthropic/claude-haiku-4-5-20251001")
    pub model: Option<String>,
    /// Optional workspace path override
    pub workspace: Option<std::path::PathBuf>,
    /// Optional max iterations override
    pub max_iterations: Option<usize>,
    /// Optional system prompt addition
    pub system_prompt_extra: Option<String>,
}

/// Multi-agent routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoutingConfig {
    /// Named agent profiles
    pub agents: HashMap<String, AgentProfile>,
    /// Channel type -> agent name routing
    /// e.g., "telegram" -> "fast_agent", "email" -> "deep_agent"
    pub routing: HashMap<String, String>,
    /// Default agent name when no routing match
    #[serde(default = "default_agent")]
    pub default_agent: String,
}

fn default_agent() -> String {
    "default".to_string()
}

impl Default for AgentRoutingConfig {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            routing: HashMap::new(),
            default_agent: default_agent(),
        }
    }
}

/// Routes incoming messages to the appropriate agent profile
pub struct AgentRouter {
    config: AgentRoutingConfig,
}

impl AgentRouter {
    pub fn new(config: AgentRoutingConfig) -> Self {
        Self { config }
    }

    /// Get the agent profile name for a given channel type
    pub fn route(&self, channel_type: &str) -> &str {
        self.config
            .routing
            .get(channel_type)
            .map(|s| s.as_str())
            .unwrap_or(&self.config.default_agent)
    }

    /// Get the agent profile for a given channel type
    pub fn get_profile(&self, channel_type: &str) -> Option<&AgentProfile> {
        let agent_name = self.route(channel_type);
        self.config.agents.get(agent_name)
    }

    /// Get all configured agent names
    pub fn agent_names(&self) -> Vec<&String> {
        self.config.agents.keys().collect()
    }

    /// Check if a specific agent profile exists
    pub fn has_agent(&self, name: &str) -> bool {
        self.config.agents.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AgentRoutingConfig {
        let mut agents = HashMap::new();
        agents.insert(
            "fast_agent".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-haiku-4-5-20251001".to_string()),
                workspace: None,
                max_iterations: Some(5),
                system_prompt_extra: Some("Be concise.".to_string()),
            },
        );
        agents.insert(
            "deep_agent".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
                workspace: Some(std::path::PathBuf::from("/tmp/deep-workspace")),
                max_iterations: Some(20),
                system_prompt_extra: None,
            },
        );
        agents.insert(
            "default".to_string(),
            AgentProfile {
                model: None,
                workspace: None,
                max_iterations: None,
                system_prompt_extra: None,
            },
        );

        let mut routing = HashMap::new();
        routing.insert("telegram".to_string(), "fast_agent".to_string());
        routing.insert("email".to_string(), "deep_agent".to_string());

        AgentRoutingConfig {
            agents,
            routing,
            default_agent: "default".to_string(),
        }
    }

    #[test]
    fn test_route_by_channel_type() {
        let router = AgentRouter::new(sample_config());
        assert_eq!(router.route("telegram"), "fast_agent");
        assert_eq!(router.route("email"), "deep_agent");
    }

    #[test]
    fn test_route_default_fallback() {
        let router = AgentRouter::new(sample_config());
        assert_eq!(router.route("discord"), "default");
        assert_eq!(router.route("unknown_channel"), "default");
    }

    #[test]
    fn test_get_profile_returns_correct_profile() {
        let router = AgentRouter::new(sample_config());

        let profile = router.get_profile("telegram").unwrap();
        assert_eq!(
            profile.model.as_deref(),
            Some("anthropic/claude-haiku-4-5-20251001")
        );
        assert_eq!(profile.max_iterations, Some(5));
        assert_eq!(profile.system_prompt_extra.as_deref(), Some("Be concise."));

        let profile = router.get_profile("email").unwrap();
        assert_eq!(
            profile.model.as_deref(),
            Some("anthropic/claude-sonnet-4-20250514")
        );
        assert_eq!(profile.max_iterations, Some(20));
        assert_eq!(
            profile.workspace,
            Some(std::path::PathBuf::from("/tmp/deep-workspace"))
        );
    }

    #[test]
    fn test_get_profile_unknown_channel_returns_default() {
        let router = AgentRouter::new(sample_config());

        let profile = router.get_profile("discord").unwrap();
        assert!(profile.model.is_none());
        assert!(profile.workspace.is_none());
        assert!(profile.max_iterations.is_none());
    }

    #[test]
    fn test_agent_names_lists_all() {
        let router = AgentRouter::new(sample_config());
        let mut names: Vec<&str> = router.agent_names().iter().map(|s| s.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["deep_agent", "default", "fast_agent"]);
    }

    #[test]
    fn test_has_agent() {
        let router = AgentRouter::new(sample_config());
        assert!(router.has_agent("fast_agent"));
        assert!(router.has_agent("deep_agent"));
        assert!(router.has_agent("default"));
        assert!(!router.has_agent("nonexistent"));
    }

    #[test]
    fn test_routing_config_serde_roundtrip() {
        let config = sample_config();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentRoutingConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.default_agent, config.default_agent);
        assert_eq!(deserialized.routing.len(), config.routing.len());
        assert_eq!(deserialized.agents.len(), config.agents.len());

        // Verify specific routing entries survived the roundtrip
        assert_eq!(
            deserialized.routing.get("telegram").map(|s| s.as_str()),
            Some("fast_agent")
        );
        assert_eq!(
            deserialized.routing.get("email").map(|s| s.as_str()),
            Some("deep_agent")
        );

        // Verify a profile survived the roundtrip
        let fast = deserialized.agents.get("fast_agent").unwrap();
        assert_eq!(
            fast.model.as_deref(),
            Some("anthropic/claude-haiku-4-5-20251001")
        );
        assert_eq!(fast.max_iterations, Some(5));
    }

    #[test]
    fn test_routing_config_default() {
        let config = AgentRoutingConfig::default();
        assert_eq!(config.default_agent, "default");
        assert!(config.agents.is_empty());
        assert!(config.routing.is_empty());
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_routing_config_with_multiple_channels() {
        let mut agents = HashMap::new();
        agents.insert(
            "fast".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-haiku-4-5-20251001".to_string()),
                workspace: None,
                max_iterations: Some(5),
                system_prompt_extra: None,
            },
        );
        agents.insert(
            "medium".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
                workspace: None,
                max_iterations: Some(15),
                system_prompt_extra: None,
            },
        );
        agents.insert(
            "deep".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-opus-4-20250514".to_string()),
                workspace: Some(std::path::PathBuf::from("/tmp/deep")),
                max_iterations: Some(30),
                system_prompt_extra: Some("Be thorough.".to_string()),
            },
        );

        let mut routing = HashMap::new();
        routing.insert("telegram".to_string(), "fast".to_string());
        routing.insert("discord".to_string(), "medium".to_string());
        routing.insert("email".to_string(), "deep".to_string());

        let config = AgentRoutingConfig {
            agents,
            routing,
            default_agent: "medium".to_string(),
        };

        let router = AgentRouter::new(config);

        assert_eq!(router.route("telegram"), "fast");
        assert_eq!(router.route("discord"), "medium");
        assert_eq!(router.route("email"), "deep");
        assert_eq!(router.route("slack"), "medium"); // falls back to default
    }

    #[test]
    fn test_route_with_channel_override() {
        let mut agents = HashMap::new();
        agents.insert(
            "default".to_string(),
            AgentProfile {
                model: Some("openai/gpt-4o".to_string()),
                workspace: None,
                max_iterations: Some(10),
                system_prompt_extra: None,
            },
        );
        agents.insert(
            "telegram_agent".to_string(),
            AgentProfile {
                model: Some("anthropic/claude-haiku-4-5-20251001".to_string()),
                workspace: None,
                max_iterations: Some(3),
                system_prompt_extra: Some("Be brief for Telegram.".to_string()),
            },
        );

        let mut routing = HashMap::new();
        routing.insert("telegram".to_string(), "telegram_agent".to_string());

        let config = AgentRoutingConfig {
            agents,
            routing,
            default_agent: "default".to_string(),
        };

        let router = AgentRouter::new(config);

        // Telegram should get its own profile
        let tg_profile = router.get_profile("telegram").unwrap();
        assert_eq!(tg_profile.max_iterations, Some(3));
        assert_eq!(
            tg_profile.system_prompt_extra.as_deref(),
            Some("Be brief for Telegram.")
        );

        // Other channels get the default profile
        let default_profile = router.get_profile("slack").unwrap();
        assert_eq!(default_profile.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(default_profile.max_iterations, Some(10));
    }

    #[test]
    fn test_agent_names_empty() {
        let config = AgentRoutingConfig::default();
        let router = AgentRouter::new(config);
        let names = router.agent_names();
        assert!(names.is_empty());
    }

    #[test]
    fn test_routing_config_debug() {
        let config = sample_config();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("AgentRoutingConfig"));
        assert!(debug_str.contains("fast_agent"));
        assert!(debug_str.contains("deep_agent"));
        assert!(debug_str.contains("default"));
        assert!(debug_str.contains("telegram"));
        assert!(debug_str.contains("email"));
    }
}
