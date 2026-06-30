//! Agent Domain Registry — Named identity system for autonomous agents
//!
//! Provides a DNS-like naming system for the Conway agent ecosystem:
//!
//! 1. **Domain registration**: agents claim human-readable names (e.g. "coder.zeus")
//! 2. **Registration cost**: domains cost tokens from the economy ledger
//! 3. **Expiration & renewal**: domains expire after a configurable period
//! 4. **Transfer**: domains can be transferred between agents
//! 5. **Resolution**: look up agent_id from domain name
//! 6. **Subdomain support**: hierarchical naming (e.g. "rust.coder.zeus")
//!
//! Integrates with TokenLedger for registration/renewal payments.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

// ============================================================================
// Domain Types
// ============================================================================

/// A registered domain name
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainRecord {
    /// The domain name (e.g. "coder.zeus")
    pub name: String,
    /// Agent ID that owns this domain
    pub owner_id: String,
    /// When the domain was registered
    pub registered_at: DateTime<Utc>,
    /// When the domain expires
    pub expires_at: DateTime<Utc>,
    /// Optional metadata (description, tags, endpoint URL)
    pub metadata: DomainMetadata,
    /// Whether the domain is currently active
    pub active: bool,
}

/// Metadata attached to a domain
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DomainMetadata {
    /// Human-readable description
    pub description: Option<String>,
    /// Tags for discovery
    pub tags: Vec<String>,
    /// API endpoint URL
    pub endpoint_url: Option<String>,
    /// Public key (hex) for authentication
    pub public_key: Option<String>,
}

/// Result of a domain operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainResult {
    Success,
    AlreadyTaken,
    NotFound,
    NotOwner,
    Expired,
    InvalidName,
    InsufficientFunds,
}

impl std::fmt::Display for DomainResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::AlreadyTaken => write!(f, "domain already taken"),
            Self::NotFound => write!(f, "domain not found"),
            Self::NotOwner => write!(f, "not the domain owner"),
            Self::Expired => write!(f, "domain expired"),
            Self::InvalidName => write!(f, "invalid domain name"),
            Self::InsufficientFunds => write!(f, "insufficient funds"),
        }
    }
}

// ============================================================================
// Registry Configuration
// ============================================================================

/// Configuration for the domain registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRegistryConfig {
    /// Cost to register a domain (in tokens)
    pub registration_cost: u64,
    /// Cost to renew a domain (in tokens)
    pub renewal_cost: u64,
    /// Domain validity period in days
    pub validity_days: i64,
    /// Top-level domain suffix (e.g. "zeus")
    pub tld: String,
    /// Minimum domain name length (excluding TLD)
    pub min_name_length: usize,
    /// Maximum domain name length (excluding TLD)
    pub max_name_length: usize,
    /// Maximum subdomains per domain
    pub max_subdomains: usize,
    /// Reserved names that cannot be registered
    pub reserved_names: Vec<String>,
}

impl Default for DomainRegistryConfig {
    fn default() -> Self {
        Self {
            registration_cost: 100,
            renewal_cost: 50,
            validity_days: 365,
            tld: "zeus".into(),
            min_name_length: 2,
            max_name_length: 32,
            max_subdomains: 5,
            reserved_names: vec![
                "admin".into(),
                "system".into(),
                "root".into(),
                "zeus".into(),
                "prometheus".into(),
                "coordinator".into(),
            ],
        }
    }
}

// ============================================================================
// Domain Registry
// ============================================================================

/// The domain registry — manages agent name registrations
pub struct DomainRegistry {
    config: DomainRegistryConfig,
    /// Domain name → DomainRecord
    domains: HashMap<String, DomainRecord>,
    /// Agent ID → list of owned domain names
    agent_domains: HashMap<String, Vec<String>>,
    /// Transfer history
    transfers: Vec<DomainTransfer>,
}

/// Record of a domain transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainTransfer {
    pub domain: String,
    pub from_agent: String,
    pub to_agent: String,
    pub timestamp: DateTime<Utc>,
}

impl DomainRegistry {
    pub fn new(config: DomainRegistryConfig) -> Self {
        Self {
            config,
            domains: HashMap::new(),
            agent_domains: HashMap::new(),
            transfers: Vec::new(),
        }
    }

