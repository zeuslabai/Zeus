//! Channel access policy enforcement
//!
//! Enforces DM, group, and tool access policies based on the
//! `ChannelPolicyConfig` defined in zeus-core.

use zeus_core::{ChannelPolicyConfig, DmPolicy, GroupPolicy};

/// Result of a policy check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyResult {
    /// Access allowed
    Allowed,
    /// Access denied with reason
    Denied(String),
}

impl PolicyResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyResult::Allowed)
    }

    pub fn is_denied(&self) -> bool {
        !self.is_allowed()
    }

    /// Get the denial reason, if any
    pub fn reason(&self) -> Option<&str> {
        match self {
            PolicyResult::Allowed => None,
            PolicyResult::Denied(reason) => Some(reason),
        }
    }
}

/// Channel policy enforcer
#[derive(Default, Clone)]
pub struct ChannelPolicy {
    config: ChannelPolicyConfig,
}

impl ChannelPolicy {
    pub fn new(config: ChannelPolicyConfig) -> Self {
        Self { config }
    }

    /// Check if a DM from the given sender is allowed
    pub fn check_dm(&self, sender_id: &str) -> PolicyResult {
        match self.config.dm {
            DmPolicy::Open => PolicyResult::Allowed,
            DmPolicy::Allowlist => {
                if self.config.allow_from.iter().any(|id| id == sender_id) {
                    PolicyResult::Allowed
                } else {
                    PolicyResult::Denied(format!("Sender '{}' not in DM allowlist", sender_id))
                }
            }
            DmPolicy::Pairing => {
                // Pairing mode - always deny here, caller should check pairing manager
                PolicyResult::Denied("Pairing required".to_string())
            }
            DmPolicy::Disabled => PolicyResult::Denied("DMs are disabled".to_string()),
        }
    }

    /// Check if a group message is allowed
    pub fn check_group(&self, group_id: &str, _sender_id: &str, is_mention: bool) -> PolicyResult {
        match self.config.group {
            GroupPolicy::Open => PolicyResult::Allowed,
            GroupPolicy::Allowlist => {
                if self.config.allow_groups.iter().any(|id| id == group_id) {
                    PolicyResult::Allowed
                } else {
                    PolicyResult::Denied(format!("Group '{}' not in allowlist", group_id))
                }
            }
            GroupPolicy::MentionOnly => {
                if is_mention {
                    PolicyResult::Allowed
                } else {
                    PolicyResult::Denied("Only responding to mentions in groups".to_string())
                }
            }
            GroupPolicy::Disabled => {
                PolicyResult::Denied("Group messages are disabled".to_string())
            }
        }
    }

