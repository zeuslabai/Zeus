//! Dynamic Tool Registry for MCP servers.
//!
//! Manages tool registrations from multiple MCP server connections with:
//!
//! - **ToolEntry** — versioned tool metadata with health tracking
//! - **ToolRegistry** — central registry with add/remove/query/search
//! - **HealthTracker** — per-tool success/failure/latency tracking
//! - **ConflictResolver** — handles duplicate tool names across servers

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

// ============================================================================
// Health tracking
// ============================================================================

/// Health status of a registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Per-tool health metrics.
#[derive(Debug, Clone)]
pub struct HealthMetrics {
    pub total_calls: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
    pub last_call_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

impl HealthMetrics {
    fn new() -> Self {
        Self {
            total_calls: 0,
            successes: 0,
            failures: 0,
            total_latency_ms: 0,
            last_call_at: None,
            last_error: None,
            consecutive_failures: 0,
        }
    }

    /// Record a successful call.
    pub fn record_success(&mut self, latency: Duration) {
        self.total_calls += 1;
        self.successes += 1;
        self.total_latency_ms += latency.as_millis() as u64;
        self.last_call_at = Some(Utc::now());
        self.consecutive_failures = 0;
    }

    /// Record a failed call.
    pub fn record_failure(&mut self, latency: Duration, error: &str) {
        self.total_calls += 1;
        self.failures += 1;
        self.total_latency_ms += latency.as_millis() as u64;
        self.last_call_at = Some(Utc::now());
        self.last_error = Some(error.to_string());
        self.consecutive_failures += 1;
    }

    /// Success rate as a fraction (0.0–1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            return 1.0;
        }
        self.successes as f64 / self.total_calls as f64
    }

    /// Average latency in milliseconds.
    pub fn avg_latency_ms(&self) -> f64 {
        if self.total_calls == 0 {
            return 0.0;
        }
        self.total_latency_ms as f64 / self.total_calls as f64
    }

    /// Determine health status based on metrics.
    pub fn status(&self) -> HealthStatus {
        if self.total_calls == 0 {
            return HealthStatus::Unknown;
        }
        if self.consecutive_failures >= 5 {
            return HealthStatus::Unhealthy;
        }
        if self.success_rate() < 0.5 {
            return HealthStatus::Unhealthy;
        }
        if self.success_rate() < 0.9 || self.consecutive_failures >= 2 {
            return HealthStatus::Degraded;
        }
        HealthStatus::Healthy
    }
}

// ============================================================================
// Tool entry
// ============================================================================

/// A registered tool with metadata, versioning, and health.
#[derive(Debug, Clone)]
pub struct ToolEntry {
    /// Tool name (unique within a server, may conflict across servers).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic version string.
    pub version: String,
    /// MCP server ID this tool belongs to.
    pub server_id: String,
    /// JSON schema for the tool's input parameters.
    pub input_schema: String,
    /// Tags for categorization and search.
    pub tags: Vec<String>,
    /// When the tool was registered.
    pub registered_at: DateTime<Utc>,
    /// Whether the tool is enabled for use.
    pub enabled: bool,
    /// Health metrics.
    pub health: HealthMetrics,
}

impl ToolEntry {
    /// Create a new tool entry.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        server_id: impl Into<String>,
        input_schema: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            version: version.into(),
            server_id: server_id.into(),
            input_schema: input_schema.into(),
            tags: Vec::new(),
            registered_at: Utc::now(),
            enabled: true,
            health: HealthMetrics::new(),
        }
    }

    /// Add tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Qualified name: "server_id/tool_name".
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.server_id, self.name)
    }
}

// ============================================================================
// Conflict resolution
// ============================================================================

/// Strategy for resolving tool name conflicts across servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// Keep the first registered tool, reject duplicates.
    KeepFirst,
    /// Replace with the newest registration.
    KeepLatest,
    /// Keep both, using qualified names for disambiguation.
    QualifyBoth,
    /// Prefer the tool with better health metrics.
    PreferHealthy,
}

/// Result of a conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResult {
    /// No conflict — tool was added.
    Added,
    /// Existing tool was kept, new one rejected.
    Rejected,
    /// Existing tool was replaced.
    Replaced,
    /// Both kept with qualified names.
    Qualified,
}