    /// Validate a domain name format
    pub fn validate_name(&self, name: &str) -> DomainResult {
        // Must end with .tld
        let suffix = format!(".{}", self.config.tld);
        if !name.ends_with(&suffix) {
            return DomainResult::InvalidName;
        }

        // Extract the name part (without TLD)
        let name_part = &name[..name.len() - suffix.len()];
        if name_part.is_empty() {
            return DomainResult::InvalidName;
        }

        // Check each label
        let labels: Vec<&str> = name_part.split('.').collect();
        if labels.is_empty() || labels.len() > self.config.max_subdomains + 1 {
            return DomainResult::InvalidName;
        }

        for label in &labels {
            if label.is_empty() {
                return DomainResult::InvalidName;
            }
            if label.len() < self.config.min_name_length
                || label.len() > self.config.max_name_length
            {
                return DomainResult::InvalidName;
            }
            // Only alphanumeric and hyphens, no leading/trailing hyphens
            if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return DomainResult::InvalidName;
            }
            if label.starts_with('-') || label.ends_with('-') {
                return DomainResult::InvalidName;
            }
        }

        // Check reserved names (first label)
        let first_label = labels[0].to_lowercase();
        if self.config.reserved_names.contains(&first_label) {
            return DomainResult::InvalidName;
        }

        DomainResult::Success
    }

    /// Register a new domain for an agent.
    /// Returns (DomainResult, cost_charged).
    pub fn register(
        &mut self,
        name: &str,
        agent_id: &str,
        metadata: DomainMetadata,
        balance: u64,
    ) -> (DomainResult, u64) {
        // Validate name
        let validation = self.validate_name(name);
        if validation != DomainResult::Success {
            return (validation, 0);
        }

        // Check if taken (and not expired)
        if let Some(existing) = self.domains.get(name)
            && existing.active
            && existing.expires_at > Utc::now()
        {
            return (DomainResult::AlreadyTaken, 0);
        }
        // Expired — allow re-registration

        // Check funds
        if balance < self.config.registration_cost {
            return (DomainResult::InsufficientFunds, 0);
        }

        let now = Utc::now();
        let record = DomainRecord {
            name: name.to_string(),
            owner_id: agent_id.to_string(),
            registered_at: now,
            expires_at: now + Duration::days(self.config.validity_days),
            metadata,
            active: true,
        };

        self.domains.insert(name.to_string(), record);
        self.agent_domains
            .entry(agent_id.to_string())
            .or_default()
            .push(name.to_string());

        info!(
            domain = name,
            agent = agent_id,
            cost = self.config.registration_cost,
            "Domain registered"
        );

        (DomainResult::Success, self.config.registration_cost)
    }

    /// Renew an existing domain.
    /// Returns (DomainResult, cost_charged).
    pub fn renew(&mut self, name: &str, agent_id: &str, balance: u64) -> (DomainResult, u64) {
        let Some(record) = self.domains.get_mut(name) else {
            return (DomainResult::NotFound, 0);
        };

        if record.owner_id != agent_id {
            return (DomainResult::NotOwner, 0);
        }

        if balance < self.config.renewal_cost {
            return (DomainResult::InsufficientFunds, 0);
        }

        // Extend from current expiry (or now if already expired)
        let base = if record.expires_at > Utc::now() {
            record.expires_at
        } else {
            Utc::now()
        };
        record.expires_at = base + Duration::days(self.config.validity_days);
        record.active = true;

        info!(domain = name, agent = agent_id, expires = %record.expires_at, "Domain renewed");

        (DomainResult::Success, self.config.renewal_cost)
    }

    /// Transfer a domain to another agent
    pub fn transfer(&mut self, name: &str, from_agent: &str, to_agent: &str) -> DomainResult {
        let Some(record) = self.domains.get_mut(name) else {
            return DomainResult::NotFound;
        };

        if record.owner_id != from_agent {
            return DomainResult::NotOwner;
        }

        if record.expires_at <= Utc::now() {
            return DomainResult::Expired;
        }

        // Remove from old owner's list
        if let Some(list) = self.agent_domains.get_mut(from_agent) {
            list.retain(|d| d != name);
        }

        // Update ownership
        record.owner_id = to_agent.to_string();

        // Add to new owner's list
        self.agent_domains
            .entry(to_agent.to_string())
            .or_default()
            .push(name.to_string());

        // Record transfer
        self.transfers.push(DomainTransfer {
            domain: name.to_string(),
            from_agent: from_agent.to_string(),
            to_agent: to_agent.to_string(),
            timestamp: Utc::now(),
        });

        info!(
            domain = name,
            from = from_agent,
            to = to_agent,
            "Domain transferred"
        );

        DomainResult::Success
    }

    /// Resolve a domain name to an agent ID
    pub fn resolve(&self, name: &str) -> Option<&DomainRecord> {
        let record = self.domains.get(name)?;
        if record.active && record.expires_at > Utc::now() {
            Some(record)
        } else {
            None
        }
    }

    /// Look up all domains owned by an agent
    pub fn domains_for(&self, agent_id: &str) -> Vec<&DomainRecord> {
        self.agent_domains
            .get(agent_id)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| self.domains.get(n))
                    .filter(|r| r.active && r.expires_at > Utc::now())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all active domains
    pub fn list_active(&self) -> Vec<&DomainRecord> {
        let now = Utc::now();
        self.domains
            .values()
            .filter(|r| r.active && r.expires_at > now)
            .collect()
    }

    /// Search domains by tag
    pub fn search_by_tag(&self, tag: &str) -> Vec<&DomainRecord> {
        let now = Utc::now();
        let tag_lower = tag.to_lowercase();
        self.domains
            .values()
            .filter(|r| {
                r.active
                    && r.expires_at > now
                    && r.metadata
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase() == tag_lower)
            })
            .collect()
    }

    /// Expire old domains (garbage collection)
    pub fn expire_domains(&mut self) -> usize {
        let now = Utc::now();
        let mut expired_count = 0;

        for record in self.domains.values_mut() {
            if record.active && record.expires_at <= now {
                record.active = false;
                expired_count += 1;
                debug!(domain = %record.name, owner = %record.owner_id, "Domain expired");
            }
        }

        if expired_count > 0 {
            warn!(count = expired_count, "Domains expired during GC");
        }

        expired_count
    }

    /// Revoke a domain (admin action)
    pub fn revoke(&mut self, name: &str) -> DomainResult {
        let Some(record) = self.domains.get_mut(name) else {
            return DomainResult::NotFound;
        };
        record.active = false;
        info!(domain = name, "Domain revoked");
        DomainResult::Success
    }

    /// Get transfer history
    pub fn transfers(&self) -> &[DomainTransfer] {
        &self.transfers
    }

    /// Get transfer history for a specific domain
    pub fn transfers_for(&self, name: &str) -> Vec<&DomainTransfer> {
        self.transfers.iter().filter(|t| t.domain == name).collect()
    }

    /// Total registered domains (active + inactive)
    pub fn total_domains(&self) -> usize {
        self.domains.len()
    }

    /// Total active domains
    pub fn active_count(&self) -> usize {
        let now = Utc::now();
        self.domains
            .values()
            .filter(|r| r.active && r.expires_at > now)
            .count()
    }

    /// Get the registry config
    pub fn config(&self) -> &DomainRegistryConfig {
        &self.config
    }
}