    /// Check if a tool is allowed from this channel
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match &self.config.tools_allowlist {
            Some(allowlist) => allowlist.iter().any(|t| t == tool_name || t == "*"),
            None => true, // No allowlist means all tools allowed
        }
    }

    /// Check if a command is allowed from this channel
    pub fn is_command_allowed(&self, command_name: &str) -> bool {
        match &self.config.commands_allowlist {
            Some(allowlist) => allowlist.iter().any(|c| c == command_name || c == "*"),
            None => true, // No allowlist means all commands allowed
        }
    }

    /// Get a reference to the underlying config
    pub fn config(&self) -> &ChannelPolicyConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dm_open_allows_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            dm: DmPolicy::Open,
            ..Default::default()
        });

        assert!(policy.check_dm("anyone").is_allowed());
        assert!(policy.check_dm("stranger").is_allowed());
        assert!(policy.check_dm("").is_allowed());
    }

    #[test]
    fn test_dm_allowlist_accept() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            dm: DmPolicy::Allowlist,
            allow_from: vec!["alice".to_string(), "bob".to_string()],
            ..Default::default()
        });

        assert!(policy.check_dm("alice").is_allowed());
        assert!(policy.check_dm("bob").is_allowed());
    }

    #[test]
    fn test_dm_allowlist_reject() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            dm: DmPolicy::Allowlist,
            allow_from: vec!["alice".to_string(), "bob".to_string()],
            ..Default::default()
        });

        let result = policy.check_dm("charlie");
        assert!(result.is_denied());
        assert!(
            result
                .reason()
                .expect("reason should succeed")
                .contains("charlie")
        );
        assert!(
            result
                .reason()
                .expect("reason should succeed")
                .contains("allowlist")
        );
    }

    #[test]
    fn test_dm_disabled_rejects_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            dm: DmPolicy::Disabled,
            ..Default::default()
        });

        let result = policy.check_dm("anyone");
        assert!(result.is_denied());
        assert_eq!(
            result.reason().expect("reason should succeed"),
            "DMs are disabled"
        );
    }

    #[test]
    fn test_dm_pairing_requires_pairing() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            dm: DmPolicy::Pairing,
            ..Default::default()
        });

        let result = policy.check_dm("anyone");
        assert!(result.is_denied());
        assert!(
            result
                .reason()
                .expect("reason should succeed")
                .contains("Pairing")
        );
    }

    #[test]
    fn test_group_open_allows_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            group: GroupPolicy::Open,
            ..Default::default()
        });

        assert!(policy.check_group("group1", "user1", false).is_allowed());
        assert!(policy.check_group("group2", "user2", true).is_allowed());
    }

    #[test]
    fn test_group_mention_only_allows_mentions() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            group: GroupPolicy::MentionOnly,
            ..Default::default()
        });

        // Mention should be allowed
        assert!(policy.check_group("group1", "user1", true).is_allowed());

        // Non-mention should be denied
        let result = policy.check_group("group1", "user1", false);
        assert!(result.is_denied());
        assert!(
            result
                .reason()
                .expect("reason should succeed")
                .contains("mentions")
        );
    }

    #[test]
    fn test_group_allowlist_accept() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            group: GroupPolicy::Allowlist,
            allow_groups: vec!["group-a".to_string(), "group-b".to_string()],
            ..Default::default()
        });

        assert!(policy.check_group("group-a", "user1", false).is_allowed());
        assert!(policy.check_group("group-b", "user2", true).is_allowed());
    }

    #[test]
    fn test_group_allowlist_reject() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            group: GroupPolicy::Allowlist,
            allow_groups: vec!["group-a".to_string()],
            ..Default::default()
        });

        let result = policy.check_group("group-c", "user1", false);
        assert!(result.is_denied());
        assert!(
            result
                .reason()
                .expect("reason should succeed")
                .contains("group-c")
        );
    }

    #[test]
    fn test_group_disabled_rejects_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            group: GroupPolicy::Disabled,
            ..Default::default()
        });

        let result = policy.check_group("any", "user", true);
        assert!(result.is_denied());
        assert_eq!(
            result.reason().expect("reason should succeed"),
            "Group messages are disabled"
        );
    }

    #[test]
    fn test_tool_filtering_with_allowlist() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            tools_allowlist: Some(vec!["read_file".to_string(), "list_dir".to_string()]),
            ..Default::default()
        });

        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("list_dir"));
        assert!(!policy.is_tool_allowed("shell"));
        assert!(!policy.is_tool_allowed("write_file"));
    }

    #[test]
    fn test_tool_filtering_wildcard() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            tools_allowlist: Some(vec!["*".to_string()]),
            ..Default::default()
        });

        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("shell"));
        assert!(policy.is_tool_allowed("anything"));
    }

    #[test]
    fn test_tool_filtering_no_allowlist_allows_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            tools_allowlist: None,
            ..Default::default()
        });

        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("shell"));
        assert!(policy.is_tool_allowed("anything"));
    }

    #[test]
    fn test_tool_filtering_empty_allowlist_denies_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            tools_allowlist: Some(vec![]),
            ..Default::default()
        });

        assert!(!policy.is_tool_allowed("read_file"));
        assert!(!policy.is_tool_allowed("shell"));
    }

    #[test]
    fn test_default_policy() {
        let policy = ChannelPolicy::default();

        // Default DmPolicy is Open
        assert!(policy.check_dm("anyone").is_allowed());

        // Default GroupPolicy is Open (changed in S43 for fleet-wide allow_bots)
        assert!(policy.check_group("g", "u", true).is_allowed());
        assert!(policy.check_group("g", "u", false).is_allowed());

        // Default tools_allowlist is None => all allowed
        assert!(policy.is_tool_allowed("anything"));

        // Default commands_allowlist is None => all allowed
        assert!(policy.is_command_allowed("anything"));
    }

    // ====================================================================
    // Command allowlist tests
    // ====================================================================

    #[test]
    fn test_command_no_allowlist_allows_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            commands_allowlist: None,
            ..Default::default()
        });

        assert!(policy.is_command_allowed("status"));
        assert!(policy.is_command_allowed("memory"));
        assert!(policy.is_command_allowed("config"));
        assert!(policy.is_command_allowed("anything"));
    }

    #[test]
    fn test_command_allowlist_filters() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            commands_allowlist: Some(vec!["status".to_string(), "memory".to_string()]),
            ..Default::default()
        });

        assert!(policy.is_command_allowed("status"));
        assert!(policy.is_command_allowed("memory"));
        assert!(!policy.is_command_allowed("config"));
        assert!(!policy.is_command_allowed("shell"));
    }

    #[test]
    fn test_command_allowlist_wildcard() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            commands_allowlist: Some(vec!["*".to_string()]),
            ..Default::default()
        });

        assert!(policy.is_command_allowed("status"));
        assert!(policy.is_command_allowed("memory"));
        assert!(policy.is_command_allowed("config"));
        assert!(policy.is_command_allowed("anything"));
    }

    #[test]
    fn test_command_empty_allowlist_denies_all() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            commands_allowlist: Some(vec![]),
            ..Default::default()
        });

        assert!(!policy.is_command_allowed("status"));
        assert!(!policy.is_command_allowed("memory"));
        assert!(!policy.is_command_allowed("anything"));
    }

    #[test]
    fn test_command_allowlist_serde_roundtrip() {
        let config = ChannelPolicyConfig {
            commands_allowlist: Some(vec!["status".to_string(), "memory".to_string()]),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: ChannelPolicyConfig =
            serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(
            deserialized.commands_allowlist,
            Some(vec!["status".to_string(), "memory".to_string()])
        );
    }

    #[test]
    fn test_command_allowlist_backward_compat() {
        // Old JSON without commands_allowlist field should deserialize fine
        let json = r#"{"dm":"open","group":"mentiononly","allow_from":[],"allow_groups":[],"tools_allowlist":null}"#;
        let config: ChannelPolicyConfig =
            serde_json::from_str(json).expect("should parse successfully");

        assert!(config.commands_allowlist.is_none());

        // Policy should allow all commands when field is absent
        let policy = ChannelPolicy::new(config);
        assert!(policy.is_command_allowed("status"));
        assert!(policy.is_command_allowed("anything"));
    }

    #[test]
    fn test_policy_combined_tool_and_command_filtering() {
        let policy = ChannelPolicy::new(ChannelPolicyConfig {
            tools_allowlist: Some(vec!["read_file".to_string(), "list_dir".to_string()]),
            commands_allowlist: Some(vec!["status".to_string(), "memory".to_string()]),
            ..Default::default()
        });

        // Tool filtering works independently
        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("list_dir"));
        assert!(!policy.is_tool_allowed("shell"));
        assert!(!policy.is_tool_allowed("write_file"));

        // Command filtering works independently
        assert!(policy.is_command_allowed("status"));
        assert!(policy.is_command_allowed("memory"));
        assert!(!policy.is_command_allowed("config"));
        assert!(!policy.is_command_allowed("shell"));

        // Cross-check: a tool name is not a command and vice versa
        assert!(!policy.is_tool_allowed("status")); // "status" not in tools_allowlist
        assert!(!policy.is_command_allowed("read_file")); // "read_file" not in commands_allowlist
    }
}