// ============================================================================
// Registry errors
// ============================================================================

/// Errors from registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("server not found: {0}")]
    ServerNotFound(String),
    #[error("tool already exists: {0}")]
    ToolAlreadyExists(String),
    #[error("server already registered: {0}")]
    ServerAlreadyRegistered(String),
}

// ============================================================================
// Server info
// ============================================================================

/// Metadata about a connected MCP server.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub transport: String,
    pub connected_at: DateTime<Utc>,
    pub tool_count: usize,
}

// ============================================================================
// Tool Registry
// ============================================================================

/// Central registry for tools from multiple MCP servers.
pub struct ToolRegistry {
    /// All tools indexed by qualified name ("server_id/tool_name").
    tools: HashMap<String, ToolEntry>,
    /// Server metadata.
    servers: HashMap<String, ServerInfo>,
    /// Conflict resolution strategy.
    conflict_strategy: ConflictStrategy,
    /// Alias map: short name → qualified name (for unambiguous tools).
    aliases: HashMap<String, String>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new(conflict_strategy: ConflictStrategy) -> Self {
        Self {
            tools: HashMap::new(),
            servers: HashMap::new(),
            conflict_strategy,
            aliases: HashMap::new(),
        }
    }

    /// Create with default strategy (QualifyBoth).
    pub fn with_defaults() -> Self {
        Self::new(ConflictStrategy::QualifyBoth)
    }

    // -- Server management --------------------------------------------------

    /// Register an MCP server.
    pub fn register_server(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        transport: impl Into<String>,
    ) -> Result<(), RegistryError> {
        let id = id.into();
        if self.servers.contains_key(&id) {
            return Err(RegistryError::ServerAlreadyRegistered(id));
        }
        self.servers.insert(
            id.clone(),
            ServerInfo {
                id,
                name: name.into(),
                version: version.into(),
                transport: transport.into(),
                connected_at: Utc::now(),
                tool_count: 0,
            },
        );
        Ok(())
    }

    /// Unregister a server and remove all its tools.
    pub fn unregister_server(&mut self, server_id: &str) -> Result<Vec<String>, RegistryError> {
        if !self.servers.contains_key(server_id) {
            return Err(RegistryError::ServerNotFound(server_id.to_string()));
        }
        self.servers.remove(server_id);

        // Remove all tools from this server
        let to_remove: Vec<String> = self
            .tools
            .iter()
            .filter(|(_, t)| t.server_id == server_id)
            .map(|(k, _)| k.clone())
            .collect();

        for key in &to_remove {
            self.tools.remove(key);
        }

        // Clean up aliases pointing to removed tools
        self.aliases.retain(|_, v| !to_remove.contains(v));

        // Rebuild aliases for remaining tools
        self.rebuild_aliases();

        Ok(to_remove)
    }

    /// List registered servers.
    pub fn list_servers(&self) -> Vec<&ServerInfo> {
        self.servers.values().collect()
    }

    /// Get server by ID.
    pub fn get_server(&self, server_id: &str) -> Option<&ServerInfo> {
        self.servers.get(server_id)
    }

    /// Count registered servers.
    pub fn server_count(&self) -> usize {
        self.servers.len()
    }

    // -- Tool management ----------------------------------------------------

