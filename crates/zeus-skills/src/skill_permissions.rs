//! Per-Skill Permission Sandbox — capability-based access control for skills.
//!
//! Each skill declares required permissions in its SKILL.md. The permission
//! sandbox enforces these at runtime:
//! - Skills can only use tools they've been granted access to
//! - File access is scoped to the skill's own directory + allowed paths
//! - Network access can be restricted to specific domains
//! - Shell execution can be fully disabled or restricted to allowlisted commands
//!
//! Integration with zeus-aegis: SkillPermissionPolicy maps to Aegis PermissionSet
//! for unified security enforcement.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Capability that a skill can request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCapability {
    /// Read files (optionally scoped to paths).
    FileRead,
    /// Write files (optionally scoped to paths).
    FileWrite,
    /// Execute shell commands.
    Shell,
    /// Make network requests (optionally scoped to domains).
    Network,
    /// Access environment variables.
    Environment,
    /// Spawn subprocesses.
    Process,
    /// Access other skills' tools.
    InterSkill,
    /// Full system access (dangerous — requires explicit approval).
    FullAccess,
}

impl std::fmt::Display for SkillCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileRead => write!(f, "file_read"),
            Self::FileWrite => write!(f, "file_write"),
            Self::Shell => write!(f, "shell"),
            Self::Network => write!(f, "network"),
            Self::Environment => write!(f, "environment"),
            Self::Process => write!(f, "process"),
            Self::InterSkill => write!(f, "inter_skill"),
            Self::FullAccess => write!(f, "full_access"),
        }
    }
}

/// Permission policy for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPermissionPolicy {
    /// Skill name this policy applies to.
    pub skill_name: String,
    /// Granted capabilities.
    pub capabilities: HashSet<SkillCapability>,
    /// Allowed file paths (glob patterns). Empty = skill directory only.
    pub allowed_paths: Vec<String>,
    /// Allowed network domains. Empty = no network (unless Network capability granted with "*").
    pub allowed_domains: Vec<String>,
    /// Allowed shell commands. Empty = no shell (unless Shell capability granted with "*").
    pub allowed_commands: Vec<String>,
    /// Maximum execution time in seconds (0 = unlimited).
    pub max_execution_secs: u64,
    /// Whether this policy was explicitly approved by the user.
    pub user_approved: bool,
    /// Trust level: 0 = untrusted, 1 = basic, 2 = trusted, 3 = full access.
    pub trust_level: u8,
}

