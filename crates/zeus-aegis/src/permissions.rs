//! Permission management

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Permission set for controlling allowed operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionSet {
    /// Allowed operations (supports wildcards)
    allowed: HashSet<String>,
    /// Explicitly denied operations
    denied: HashSet<String>,
}

impl PermissionSet {
    /// Create a new permission set
    pub fn new(allowed: Vec<String>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
            denied: HashSet::new(),
        }
    }

    /// Create an empty (deny-all) permission set
    pub fn empty() -> Self {
        Self {
            allowed: HashSet::new(),
            denied: HashSet::new(),
        }
    }

    /// Create an allow-all permission set
    pub fn allow_all() -> Self {
        let mut allowed = HashSet::new();
        allowed.insert("*".to_string());
        Self {
            allowed,
            denied: HashSet::new(),
        }
    }

    /// Add an allowed operation
    pub fn allow(&mut self, operation: impl Into<String>) {
        self.allowed.insert(operation.into());
    }

    /// Add a denied operation
    pub fn deny(&mut self, operation: impl Into<String>) {
        self.denied.insert(operation.into());
    }

    /// Check if an operation is allowed
    pub fn is_allowed(&self, operation: &str) -> bool {
        // Explicit deny takes precedence
        if self.denied.contains(operation) {
            return false;
        }

        // Check for deny wildcards
        for denied in &self.denied {
            if Self::matches_pattern(denied, operation) {
                return false;
            }
        }

        // Check explicit allow
        if self.allowed.contains(operation) {
            return true;
        }

        // Check allow wildcards
        for allowed in &self.allowed {
            if Self::matches_pattern(allowed, operation) {
                return true;
            }
        }

        false
    }

    /// Check if a pattern matches an operation
    fn matches_pattern(pattern: &str, operation: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if let Some(prefix) = pattern.strip_suffix(".*") {
            // Require the dot separator: "fs.*" should match "fs.read" but not "fssomething"
            let prefix_with_dot = format!("{}.", prefix);
            return operation.starts_with(&prefix_with_dot);
        }

        if let Some(prefix) = pattern.strip_suffix('*') {
            return operation.starts_with(prefix);
        }

        pattern == operation
    }
}

impl Default for PermissionSet {
    fn default() -> Self {
        Self::allow_all()
    }
}

/// Operation categories
pub mod operations {
    // Filesystem operations
    pub const FS_READ: &str = "fs.read";
    pub const FS_WRITE: &str = "fs.write";
    pub const FS_DELETE: &str = "fs.delete";
    pub const FS_EXECUTE: &str = "fs.execute";

    // Network operations
    pub const NET_HTTP: &str = "net.http";
    pub const NET_SOCKET: &str = "net.socket";
    pub const NET_DNS: &str = "net.dns";

    // System operations
    pub const SYS_PROCESS: &str = "sys.process";
    pub const SYS_ENV: &str = "sys.env";
    pub const SYS_SHELL: &str = "sys.shell";

    // Tool operations
    pub const TOOL_MCP: &str = "tool.mcp";
    pub const TOOL_SKILL: &str = "tool.skill";

    // Channel operations
    pub const CHANNEL_SEND: &str = "channel.send";
    pub const CHANNEL_RECEIVE: &str = "channel.receive";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_allow() {
        let mut perms = PermissionSet::empty();
        perms.allow("fs.read");

        assert!(perms.is_allowed("fs.read"));
        assert!(!perms.is_allowed("fs.write"));
    }

    #[test]
    fn test_wildcard_allow() {
        let mut perms = PermissionSet::empty();
        perms.allow("fs.*");

        assert!(perms.is_allowed("fs.read"));
        assert!(perms.is_allowed("fs.write"));
        assert!(!perms.is_allowed("net.http"));
    }

    #[test]
    fn test_wildcard_requires_dot_separator() {
        let mut perms = PermissionSet::empty();
        perms.allow("fs.*");

        // Should match with dot separator
        assert!(perms.is_allowed("fs.read"));
        assert!(perms.is_allowed("fs.write"));
        // Should NOT match without dot separator
        assert!(!perms.is_allowed("fssomething"));
        assert!(!perms.is_allowed("fsread"));
    }

    #[test]
    fn test_deny_precedence() {
        let mut perms = PermissionSet::allow_all();
        perms.deny("fs.delete");

        assert!(perms.is_allowed("fs.read"));
        assert!(!perms.is_allowed("fs.delete"));
    }

    #[test]
    fn test_allow_all() {
        let perms = PermissionSet::allow_all();
        assert!(perms.is_allowed("anything"));
    }
}