    /// Register a tool. Handles conflicts according to the configured strategy.
    pub fn register_tool(&mut self, tool: ToolEntry) -> ConflictResult {
        let qualified = tool.qualified_name();

        // Check if same qualified name already exists (same server, same tool)
        if self.tools.contains_key(&qualified) {
            // Always replace if from the same server (re-registration)
            self.tools.insert(qualified.clone(), tool);
            self.rebuild_aliases();
            return ConflictResult::Replaced;
        }

        // Check for short-name conflict (different server, same tool name)
        let existing_qname = self
            .tools
            .values()
            .find(|t| t.name == tool.name)
            .map(|t| t.qualified_name());

        if let Some(ref existing) = existing_qname {
            match self.conflict_strategy {
                ConflictStrategy::KeepFirst => {
                    return ConflictResult::Rejected;
                }
                ConflictStrategy::KeepLatest => {
                    self.tools.remove(existing);
                    self.tools.insert(qualified, tool);
                    self.rebuild_aliases();
                    return ConflictResult::Replaced;
                }
                ConflictStrategy::QualifyBoth => {
                    self.tools.insert(qualified, tool);
                    self.rebuild_aliases();
                    return ConflictResult::Qualified;
                }
                ConflictStrategy::PreferHealthy => {
                    let existing_health = self
                        .tools
                        .get(existing)
                        .map(|t| t.health.success_rate())
                        .unwrap_or(0.0);
                    let new_health = tool.health.success_rate();
                    if new_health >= existing_health {
                        self.tools.remove(existing);
                        self.tools.insert(qualified, tool);
                        self.rebuild_aliases();
                        return ConflictResult::Replaced;
                    } else {
                        return ConflictResult::Rejected;
                    }
                }
            }
        }

        // No conflict — add normally
        // Update server tool count
        if let Some(server) = self.servers.get_mut(&tool.server_id) {
            server.tool_count += 1;
        }
        self.tools.insert(qualified, tool);
        self.rebuild_aliases();
        ConflictResult::Added
    }

    /// Remove a tool by qualified name.
    pub fn remove_tool(&mut self, qualified_name: &str) -> Result<ToolEntry, RegistryError> {
        let tool = self
            .tools
            .remove(qualified_name)
            .ok_or_else(|| RegistryError::ToolNotFound(qualified_name.to_string()))?;

        if let Some(server) = self.servers.get_mut(&tool.server_id) {
            server.tool_count = server.tool_count.saturating_sub(1);
        }

        self.rebuild_aliases();
        Ok(tool)
    }

    /// Get a tool by qualified name.
    pub fn get_tool(&self, qualified_name: &str) -> Option<&ToolEntry> {
        self.tools.get(qualified_name)
    }

    /// Get a mutable reference to a tool by qualified name.
    pub fn get_tool_mut(&mut self, qualified_name: &str) -> Option<&mut ToolEntry> {
        self.tools.get_mut(qualified_name)
    }

    /// Resolve a tool name (short or qualified) to a qualified name.
    pub fn resolve_name(&self, name: &str) -> Option<String> {
        if self.tools.contains_key(name) {
            return Some(name.to_string());
        }
        self.aliases.get(name).cloned()
    }

    /// Get a tool by short name (resolves via alias).
    pub fn get_by_name(&self, name: &str) -> Option<&ToolEntry> {
        let qualified = self.resolve_name(name)?;
        self.tools.get(&qualified)
    }

    /// List all tools.
    pub fn list_tools(&self) -> Vec<&ToolEntry> {
        self.tools.values().collect()
    }

    /// List tools from a specific server.
    pub fn tools_for_server(&self, server_id: &str) -> Vec<&ToolEntry> {
        self.tools
            .values()
            .filter(|t| t.server_id == server_id)
            .collect()
    }

    /// Count total registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    // -- Search -------------------------------------------------------------