impl SkillPermissionPolicy {
    /// Create a minimal (restrictive) policy for a new skill.
    pub fn minimal(skill_name: &str) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            capabilities: HashSet::new(),
            allowed_paths: vec![],
            allowed_domains: vec![],
            allowed_commands: vec![],
            max_execution_secs: 30,
            user_approved: false,
            trust_level: 0,
        }
    }

    /// Create a basic policy with file read + network.
    pub fn basic(skill_name: &str) -> Self {
        let mut caps = HashSet::new();
        caps.insert(SkillCapability::FileRead);
        caps.insert(SkillCapability::Network);
        Self {
            skill_name: skill_name.to_string(),
            capabilities: caps,
            allowed_paths: vec![],
            allowed_domains: vec!["*".to_string()],
            allowed_commands: vec![],
            max_execution_secs: 60,
            user_approved: false,
            trust_level: 1,
        }
    }

    /// Create a trusted policy with most capabilities.
    pub fn trusted(skill_name: &str) -> Self {
        let mut caps = HashSet::new();
        caps.insert(SkillCapability::FileRead);
        caps.insert(SkillCapability::FileWrite);
        caps.insert(SkillCapability::Shell);
        caps.insert(SkillCapability::Network);
        caps.insert(SkillCapability::Environment);
        caps.insert(SkillCapability::Process);
        Self {
            skill_name: skill_name.to_string(),
            capabilities: caps,
            allowed_paths: vec!["*".to_string()],
            allowed_domains: vec!["*".to_string()],
            allowed_commands: vec!["*".to_string()],
            max_execution_secs: 300,
            user_approved: true,
            trust_level: 2,
        }
    }

    /// Create a restricted policy for marketplace/remote skills.
    /// Read-only filesystem, no shell, scoped network, short timeout.
    pub fn marketplace_restricted(skill_name: &str) -> Self {
        let mut caps = HashSet::new();
        caps.insert(SkillCapability::FileRead);
        caps.insert(SkillCapability::Network);
        Self {
            skill_name: skill_name.to_string(),
            capabilities: caps,
            allowed_paths: vec![], // skill directory only
            allowed_domains: vec![], // no network by default — must be explicitly approved
            allowed_commands: vec![], // no shell
            max_execution_secs: 30,
            user_approved: false,
            trust_level: 0,
        }
    }

    /// Create a policy based on skill source.
    /// - Builtin: trusted (full access, pre-approved)
    /// - Local: basic (read + network, user can escalate)
    /// - Remote/ClawHub: marketplace_restricted (read-only, no shell, short timeout)
    pub fn for_source(skill_name: &str, source: &str) -> Self {
        match source {
            "builtin" => Self::trusted(skill_name),
            "local" => Self::basic(skill_name),
            _ => Self::marketplace_restricted(skill_name), // clawhub, remote, unknown
        }
    }

    /// Check if a specific capability is granted.
    pub fn has_capability(&self, cap: &SkillCapability) -> bool {
        self.capabilities.contains(&SkillCapability::FullAccess) || self.capabilities.contains(cap)
    }

    /// Check if a file path is allowed for this skill.
    pub fn is_path_allowed(&self, path: &Path, skill_dir: &Path) -> bool {
        // Full access bypasses checks
        if self.capabilities.contains(&SkillCapability::FullAccess) {
            return true;
        }

        // Always allow access to the skill's own directory
        if path.starts_with(skill_dir) {
            return true;
        }

        // Check against allowed paths
        let path_str = path.to_string_lossy();
        for pattern in &self.allowed_paths {
            if pattern == "*" {
                return true;
            }
            if path_str.starts_with(pattern.trim_end_matches('*')) {
                return true;
            }
        }

        false
    }

    /// Check if a network domain is allowed.
    pub fn is_domain_allowed(&self, domain: &str) -> bool {
        if self.capabilities.contains(&SkillCapability::FullAccess) {
            return true;
        }
        if !self.has_capability(&SkillCapability::Network) {
            return false;
        }
        for allowed in &self.allowed_domains {
            if allowed == "*" || allowed == domain {
                return true;
            }
            // Subdomain matching: ".example.com" matches "api.example.com"
            if allowed.starts_with('.') && domain.ends_with(allowed.as_str()) {
                return true;
            }
        }
        false
    }

    /// Check if a shell command is allowed.
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if self.capabilities.contains(&SkillCapability::FullAccess) {
            return true;
        }
        if !self.has_capability(&SkillCapability::Shell) {
            return false;
        }
        for allowed in &self.allowed_commands {
            if allowed == "*" {
                return true;
            }
            // Match command prefix (e.g., "git" allows "git status", "git commit", etc.)
            if command == *allowed || command.starts_with(&format!("{allowed} ")) {
                return true;
            }
        }
        false
    }
}

/// Result of a permission check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionCheckResult {
    pub allowed: bool,
    pub skill_name: String,
    pub action: String,
    pub reason: String,
}

/// Parse capabilities from SKILL.md permission declarations.
pub fn parse_skill_capabilities(permissions: &[String]) -> HashSet<SkillCapability> {
    let mut caps = HashSet::new();
    for perm in permissions {
        match perm.to_lowercase().as_str() {
            "file_read" | "read" | "fs_read" => {
                caps.insert(SkillCapability::FileRead);
            }
            "file_write" | "write" | "fs_write" => {
                caps.insert(SkillCapability::FileWrite);
            }
            "shell" | "exec" | "execute" => {
                caps.insert(SkillCapability::Shell);
            }
            "network" | "net" | "http" | "fetch" => {
                caps.insert(SkillCapability::Network);
            }
            "env" | "environment" => {
                caps.insert(SkillCapability::Environment);
            }
            "process" | "spawn" => {
                caps.insert(SkillCapability::Process);
            }
            "inter_skill" | "skills" => {
                caps.insert(SkillCapability::InterSkill);
            }
            "full" | "full_access" | "all" => {
                caps.insert(SkillCapability::FullAccess);
            }
            _ => {} // Unknown permissions are silently ignored
        }
    }
    caps
}

/// Registry that manages permission policies for all installed skills.
#[derive(Clone)]
pub struct SkillPermissionRegistry {
    policies: Arc<RwLock<HashMap<String, SkillPermissionPolicy>>>,
    /// Path for persisting policies to disk.
    persist_path: Option<PathBuf>,
}

