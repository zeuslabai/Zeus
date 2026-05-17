//! Inter-agent HTTP protocol types for the Agora.

use serde::{Deserialize, Serialize};

/// Identity of an agent participating in the Agora.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentIdentity {
    /// Unique agent identifier (e.g. "agent-abc123")
    pub agent_id: String,
    /// Human-readable display name
    pub display_name: String,
    /// Base URL where the agent's HTTP API is reachable
    pub endpoint_url: String,
    /// Public key for request signing (hex-encoded)
    pub public_key: Option<String>,
    /// Protocol version this agent supports
    pub protocol_version: String,
}

/// A capability an agent advertises to the Agora.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCapability {
    /// Skill name this capability maps to
    pub skill_name: String,
    /// Semantic version of the skill implementation
    pub version: String,
    /// Input JSON schema (as string)
    pub input_schema: String,
    /// Output JSON schema (as string)
    pub output_schema: String,
    /// Maximum concurrent invocations this agent can handle
    pub max_concurrency: u32,
    /// Whether the agent requires credits for this skill
    pub requires_credits: bool,
    /// Tags for discovery (e.g. ["code", "analysis", "rust"])
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_identity_serde() {
        let identity = AgentIdentity {
            agent_id: "agent-001".into(),
            display_name: "Coder Agent".into(),
            endpoint_url: "http://localhost:9000".into(),
            public_key: Some("deadbeef".into()),
            protocol_version: "1.0".into(),
        };
        let json = serde_json::to_string(&identity).expect("should serialize to JSON");
        let parsed: AgentIdentity = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(identity, parsed);
    }

    #[test]
    fn test_agent_identity_no_public_key() {
        let identity = AgentIdentity {
            agent_id: "agent-002".into(),
            display_name: "Helper".into(),
            endpoint_url: "http://localhost:9001".into(),
            public_key: None,
            protocol_version: "1.0".into(),
        };
        let json = serde_json::to_string(&identity).expect("should serialize to JSON");
        assert!(json.contains("\"public_key\":null"));
        let parsed: AgentIdentity = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.public_key, None);
    }

    #[test]
    fn test_agent_capability_serde() {
        let cap = AgentCapability {
            skill_name: "code_review".into(),
            version: "1.2.0".into(),
            input_schema: r#"{"type":"object"}"#.into(),
            output_schema: r#"{"type":"string"}"#.into(),
            max_concurrency: 5,
            requires_credits: true,
            tags: vec!["code".into(), "review".into()],
        };
        let json = serde_json::to_string(&cap).expect("should serialize to JSON");
        let parsed: AgentCapability =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(cap, parsed);
        assert_eq!(parsed.tags.len(), 2);
    }

    #[test]
    fn test_capability_no_credits() {
        let cap = AgentCapability {
            skill_name: "ping".into(),
            version: "0.1.0".into(),
            input_schema: "{}".into(),
            output_schema: "{}".into(),
            max_concurrency: 100,
            requires_credits: false,
            tags: vec![],
        };
        assert!(!cap.requires_credits);
        assert!(cap.tags.is_empty());
    }
}