impl Default for DomainRegistry {
    fn default() -> Self {
        Self::new(DomainRegistryConfig::default())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> DomainRegistry {
        DomainRegistry::new(DomainRegistryConfig {
            registration_cost: 100,
            renewal_cost: 50,
            validity_days: 30,
            tld: "zeus".into(),
            min_name_length: 2,
            max_name_length: 32,
            max_subdomains: 5,
            reserved_names: vec!["admin".into(), "system".into()],
        })
    }

    fn meta() -> DomainMetadata {
        DomainMetadata {
            description: Some("Test agent".into()),
            tags: vec!["test".into()],
            endpoint_url: Some("http://localhost:9000".into()),
            public_key: None,
        }
    }

    #[test]
    fn test_register_domain() {
        let mut reg = test_registry();
        let (result, cost) = reg.register("coder.zeus", "agent-1", meta(), 1000);
        assert_eq!(result, DomainResult::Success);
        assert_eq!(cost, 100);
    }

    #[test]
    fn test_resolve_domain() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let record = reg.resolve("coder.zeus").unwrap();
        assert_eq!(record.owner_id, "agent-1");
        assert_eq!(record.name, "coder.zeus");
    }

    #[test]
    fn test_domain_not_found() {
        let reg = test_registry();
        assert!(reg.resolve("nobody.zeus").is_none());
    }