impl SkillPermissionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            persist_path: None,
        }
    }

    /// Create with a persistence path.
    pub fn with_persistence(path: PathBuf) -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            persist_path: Some(path),
        }
    }

    /// Register a policy for a skill.
    pub async fn register(&self, policy: SkillPermissionPolicy) {
        let name = policy.skill_name.clone();
        self.policies.write().await.insert(name, policy);
        self.persist().await;
    }

    /// Remove a skill's policy.
    pub async fn remove(&self, skill_name: &str) -> bool {
        let removed = self.policies.write().await.remove(skill_name).is_some();
        if removed {
            self.persist().await;
        }
        removed
    }

    /// Get a skill's policy.
    pub async fn get(&self, skill_name: &str) -> Option<SkillPermissionPolicy> {
        self.policies.read().await.get(skill_name).cloned()
    }

    /// Check if a skill has a specific capability.
    pub async fn check_capability(
        &self,
        skill_name: &str,
        capability: &SkillCapability,
    ) -> PermissionCheckResult {
        let policies = self.policies.read().await;
        match policies.get(skill_name) {
            Some(policy) => {
                let allowed = policy.has_capability(capability);
                PermissionCheckResult {
                    allowed,
                    skill_name: skill_name.to_string(),
                    action: format!("capability:{capability}"),
                    reason: if allowed {
                        "granted".to_string()
                    } else {
                        format!("skill '{skill_name}' does not have {capability} capability")
                    },
                }
            }
            None => PermissionCheckResult {
                allowed: false,
                skill_name: skill_name.to_string(),
                action: format!("capability:{capability}"),
                reason: format!("no policy registered for skill '{skill_name}'"),
            },
        }
    }

    /// Check if a skill can access a file path.
    pub async fn check_path(
        &self,
        skill_name: &str,
        path: &Path,
        skill_dir: &Path,
    ) -> PermissionCheckResult {
        let policies = self.policies.read().await;
        match policies.get(skill_name) {
            Some(policy) => {
                let allowed = policy.is_path_allowed(path, skill_dir);
                PermissionCheckResult {
                    allowed,
                    skill_name: skill_name.to_string(),
                    action: format!("path:{}", path.display()),
                    reason: if allowed {
                        "path allowed".to_string()
                    } else {
                        format!(
                            "path '{}' not in allowed list for skill '{skill_name}'",
                            path.display()
                        )
                    },
                }
            }
            None => PermissionCheckResult {
                allowed: false,
                skill_name: skill_name.to_string(),
                action: format!("path:{}", path.display()),
                reason: format!("no policy registered for skill '{skill_name}'"),
            },
        }
    }

    /// List all registered policies.
    pub async fn list(&self) -> Vec<SkillPermissionPolicy> {
        self.policies.read().await.values().cloned().collect()
    }

    /// Total number of registered policies.
    pub async fn count(&self) -> usize {
        self.policies.read().await.len()
    }

    /// Persist policies to disk (if persist_path is set).
    async fn persist(&self) {
        if let Some(ref path) = self.persist_path {
            let policies = self.policies.read().await;
            let data: Vec<&SkillPermissionPolicy> = policies.values().collect();
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// Load policies from disk.
    pub async fn load(&self) -> usize {
        if let Some(ref path) = self.persist_path
            && path.exists()
            && let Ok(content) = std::fs::read_to_string(path)
            && let Ok(policies) = serde_json::from_str::<Vec<SkillPermissionPolicy>>(&content)
        {
            let count = policies.len();
            let mut map = self.policies.write().await;
            for policy in policies {
                map.insert(policy.skill_name.clone(), policy);
            }
            return count;
        }
        0
    }
}

/// Upsert a single policy into the on-disk `permissions.json` at `path`,
/// synchronously. Reads the existing `Vec<SkillPermissionPolicy>` (if any),
/// replaces the entry with the same `skill_name`, and writes it back in the
/// same format `SkillPermissionRegistry::load`/`persist` round-trips.
///
/// This is the persistence seam shared by both the async (`install`) and the
/// sync (`install_local`) ingestion sites in `ClawHubClient`, so the recorded
/// policy lands durably regardless of the caller's async-ness. The path is
/// `skills_dir.join("permissions.json")` — configurable, never hardcoded.
pub fn upsert_policy_file(path: &Path, policy: &SkillPermissionPolicy) -> std::io::Result<()> {
    let mut policies: Vec<SkillPermissionPolicy> = if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Upsert by skill_name — one row per skill, no near-dup accumulation.
    if let Some(existing) = policies
        .iter_mut()
        .find(|p| p.skill_name == policy.skill_name)
    {
        *existing = policy.clone();
    } else {
        policies.push(policy.clone());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&policies)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// Map a recorded skill `trust_level` (0=untrusted .. 3=full) to the aegis
/// [`SandboxLevel`] used at execution time. Higher trust → looser sandbox.
///
/// Sandbox-by-default: any unrecognized trust value (including the absence of a
/// policy, handled at the call site) maps to the most-restrictive `Paranoid` —
/// mirroring Cut-1's "no allowed-tools → minimal" posture.
pub fn trust_level_to_sandbox(trust_level: u8) -> zeus_aegis::SandboxLevel {
    use zeus_aegis::SandboxLevel;
    match trust_level {
        // Full trust → no per-command sandbox wrap.
        3 => SandboxLevel::None,
        // Source-trusted (e.g. for_source) → standard filesystem restriction.
        2 => SandboxLevel::Standard,
        1 => SandboxLevel::Strict,
        // Untrusted (0) or anything unrecognized → most-restrictive.
        _ => SandboxLevel::Paranoid,
    }
}

impl Default for SkillPermissionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_policy_denies_everything() {
        let policy = SkillPermissionPolicy::minimal("test-skill");
        assert!(!policy.has_capability(&SkillCapability::FileRead));
        assert!(!policy.has_capability(&SkillCapability::Shell));
        assert!(!policy.has_capability(&SkillCapability::Network));
        assert_eq!(policy.trust_level, 0);
    }

    #[test]
    fn test_basic_policy() {
        let policy = SkillPermissionPolicy::basic("test-skill");
        assert!(policy.has_capability(&SkillCapability::FileRead));
        assert!(policy.has_capability(&SkillCapability::Network));
        assert!(!policy.has_capability(&SkillCapability::Shell));
        assert!(!policy.has_capability(&SkillCapability::FileWrite));
        assert_eq!(policy.trust_level, 1);
    }

    #[test]
    fn test_trusted_policy() {
        let policy = SkillPermissionPolicy::trusted("test-skill");
        assert!(policy.has_capability(&SkillCapability::FileRead));
        assert!(policy.has_capability(&SkillCapability::FileWrite));
        assert!(policy.has_capability(&SkillCapability::Shell));
        assert!(policy.has_capability(&SkillCapability::Network));
        assert!(policy.user_approved);
        assert_eq!(policy.trust_level, 2);
    }

    #[test]
    fn test_full_access_bypasses_all() {
        let mut policy = SkillPermissionPolicy::minimal("test");
        policy.capabilities.insert(SkillCapability::FullAccess);
        assert!(policy.has_capability(&SkillCapability::Shell));
        assert!(policy.has_capability(&SkillCapability::Network));
        assert!(policy.is_path_allowed(Path::new("/etc/passwd"), Path::new("/tmp/skills/test")));
        assert!(policy.is_domain_allowed("evil.com"));
        assert!(policy.is_command_allowed("rm -rf /"));
    }

    #[test]
    fn test_path_allows_skill_dir() {
        let policy = SkillPermissionPolicy::minimal("test");
        let skill_dir = Path::new("/home/user/.zeus/skills/test");
        // Always allowed: files within the skill's own directory
        assert!(policy.is_path_allowed(
            Path::new("/home/user/.zeus/skills/test/data.json"),
            skill_dir
        ));
        // Denied: files outside skill dir
        assert!(!policy.is_path_allowed(Path::new("/home/user/.ssh/id_rsa"), skill_dir));
    }

    #[test]
    fn test_path_allowed_patterns() {
        let mut policy = SkillPermissionPolicy::minimal("test");
        policy.capabilities.insert(SkillCapability::FileRead);
        policy.allowed_paths = vec!["/tmp/*".to_string(), "/home/user/docs".to_string()];
        let skill_dir = Path::new("/skills/test");

        assert!(policy.is_path_allowed(Path::new("/tmp/data.txt"), skill_dir));
        assert!(policy.is_path_allowed(Path::new("/home/user/docs/file.md"), skill_dir));
        assert!(!policy.is_path_allowed(Path::new("/etc/passwd"), skill_dir));
    }

    #[test]
    fn test_domain_allowed() {
        let mut policy = SkillPermissionPolicy::minimal("test");
        policy.capabilities.insert(SkillCapability::Network);
        policy.allowed_domains = vec!["api.github.com".to_string(), ".example.com".to_string()];

        assert!(policy.is_domain_allowed("api.github.com"));
        assert!(policy.is_domain_allowed("sub.example.com"));
        assert!(!policy.is_domain_allowed("evil.com"));
    }

    #[test]
    fn test_domain_denied_without_capability() {
        let mut policy = SkillPermissionPolicy::minimal("test");
        policy.allowed_domains = vec!["*".to_string()];
        // No Network capability = denied even with wildcard domain
        assert!(!policy.is_domain_allowed("anything.com"));
    }

    #[test]
    fn test_command_allowed() {
        let mut policy = SkillPermissionPolicy::minimal("test");
        policy.capabilities.insert(SkillCapability::Shell);
        policy.allowed_commands = vec!["git".to_string(), "cargo test".to_string()];

        assert!(policy.is_command_allowed("git"));
        assert!(policy.is_command_allowed("git status"));
        assert!(policy.is_command_allowed("git commit -m 'test'"));
        assert!(policy.is_command_allowed("cargo test"));
        assert!(!policy.is_command_allowed("rm -rf /"));
        assert!(!policy.is_command_allowed("cargo build"));
    }

    #[test]
    fn test_parse_capabilities() {
        let perms = vec![
            "read".to_string(),
            "network".to_string(),
            "shell".to_string(),
            "unknown_thing".to_string(),
        ];
        let caps = parse_skill_capabilities(&perms);
        assert!(caps.contains(&SkillCapability::FileRead));
        assert!(caps.contains(&SkillCapability::Network));
        assert!(caps.contains(&SkillCapability::Shell));
        assert_eq!(caps.len(), 3); // unknown ignored
    }

    #[tokio::test]
    async fn test_registry_register_and_get() {
        let registry = SkillPermissionRegistry::new();
        let policy = SkillPermissionPolicy::basic("my-skill");
        registry.register(policy).await;

        let retrieved = registry.get("my-skill").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().trust_level, 1);
        assert_eq!(registry.count().await, 1);
    }

    #[tokio::test]
    async fn test_registry_remove() {
        let registry = SkillPermissionRegistry::new();
        registry.register(SkillPermissionPolicy::basic("a")).await;
        registry.register(SkillPermissionPolicy::basic("b")).await;
        assert_eq!(registry.count().await, 2);

        let removed = registry.remove("a").await;
        assert!(removed);
        assert_eq!(registry.count().await, 1);
        assert!(registry.get("a").await.is_none());
    }

    #[tokio::test]
    async fn test_registry_check_capability() {
        let registry = SkillPermissionRegistry::new();
        registry
            .register(SkillPermissionPolicy::basic("reader"))
            .await;

        let result = registry
            .check_capability("reader", &SkillCapability::FileRead)
            .await;
        assert!(result.allowed);

        let result = registry
            .check_capability("reader", &SkillCapability::Shell)
            .await;
        assert!(!result.allowed);

        let result = registry
            .check_capability("unknown", &SkillCapability::FileRead)
            .await;
        assert!(!result.allowed);
        assert!(result.reason.contains("no policy registered"));
    }

    #[tokio::test]
    async fn test_registry_check_path() {
        let registry = SkillPermissionRegistry::new();
        registry
            .register(SkillPermissionPolicy::basic("test"))
            .await;

        let skill_dir = Path::new("/skills/test");
        let result = registry
            .check_path("test", Path::new("/skills/test/data.json"), skill_dir)
            .await;
        assert!(result.allowed);

        let result = registry
            .check_path("test", Path::new("/etc/shadow"), skill_dir)
            .await;
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn test_registry_list() {
        let registry = SkillPermissionRegistry::new();
        registry.register(SkillPermissionPolicy::minimal("a")).await;
        registry.register(SkillPermissionPolicy::basic("b")).await;
        registry.register(SkillPermissionPolicy::trusted("c")).await;

        let list = registry.list().await;
        assert_eq!(list.len(), 3);
    }

    #[tokio::test]
    async fn test_registry_persistence() {
        let tmp = std::env::temp_dir().join("zeus_test_perm_registry.json");
        let _ = std::fs::remove_file(&tmp);

        // Write
        let registry = SkillPermissionRegistry::with_persistence(tmp.clone());
        registry
            .register(SkillPermissionPolicy::basic("persisted"))
            .await;
        assert!(tmp.exists());

        // Read in a new registry
        let registry2 = SkillPermissionRegistry::with_persistence(tmp.clone());
        let loaded = registry2.load().await;
        assert_eq!(loaded, 1);
        let policy = registry2.get("persisted").await;
        assert!(policy.is_some());
        assert_eq!(policy.unwrap().trust_level, 1);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_capability_display() {
        assert_eq!(format!("{}", SkillCapability::FileRead), "file_read");
        assert_eq!(format!("{}", SkillCapability::Shell), "shell");
        assert_eq!(format!("{}", SkillCapability::FullAccess), "full_access");
    }

    #[test]
    fn test_marketplace_restricted_policy() {
        let policy = SkillPermissionPolicy::marketplace_restricted("remote-skill");
        assert!(policy.has_capability(&SkillCapability::FileRead));
        assert!(policy.has_capability(&SkillCapability::Network));
        assert!(!policy.has_capability(&SkillCapability::Shell));
        assert!(!policy.has_capability(&SkillCapability::FileWrite));
        assert!(!policy.has_capability(&SkillCapability::Process));
        assert_eq!(policy.trust_level, 0);
        assert_eq!(policy.max_execution_secs, 30);
        assert!(policy.allowed_domains.is_empty()); // no network by default
        assert!(!policy.user_approved);
    }

    #[test]
    fn test_for_source_builtin() {
        let policy = SkillPermissionPolicy::for_source("git", "builtin");
        assert_eq!(policy.trust_level, 2);
        assert!(policy.has_capability(&SkillCapability::Shell));
        assert!(policy.user_approved);
    }

    #[test]
    fn test_for_source_local() {
        let policy = SkillPermissionPolicy::for_source("my-tool", "local");
        assert_eq!(policy.trust_level, 1);
        assert!(policy.has_capability(&SkillCapability::FileRead));
        assert!(!policy.has_capability(&SkillCapability::Shell));
    }

    #[test]
    fn test_for_source_clawhub() {
        let policy = SkillPermissionPolicy::for_source("untrusted", "clawhub");
        assert_eq!(policy.trust_level, 0);
        assert!(!policy.has_capability(&SkillCapability::Shell));
        assert!(!policy.has_capability(&SkillCapability::FileWrite));
        assert_eq!(policy.max_execution_secs, 30);
    }

    #[test]
    fn test_for_source_unknown_defaults_restricted() {
        let policy = SkillPermissionPolicy::for_source("mystery", "unknown_registry");
        assert_eq!(policy.trust_level, 0);
        assert!(!policy.has_capability(&SkillCapability::Shell));
    }

    // --- GAP#3 Cut-2: persistence + trust→sandbox mapping ---

    #[test]
    fn test_trust_level_to_sandbox_mapping() {
        use zeus_aegis::SandboxLevel;
        // Higher trust → looser sandbox; untrusted/unknown → most-restrictive.
        assert_eq!(trust_level_to_sandbox(3), SandboxLevel::None);
        assert_eq!(trust_level_to_sandbox(2), SandboxLevel::Standard);
        assert_eq!(trust_level_to_sandbox(1), SandboxLevel::Strict);
        assert_eq!(trust_level_to_sandbox(0), SandboxLevel::Paranoid);
        // Sandbox-by-default: any unrecognized value is most-restrictive.
        assert_eq!(trust_level_to_sandbox(99), SandboxLevel::Paranoid);
    }

    #[tokio::test]
    async fn test_upsert_policy_file_round_trips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("permissions.json");

        let p1 = SkillPermissionPolicy::for_source("alpha", "clawhub");
        upsert_policy_file(&path, &p1).expect("write alpha");
        assert!(path.exists(), "permissions.json should be created");

        // load() round-trips the persisted policy.
        let registry = SkillPermissionRegistry::with_persistence(path.clone());
        let count = registry.load().await;
        assert_eq!(count, 1);
    }

    #[test]
    fn test_upsert_policy_file_dedups_by_skill_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("permissions.json");

        // Two writes for the same skill → one row, not two.
        let mut p = SkillPermissionPolicy::for_source("beta", "clawhub");
        upsert_policy_file(&path, &p).expect("first write");
        p.trust_level = 2;
        upsert_policy_file(&path, &p).expect("second write");

        let content = std::fs::read_to_string(&path).expect("read");
        let policies: Vec<SkillPermissionPolicy> =
            serde_json::from_str(&content).expect("parse");
        assert_eq!(policies.len(), 1, "upsert must not accumulate near-dups");
        assert_eq!(policies[0].trust_level, 2, "latest write wins");
    }
}