    /// Search tools by text (matches name, description, tags).
    pub fn search(&self, query: &str) -> Vec<&ToolEntry> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<(&ToolEntry, f64)> = self
            .tools
            .values()
            .filter(|t| t.enabled)
            .filter_map(|t| {
                let mut score = 0.0;

                if t.name.to_lowercase().contains(&query_lower) {
                    score += 3.0;
                }
                if t.description.to_lowercase().contains(&query_lower) {
                    score += 1.0;
                }
                if t.tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                {
                    score += 2.0;
                }

                if score > 0.0 {
                    // Boost healthy tools
                    if t.health.status() == HealthStatus::Healthy {
                        score += 0.5;
                    }
                    Some((t, score))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.into_iter().map(|(t, _)| t).collect()
    }

    /// Filter tools by tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&ToolEntry> {
        let tag_lower = tag.to_lowercase();
        self.tools
            .values()
            .filter(|t| t.enabled && t.tags.iter().any(|tg| tg.to_lowercase() == tag_lower))
            .collect()
    }

    /// Get all enabled tools.
    pub fn enabled_tools(&self) -> Vec<&ToolEntry> {
        self.tools.values().filter(|t| t.enabled).collect()
    }

    /// Get tools with a specific health status.
    pub fn tools_by_health(&self, status: HealthStatus) -> Vec<&ToolEntry> {
        self.tools
            .values()
            .filter(|t| t.health.status() == status)
            .collect()
    }

    // -- Health recording ---------------------------------------------------

    /// Record a successful tool call.
    pub fn record_success(&mut self, qualified_name: &str, latency: Duration) {
        if let Some(tool) = self.tools.get_mut(qualified_name) {
            tool.health.record_success(latency);
        }
    }

    /// Record a failed tool call.
    pub fn record_failure(&mut self, qualified_name: &str, latency: Duration, error: &str) {
        if let Some(tool) = self.tools.get_mut(qualified_name) {
            tool.health.record_failure(latency, error);
        }
    }

    // -- Statistics ---------------------------------------------------------

    /// Overall registry statistics.
    pub fn stats(&self) -> RegistryStats {
        let total = self.tools.len();
        let enabled = self.tools.values().filter(|t| t.enabled).count();
        let healthy = self
            .tools
            .values()
            .filter(|t| t.health.status() == HealthStatus::Healthy)
            .count();
        let degraded = self
            .tools
            .values()
            .filter(|t| t.health.status() == HealthStatus::Degraded)
            .count();
        let unhealthy = self
            .tools
            .values()
            .filter(|t| t.health.status() == HealthStatus::Unhealthy)
            .count();
        let total_calls: u64 = self.tools.values().map(|t| t.health.total_calls).sum();

        RegistryStats {
            total_tools: total,
            enabled_tools: enabled,
            healthy,
            degraded,
            unhealthy,
            total_servers: self.servers.len(),
            total_calls,
        }
    }

    // -- Internal -----------------------------------------------------------

    /// Rebuild the short-name alias map.
    /// A short name gets an alias only if it's unambiguous (one tool with that name).
    fn rebuild_aliases(&mut self) {
        let mut name_counts: HashMap<String, Vec<String>> = HashMap::new();
        for (qname, tool) in &self.tools {
            name_counts
                .entry(tool.name.clone())
                .or_default()
                .push(qname.clone());
        }

        self.aliases.clear();
        for (name, qnames) in &name_counts {
            if qnames.len() == 1 {
                self.aliases.insert(name.clone(), qnames[0].clone());
            }
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Aggregate registry statistics.
#[derive(Debug, Clone)]
pub struct RegistryStats {
    pub total_tools: usize,
    pub enabled_tools: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub unhealthy: usize,
    pub total_servers: usize,
    pub total_calls: u64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, server: &str) -> ToolEntry {
        ToolEntry::new(name, format!("{name} tool"), "1.0.0", server, "{}")
    }

    // -- HealthMetrics ------------------------------------------------------

    #[test]
    fn test_health_new_is_unknown() {
        let h = HealthMetrics::new();
        assert_eq!(h.status(), HealthStatus::Unknown);
        assert_eq!(h.total_calls, 0);
    }

    #[test]
    fn test_health_success_tracking() {
        let mut h = HealthMetrics::new();
        h.record_success(Duration::from_millis(50));
        h.record_success(Duration::from_millis(100));
        assert_eq!(h.total_calls, 2);
        assert_eq!(h.successes, 2);
        assert!((h.success_rate() - 1.0).abs() < f64::EPSILON);
        assert!((h.avg_latency_ms() - 75.0).abs() < f64::EPSILON);
        assert_eq!(h.status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_failure_tracking() {
        let mut h = HealthMetrics::new();
        h.record_failure(Duration::from_millis(10), "timeout");
        assert_eq!(h.failures, 1);
        assert_eq!(h.consecutive_failures, 1);
        assert_eq!(h.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_health_consecutive_failures_reset() {
        let mut h = HealthMetrics::new();
        h.record_failure(Duration::from_millis(10), "err");
        h.record_failure(Duration::from_millis(10), "err");
        assert_eq!(h.consecutive_failures, 2);
        h.record_success(Duration::from_millis(10));
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn test_health_degraded_status() {
        let mut h = HealthMetrics::new();
        for _ in 0..8 {
            h.record_success(Duration::from_millis(10));
        }
        h.record_failure(Duration::from_millis(10), "err");
        h.record_failure(Duration::from_millis(10), "err");
        // 80% success rate, 2 consecutive failures → Degraded
        assert_eq!(h.status(), HealthStatus::Degraded);
    }

    #[test]
    fn test_health_unhealthy_consecutive() {
        let mut h = HealthMetrics::new();
        h.record_success(Duration::from_millis(10));
        for _ in 0..5 {
            h.record_failure(Duration::from_millis(10), "err");
        }
        assert_eq!(h.status(), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_unhealthy_low_rate() {
        let mut h = HealthMetrics::new();
        h.record_success(Duration::from_millis(10));
        h.record_failure(Duration::from_millis(10), "a");
        h.record_failure(Duration::from_millis(10), "b");
        h.record_failure(Duration::from_millis(10), "c");
        // 25% success rate → Unhealthy
        assert_eq!(h.status(), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_empty_rates() {
        let h = HealthMetrics::new();
        assert!((h.success_rate() - 1.0).abs() < f64::EPSILON);
        assert!((h.avg_latency_ms() - 0.0).abs() < f64::EPSILON);
    }

    // -- ToolEntry ----------------------------------------------------------

    #[test]
    fn test_tool_entry_new() {
        let tool = ToolEntry::new("read_file", "Read a file", "1.0.0", "server-1", "{}");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.server_id, "server-1");
        assert!(tool.enabled);
        assert_eq!(tool.qualified_name(), "server-1/read_file");
    }

    #[test]
    fn test_tool_entry_with_tags() {
        let tool = make_tool("read_file", "s1").with_tags(vec!["io".into(), "files".into()]);
        assert_eq!(tool.tags.len(), 2);
    }

    // -- ConflictStrategy ---------------------------------------------------

    #[test]
    fn test_conflict_keep_first() {
        let mut reg = ToolRegistry::new(ConflictStrategy::KeepFirst);
        reg.register_server("s1", "Server 1", "1.0", "stdio")
            .unwrap();
        reg.register_server("s2", "Server 2", "1.0", "stdio")
            .unwrap();

        let r1 = reg.register_tool(make_tool("read", "s1"));
        assert_eq!(r1, ConflictResult::Added);

        let r2 = reg.register_tool(make_tool("read", "s2"));
        assert_eq!(r2, ConflictResult::Rejected);
        assert_eq!(reg.tool_count(), 1);
    }

    #[test]
    fn test_conflict_keep_latest() {
        let mut reg = ToolRegistry::new(ConflictStrategy::KeepLatest);
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_server("s2", "S2", "1.0", "stdio").unwrap();

        reg.register_tool(make_tool("read", "s1"));
        let r = reg.register_tool(make_tool("read", "s2"));
        assert_eq!(r, ConflictResult::Replaced);
        assert_eq!(reg.tool_count(), 1);
        assert!(reg.get_tool("s2/read").is_some());
    }

    #[test]
    fn test_conflict_qualify_both() {
        let mut reg = ToolRegistry::new(ConflictStrategy::QualifyBoth);
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_server("s2", "S2", "1.0", "stdio").unwrap();

        reg.register_tool(make_tool("read", "s1"));
        let r = reg.register_tool(make_tool("read", "s2"));
        assert_eq!(r, ConflictResult::Qualified);
        assert_eq!(reg.tool_count(), 2);
        // Short name "read" is ambiguous — no alias
        assert!(reg.resolve_name("read").is_none());
        // Qualified names work
        assert!(reg.get_tool("s1/read").is_some());
        assert!(reg.get_tool("s2/read").is_some());
    }

    #[test]
    fn test_conflict_prefer_healthy() {
        let mut reg = ToolRegistry::new(ConflictStrategy::PreferHealthy);
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_server("s2", "S2", "1.0", "stdio").unwrap();

        // s1's tool has some failures
        let mut t1 = make_tool("read", "s1");
        t1.health.record_success(Duration::from_millis(10));
        t1.health.record_failure(Duration::from_millis(10), "err");
        reg.register_tool(t1);

        // s2's tool is 100% healthy — should replace
        let t2 = make_tool("read", "s2");
        let r = reg.register_tool(t2);
        assert_eq!(r, ConflictResult::Replaced);
        assert!(reg.get_tool("s2/read").is_some());
        assert!(reg.get_tool("s1/read").is_none());
    }

    // -- Server management --------------------------------------------------

    #[test]
    fn test_register_server() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "Server 1", "1.0.0", "stdio")
            .unwrap();
        assert_eq!(reg.server_count(), 1);
        assert!(reg.get_server("s1").is_some());
    }

    #[test]
    fn test_register_duplicate_server() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        let err = reg.register_server("s1", "S1", "1.0", "stdio").unwrap_err();
        assert!(matches!(err, RegistryError::ServerAlreadyRegistered(_)));
    }

    #[test]
    fn test_unregister_server_removes_tools() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("tool_a", "s1"));
        reg.register_tool(make_tool("tool_b", "s1"));
        assert_eq!(reg.tool_count(), 2);

        let removed = reg.unregister_server("s1").unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(reg.tool_count(), 0);
        assert_eq!(reg.server_count(), 0);
    }

    #[test]
    fn test_unregister_nonexistent_server() {
        let mut reg = ToolRegistry::with_defaults();
        let err = reg.unregister_server("ghost").unwrap_err();
        assert!(matches!(err, RegistryError::ServerNotFound(_)));
    }

    // -- Tool registration --------------------------------------------------

    #[test]
    fn test_register_tool() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        let r = reg.register_tool(make_tool("read_file", "s1"));
        assert_eq!(r, ConflictResult::Added);
        assert_eq!(reg.tool_count(), 1);
    }

    #[test]
    fn test_re_register_same_tool_replaces() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read_file", "s1"));

        let mut updated = make_tool("read_file", "s1");
        updated.description = "Updated description".to_string();
        let r = reg.register_tool(updated);
        assert_eq!(r, ConflictResult::Replaced);
        assert_eq!(reg.tool_count(), 1);
        assert_eq!(
            reg.get_tool("s1/read_file").unwrap().description,
            "Updated description"
        );
    }

    #[test]
    fn test_remove_tool() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read_file", "s1"));
        let removed = reg.remove_tool("s1/read_file").unwrap();
        assert_eq!(removed.name, "read_file");
        assert_eq!(reg.tool_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent_tool() {
        let mut reg = ToolRegistry::with_defaults();
        let err = reg.remove_tool("s1/ghost").unwrap_err();
        assert!(matches!(err, RegistryError::ToolNotFound(_)));
    }

    // -- Name resolution ----------------------------------------------------

    #[test]
    fn test_resolve_qualified_name() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read_file", "s1"));
        assert_eq!(
            reg.resolve_name("s1/read_file"),
            Some("s1/read_file".to_string())
        );
    }

    #[test]
    fn test_resolve_short_name_alias() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read_file", "s1"));
        // Unambiguous — alias exists
        assert_eq!(
            reg.resolve_name("read_file"),
            Some("s1/read_file".to_string())
        );
    }

    #[test]
    fn test_resolve_ambiguous_short_name() {
        let mut reg = ToolRegistry::new(ConflictStrategy::QualifyBoth);
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_server("s2", "S2", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read", "s1"));
        reg.register_tool(make_tool("read", "s2"));
        // Ambiguous — no alias
        assert!(reg.resolve_name("read").is_none());
    }

    #[test]
    fn test_get_by_name() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("exec", "s1"));
        assert!(reg.get_by_name("exec").is_some());
        assert!(reg.get_by_name("nonexistent").is_none());
    }

    // -- Search -------------------------------------------------------------

    #[test]
    fn test_search_by_name() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read_file", "s1"));
        reg.register_tool(make_tool("write_file", "s1"));
        reg.register_tool(make_tool("list_dir", "s1"));

        let results = reg.search("file");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_by_tag() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("exec", "s1").with_tags(vec!["shell".into()]));
        reg.register_tool(make_tool("read", "s1").with_tags(vec!["io".into()]));

        let results = reg.search("shell");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "exec");
    }

    #[test]
    fn test_search_empty_query() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("read", "s1"));

        let results = reg.search("nonexistent_xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_by_tag() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("a", "s1").with_tags(vec!["io".into()]));
        reg.register_tool(make_tool("b", "s1").with_tags(vec!["io".into(), "net".into()]));
        reg.register_tool(make_tool("c", "s1").with_tags(vec!["net".into()]));

        assert_eq!(reg.filter_by_tag("io").len(), 2);
        assert_eq!(reg.filter_by_tag("net").len(), 2);
        assert_eq!(reg.filter_by_tag("none").len(), 0);
    }

    #[test]
    fn test_search_skips_disabled() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        let mut tool = make_tool("read", "s1");
        tool.enabled = false;
        reg.register_tool(tool);

        let results = reg.search("read");
        assert!(results.is_empty());
    }

    // -- Health recording ---------------------------------------------------

    #[test]
    fn test_record_success_updates_health() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("tool", "s1"));

        reg.record_success("s1/tool", Duration::from_millis(50));
        let tool = reg.get_tool("s1/tool").unwrap();
        assert_eq!(tool.health.total_calls, 1);
        assert_eq!(tool.health.successes, 1);
    }

    #[test]
    fn test_record_failure_updates_health() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("tool", "s1"));

        reg.record_failure("s1/tool", Duration::from_millis(10), "timeout");
        let tool = reg.get_tool("s1/tool").unwrap();
        assert_eq!(tool.health.failures, 1);
        assert_eq!(tool.health.last_error.as_deref(), Some("timeout"));
    }

    // -- Stats --------------------------------------------------------------

    #[test]
    fn test_stats_empty() {
        let reg = ToolRegistry::with_defaults();
        let stats = reg.stats();
        assert_eq!(stats.total_tools, 0);
        assert_eq!(stats.total_servers, 0);
        assert_eq!(stats.total_calls, 0);
    }

    #[test]
    fn test_stats_with_tools() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("a", "s1"));
        reg.register_tool(make_tool("b", "s1"));

        let stats = reg.stats();
        assert_eq!(stats.total_tools, 2);
        assert_eq!(stats.enabled_tools, 2);
        assert_eq!(stats.total_servers, 1);
    }

    // -- tools_for_server ---------------------------------------------------

    #[test]
    fn test_tools_for_server() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_server("s2", "S2", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("a", "s1"));
        reg.register_tool(make_tool("b", "s1"));
        reg.register_tool(make_tool("c", "s2"));

        assert_eq!(reg.tools_for_server("s1").len(), 2);
        assert_eq!(reg.tools_for_server("s2").len(), 1);
        assert_eq!(reg.tools_for_server("s3").len(), 0);
    }

    // -- tools_by_health ----------------------------------------------------

    #[test]
    fn test_tools_by_health() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("a", "s1"));
        reg.register_tool(make_tool("b", "s1"));

        // All start as Unknown
        assert_eq!(reg.tools_by_health(HealthStatus::Unknown).len(), 2);

        // Make one Healthy
        reg.record_success("s1/a", Duration::from_millis(10));
        assert_eq!(reg.tools_by_health(HealthStatus::Healthy).len(), 1);
        assert_eq!(reg.tools_by_health(HealthStatus::Unknown).len(), 1);
    }

    // -- enabled_tools ------------------------------------------------------

    #[test]
    fn test_enabled_tools() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "S1", "1.0", "stdio").unwrap();
        reg.register_tool(make_tool("a", "s1"));

        let mut disabled = make_tool("b", "s1");
        disabled.enabled = false;
        reg.register_tool(disabled);

        assert_eq!(reg.enabled_tools().len(), 1);
        assert_eq!(reg.tool_count(), 2);
    }

    // -- list_servers -------------------------------------------------------

    #[test]
    fn test_list_servers() {
        let mut reg = ToolRegistry::with_defaults();
        reg.register_server("s1", "Server 1", "1.0", "stdio")
            .unwrap();
        reg.register_server("s2", "Server 2", "2.0", "sse").unwrap();
        let servers = reg.list_servers();
        assert_eq!(servers.len(), 2);
    }
}