    #[test]
    fn test_domain_already_taken() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let (result, cost) = reg.register("coder.zeus", "agent-2", meta(), 1000);
        assert_eq!(result, DomainResult::AlreadyTaken);
        assert_eq!(cost, 0);
    }

    #[test]
    fn test_insufficient_funds() {
        let mut reg = test_registry();
        let (result, _) = reg.register("coder.zeus", "agent-1", meta(), 50);
        assert_eq!(result, DomainResult::InsufficientFunds);
    }

    #[test]
    fn test_invalid_name_no_tld() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("coder"), DomainResult::InvalidName);
    }

    #[test]
    fn test_invalid_name_too_short() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("a.zeus"), DomainResult::InvalidName);
    }

    #[test]
    fn test_invalid_name_special_chars() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("co@der.zeus"), DomainResult::InvalidName);
    }

    #[test]
    fn test_invalid_name_leading_hyphen() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("-coder.zeus"), DomainResult::InvalidName);
    }

    #[test]
    fn test_reserved_name() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("admin.zeus"), DomainResult::InvalidName);
        assert_eq!(reg.validate_name("system.zeus"), DomainResult::InvalidName);
    }

    #[test]
    fn test_valid_subdomain() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("rust.coder.zeus"), DomainResult::Success);
    }

    #[test]
    fn test_renew_domain() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let old_expiry = reg.resolve("coder.zeus").unwrap().expires_at;
        let (result, cost) = reg.renew("coder.zeus", "agent-1", 500);
        assert_eq!(result, DomainResult::Success);
        assert_eq!(cost, 50);
        let new_expiry = reg.resolve("coder.zeus").unwrap().expires_at;
        assert!(new_expiry > old_expiry);
    }

    #[test]
    fn test_renew_not_owner() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let (result, _) = reg.renew("coder.zeus", "agent-2", 500);
        assert_eq!(result, DomainResult::NotOwner);
    }

    #[test]
    fn test_transfer_domain() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let result = reg.transfer("coder.zeus", "agent-1", "agent-2");
        assert_eq!(result, DomainResult::Success);
        let record = reg.resolve("coder.zeus").unwrap();
        assert_eq!(record.owner_id, "agent-2");
    }

    #[test]
    fn test_transfer_not_owner() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let result = reg.transfer("coder.zeus", "agent-3", "agent-2");
        assert_eq!(result, DomainResult::NotOwner);
    }

    #[test]
    fn test_domains_for_agent() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        reg.register("helper.zeus", "agent-1", meta(), 1000);
        reg.register("other.zeus", "agent-2", meta(), 1000);
        let domains = reg.domains_for("agent-1");
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn test_list_active() {
        let mut reg = test_registry();
        reg.register("aa.zeus", "a1", meta(), 1000);
        reg.register("bb.zeus", "a2", meta(), 1000);
        assert_eq!(reg.list_active().len(), 2);
    }

    #[test]
    fn test_search_by_tag() {
        let mut reg = test_registry();
        let mut m1 = meta();
        m1.tags = vec!["rust".into(), "code".into()];
        let mut m2 = meta();
        m2.tags = vec!["python".into()];
        reg.register("rs.zeus", "a1", m1, 1000);
        reg.register("py.zeus", "a2", m2, 1000);
        let results = reg.search_by_tag("rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rs.zeus");
    }

    #[test]
    fn test_revoke_domain() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        let result = reg.revoke("coder.zeus");
        assert_eq!(result, DomainResult::Success);
        assert!(reg.resolve("coder.zeus").is_none());
    }

    #[test]
    fn test_transfer_history() {
        let mut reg = test_registry();
        reg.register("coder.zeus", "a1", meta(), 1000);
        reg.transfer("coder.zeus", "a1", "a2");
        reg.transfer("coder.zeus", "a2", "a3");
        assert_eq!(reg.transfers().len(), 2);
        let for_domain = reg.transfers_for("coder.zeus");
        assert_eq!(for_domain.len(), 2);
        assert_eq!(for_domain[0].from_agent, "a1");
        assert_eq!(for_domain[1].from_agent, "a2");
    }

    #[test]
    fn test_active_count() {
        let mut reg = test_registry();
        reg.register("aa.zeus", "a1", meta(), 1000);
        reg.register("bb.zeus", "a2", meta(), 1000);
        reg.revoke("bb.zeus");
        assert_eq!(reg.active_count(), 1);
        assert_eq!(reg.total_domains(), 2);
    }

    #[test]
    fn test_expire_and_reregister() {
        let mut reg = test_registry(); // validity_days=30
        reg.register("coder.zeus", "agent-1", meta(), 1000);
        // Manually expire by backdating
        if let Some(r) = reg.domains.get_mut("coder.zeus") {
            r.expires_at = Utc::now() - Duration::hours(1);
        }
        let expired = reg.expire_domains();
        assert_eq!(expired, 1);
        assert!(reg.resolve("coder.zeus").is_none());
        // Re-register by different agent (expired domain allows re-registration)
        let (result, _) = reg.register("coder.zeus", "agent-2", meta(), 1000);
        assert_eq!(result, DomainResult::Success);
        assert_eq!(reg.resolve("coder.zeus").unwrap().owner_id, "agent-2");
    }

    #[test]
    fn test_hyphenated_name_valid() {
        let reg = test_registry();
        assert_eq!(reg.validate_name("my-agent.zeus"), DomainResult::Success);
    }

    #[test]
    fn test_default_registry() {
        let reg = DomainRegistry::default();
        assert_eq!(reg.config().tld, "zeus");
        assert_eq!(reg.config().registration_cost, 100);
    }
}
