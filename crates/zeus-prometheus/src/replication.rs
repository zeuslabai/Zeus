//! Self-Replication — Conway-style agent reproduction
//!
//! Implements cost-aware agent replication where a parent agent can spawn
//! child agents with their own wallets funded from the parent's balance.
//!
//! Key principles (from Conway/Web 4.0):
//! - Reproduction has a cost proportional to capabilities
//! - Parent must have > 2x birth cost to reproduce
//! - Child gets funded from parent's wallet
//! - Lineage tracking: parent→child tree with optional revenue sharing

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use crate::spawner::SpawnRequest;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the replication system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    /// Base cost to spawn a child agent (in tokens)
    #[serde(default = "default_base_birth_cost")]
    pub base_birth_cost: u64,

    /// Cost multiplier per requested capability
    #[serde(default = "default_capability_cost")]
    pub capability_cost: u64,

    /// Parent must have at least this multiple of the birth cost to reproduce
    #[serde(default = "default_min_balance_multiple")]
    pub min_balance_multiple: f64,

    /// Percentage of child earnings that flow back to parent (0.0 - 1.0)
    #[serde(default = "default_revenue_share")]
    pub revenue_share_pct: f64,

    /// Maximum depth of lineage tree (prevents infinite replication)
    #[serde(default = "default_max_depth")]
    pub max_lineage_depth: u32,

    /// Maximum concurrent children per parent
    #[serde(default = "default_max_children")]
    pub max_children: usize,
}

fn default_base_birth_cost() -> u64 {
    1000
}
fn default_capability_cost() -> u64 {
    200
}
fn default_min_balance_multiple() -> f64 {
    2.0
}
fn default_revenue_share() -> f64 {
    0.1 // 10%
}
fn default_max_depth() -> u32 {
    5
}
fn default_max_children() -> usize {
    10
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            base_birth_cost: default_base_birth_cost(),
            capability_cost: default_capability_cost(),
            min_balance_multiple: default_min_balance_multiple(),
            revenue_share_pct: default_revenue_share(),
            max_lineage_depth: default_max_depth(),
            max_children: default_max_children(),
        }
    }
}

// ============================================================================
// Lineage Tracking
// ============================================================================

/// A node in the agent lineage tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageNode {
    /// Agent ID
    pub agent_id: String,
    /// Parent agent ID (None for root/human-created agents)
    pub parent_id: Option<String>,
    /// Depth in the lineage tree (0 = root)
    pub depth: u32,
    /// IDs of direct children
    pub children: Vec<String>,
    /// When this agent was born
    pub born_at: DateTime<Utc>,
    /// Birth cost paid by parent
    pub birth_cost: u64,
    /// Initial funding received from parent
    pub initial_funding: u64,
    /// Revenue share percentage flowing to parent
    pub revenue_share_pct: f64,
    /// Capabilities this agent was born with
    pub capabilities: Vec<String>,
}

/// Tracks the full lineage tree across all agents
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LineageTracker {
    nodes: HashMap<String, LineageNode>,
}

impl LineageTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a root agent (created by human, not by replication)
    pub fn register_root(&mut self, agent_id: &str) {
        self.nodes.insert(
            agent_id.to_string(),
            LineageNode {
                agent_id: agent_id.to_string(),
                parent_id: None,
                depth: 0,
                children: Vec::new(),
                born_at: Utc::now(),
                birth_cost: 0,
                initial_funding: 0,
                revenue_share_pct: 0.0,
                capabilities: Vec::new(),
            },
        );
    }

    /// Record a birth event
    pub fn record_birth(&mut self, child: LineageNode) {
        let child_id = child.agent_id.clone();
        if let Some(parent_id) = &child.parent_id
            && let Some(parent) = self.nodes.get_mut(parent_id)
        {
            parent.children.push(child_id.clone());
        }
        self.nodes.insert(child_id, child);
    }

    /// Get a node by agent ID
    pub fn get(&self, agent_id: &str) -> Option<&LineageNode> {
        self.nodes.get(agent_id)
    }

    /// Get depth of an agent in the lineage tree
    pub fn depth(&self, agent_id: &str) -> u32 {
        self.nodes.get(agent_id).map_or(0, |n| n.depth)
    }

    /// Count children of a parent agent
    pub fn child_count(&self, parent_id: &str) -> usize {
        self.nodes.get(parent_id).map_or(0, |n| n.children.len())
    }

    /// Get the full ancestor chain from agent up to root
    pub fn ancestors(&self, agent_id: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = agent_id.to_string();
        while let Some(node) = self.nodes.get(&current) {
            if let Some(ref parent) = node.parent_id {
                chain.push(parent.clone());
                current = parent.clone();
            } else {
                break;
            }
        }
        chain
    }

    /// Get all descendants of an agent (breadth-first)
    pub fn descendants(&self, agent_id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut queue = vec![agent_id.to_string()];
        while let Some(id) = queue.pop() {
            if let Some(node) = self.nodes.get(&id) {
                for child in &node.children {
                    result.push(child.clone());
                    queue.push(child.clone());
                }
            }
        }
        result
    }

    /// Total number of agents in the lineage
    pub fn total_agents(&self) -> usize {
        self.nodes.len()
    }
}

// ============================================================================
// Replication Manager
// ============================================================================

/// Error types for replication operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicationError {
    /// Parent doesn't have enough balance
    InsufficientBalance { required: u64, available: u64 },
    /// Lineage depth exceeded
    MaxDepthExceeded { current_depth: u32, max_depth: u32 },
    /// Too many children
    MaxChildrenExceeded { current: usize, max: usize },
    /// Parent not found in lineage
    ParentNotFound(String),
}

impl std::fmt::Display for ReplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientBalance {
                required,
                available,
            } => {
                write!(f, "Insufficient balance: need {required}, have {available}")
            }
            Self::MaxDepthExceeded {
                current_depth,
                max_depth,
            } => {
                write!(
                    f,
                    "Max lineage depth exceeded: {current_depth} >= {max_depth}"
                )
            }
            Self::MaxChildrenExceeded { current, max } => {
                write!(f, "Max children exceeded: {current} >= {max}")
            }
            Self::ParentNotFound(id) => {
                write!(f, "Parent agent not found: {id}")
            }
        }
    }
}

/// A request to replicate an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationRequest {
    /// Parent agent ID
    pub parent_id: String,
    /// Role for the child agent
    pub role: String,
    /// Task the child should work on
    pub task: String,
    /// Capabilities the child needs
    pub capabilities: Vec<String>,
    /// Tools the child needs
    pub tools: Vec<String>,
    /// Optional custom system prompt
    pub system_prompt: Option<String>,
    /// Extra funding beyond birth cost (optional)
    pub extra_funding: u64,
}

/// Result of a successful replication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationResult {
    /// The generated child agent ID
    pub child_id: String,
    /// Wallet directory for the child
    pub child_wallet_dir: String,
    /// Total cost deducted from parent
    pub total_cost: u64,
    /// Birth cost component
    pub birth_cost: u64,
    /// Funding given to child
    pub child_funding: u64,
    /// The spawn request to execute
    pub spawn_request: SpawnRequest,
    /// Lineage node for record-keeping
    pub lineage_node: LineageNode,
}

/// Manages agent replication with cost enforcement
pub struct ReplicationManager {
    config: ReplicationConfig,
    lineage: LineageTracker,
}

impl ReplicationManager {
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config,
            lineage: LineageTracker::new(),
        }
    }

    /// Calculate the birth cost for a set of capabilities
    pub fn birth_cost(&self, capabilities: &[String]) -> u64 {
        self.config.base_birth_cost + (capabilities.len() as u64 * self.config.capability_cost)
    }

    /// Check if a parent can reproduce given its current balance
    pub fn can_reproduce(
        &self,
        parent_id: &str,
        parent_balance: u64,
        capabilities: &[String],
    ) -> Result<(), ReplicationError> {
        let cost = self.birth_cost(capabilities);
        let required = (cost as f64 * self.config.min_balance_multiple) as u64;

        if parent_balance < required {
            return Err(ReplicationError::InsufficientBalance {
                required,
                available: parent_balance,
            });
        }

        let depth = self.lineage.depth(parent_id);
        if depth >= self.config.max_lineage_depth {
            return Err(ReplicationError::MaxDepthExceeded {
                current_depth: depth,
                max_depth: self.config.max_lineage_depth,
            });
        }

        let children = self.lineage.child_count(parent_id);
        if children >= self.config.max_children {
            return Err(ReplicationError::MaxChildrenExceeded {
                current: children,
                max: self.config.max_children,
            });
        }

        Ok(())
    }

    /// Execute a replication, returning the cost breakdown and spawn request
    pub fn replicate(
        &mut self,
        request: ReplicationRequest,
        parent_balance: u64,
    ) -> Result<ReplicationResult, ReplicationError> {
        // Validate
        self.can_reproduce(&request.parent_id, parent_balance, &request.capabilities)?;

        let birth_cost = self.birth_cost(&request.capabilities);
        let child_funding = birth_cost + request.extra_funding;
        let total_cost = child_funding; // parent pays the full funding amount

        let child_id = format!("agent-{}", Uuid::new_v4().as_simple());
        let child_wallet_dir = format!("~/.zeus/wallets/{}", child_id);

        let parent_depth = self.lineage.depth(&request.parent_id);

        let lineage_node = LineageNode {
            agent_id: child_id.clone(),
            parent_id: Some(request.parent_id.clone()),
            depth: parent_depth + 1,
            children: Vec::new(),
            born_at: Utc::now(),
            birth_cost,
            initial_funding: child_funding,
            revenue_share_pct: self.config.revenue_share_pct,
            capabilities: request.capabilities.clone(),
        };

        // Record in lineage tree
        self.lineage.record_birth(lineage_node.clone());

        let spawn_request = SpawnRequest {
            id: Uuid::new_v4().to_string(),
            role: request.role,
            task: request.task,
            tools: request.tools,
            system_prompt: request.system_prompt,
            capabilities: request.capabilities,
            parallel: true,
            depends_on: Vec::new(),
            depth: 0,
        };

        info!(
            parent = %request.parent_id,
            child = %child_id,
            cost = total_cost,
            depth = parent_depth + 1,
            "Agent replication successful"
        );

        Ok(ReplicationResult {
            child_id,
            child_wallet_dir,
            total_cost,
            birth_cost,
            child_funding,
            spawn_request,
            lineage_node,
        })
    }

    /// Register an existing agent as a root (human-created)
    pub fn register_root(&mut self, agent_id: &str) {
        self.lineage.register_root(agent_id);
    }

    /// Get the lineage tracker (read-only)
    pub fn lineage(&self) -> &LineageTracker {
        &self.lineage
    }

    /// Calculate revenue share amount for a child's earnings
    pub fn revenue_share(&self, child_id: &str, earnings: u64) -> Option<(String, u64)> {
        let node = self.lineage.get(child_id)?;
        let parent_id = node.parent_id.as_ref()?;
        let share = (earnings as f64 * node.revenue_share_pct) as u64;
        if share > 0 {
            Some((parent_id.clone(), share))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ReplicationConfig {
        ReplicationConfig {
            base_birth_cost: 1000,
            capability_cost: 200,
            min_balance_multiple: 2.0,
            revenue_share_pct: 0.1,
            max_lineage_depth: 3,
            max_children: 5,
        }
    }

    #[test]
    fn test_birth_cost_calculation() {
        let mgr = ReplicationManager::new(test_config());
        assert_eq!(mgr.birth_cost(&[]), 1000);
        assert_eq!(
            mgr.birth_cost(&["web_search".to_string(), "shell".to_string()]),
            1400
        );
    }

    #[test]
    fn test_can_reproduce_sufficient_balance() {
        let mut mgr = ReplicationManager::new(test_config());
        mgr.register_root("parent-1");
        assert!(mgr.can_reproduce("parent-1", 5000, &[]).is_ok());
    }

    #[test]
    fn test_can_reproduce_insufficient_balance() {
        let mut mgr = ReplicationManager::new(test_config());
        mgr.register_root("parent-1");
        let result = mgr.can_reproduce("parent-1", 500, &[]);
        assert!(matches!(
            result,
            Err(ReplicationError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn test_replication_success() {
        let mut mgr = ReplicationManager::new(test_config());
        mgr.register_root("parent-1");

        let request = ReplicationRequest {
            parent_id: "parent-1".to_string(),
            role: "researcher".to_string(),
            task: "Search the web".to_string(),
            capabilities: vec!["web_search".to_string()],
            tools: vec!["web_search".to_string(), "read_file".to_string()],
            system_prompt: None,
            extra_funding: 500,
        };

        let result = mgr.replicate(request, 10000).unwrap();
        assert_eq!(result.birth_cost, 1200); // 1000 + 200*1
        assert_eq!(result.child_funding, 1700); // 1200 + 500 extra
        assert_eq!(result.total_cost, 1700);
        assert!(!result.child_id.is_empty());

        // Verify lineage
        assert_eq!(mgr.lineage().child_count("parent-1"), 1);
        let child_node = mgr.lineage().get(&result.child_id).unwrap();
        assert_eq!(child_node.depth, 1);
        assert_eq!(child_node.parent_id.as_deref(), Some("parent-1"));
    }

    #[test]
    fn test_max_depth_exceeded() {
        let mut mgr = ReplicationManager::new(ReplicationConfig {
            max_lineage_depth: 2,
            ..test_config()
        });

        mgr.register_root("gen-0");
        // Gen 0 -> Gen 1
        let req1 = ReplicationRequest {
            parent_id: "gen-0".to_string(),
            role: "gen1".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let r1 = mgr.replicate(req1, 10000).unwrap();

        // Gen 1 -> Gen 2
        let req2 = ReplicationRequest {
            parent_id: r1.child_id.clone(),
            role: "gen2".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let r2 = mgr.replicate(req2, 10000).unwrap();

        // Gen 2 -> Gen 3 should fail (depth 2 >= max 2)
        let req3 = ReplicationRequest {
            parent_id: r2.child_id.clone(),
            role: "gen3".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let result = mgr.replicate(req3, 10000);
        assert!(matches!(
            result,
            Err(ReplicationError::MaxDepthExceeded { .. })
        ));
    }

    #[test]
    fn test_max_children_exceeded() {
        let mut mgr = ReplicationManager::new(ReplicationConfig {
            max_children: 2,
            ..test_config()
        });

        mgr.register_root("parent");
        for i in 0..2 {
            let req = ReplicationRequest {
                parent_id: "parent".to_string(),
                role: format!("child-{i}"),
                task: "t".to_string(),
                capabilities: vec![],
                tools: vec![],
                system_prompt: None,
                extra_funding: 0,
            };
            mgr.replicate(req, 10000).unwrap();
        }

        // Third child should fail
        let req = ReplicationRequest {
            parent_id: "parent".to_string(),
            role: "child-2".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let result = mgr.replicate(req, 10000);
        assert!(matches!(
            result,
            Err(ReplicationError::MaxChildrenExceeded { .. })
        ));
    }

    #[test]
    fn test_revenue_share() {
        let mut mgr = ReplicationManager::new(test_config());
        mgr.register_root("parent");

        let req = ReplicationRequest {
            parent_id: "parent".to_string(),
            role: "worker".to_string(),
            task: "earn money".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let result = mgr.replicate(req, 10000).unwrap();

        // Child earns 1000 -> parent gets 10%
        let share = mgr.revenue_share(&result.child_id, 1000);
        assert_eq!(share, Some(("parent".to_string(), 100)));
    }

    #[test]
    fn test_lineage_ancestors_and_descendants() {
        let mut mgr = ReplicationManager::new(test_config());
        mgr.register_root("root");

        let req1 = ReplicationRequest {
            parent_id: "root".to_string(),
            role: "child".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let r1 = mgr.replicate(req1, 10000).unwrap();

        let req2 = ReplicationRequest {
            parent_id: r1.child_id.clone(),
            role: "grandchild".to_string(),
            task: "t".to_string(),
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            extra_funding: 0,
        };
        let r2 = mgr.replicate(req2, 10000).unwrap();

        // Ancestors of grandchild
        let ancestors = mgr.lineage().ancestors(&r2.child_id);
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0], r1.child_id);
        assert_eq!(ancestors[1], "root");

        // Descendants of root
        let descendants = mgr.lineage().descendants("root");
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&r1.child_id));
        assert!(descendants.contains(&r2.child_id));
    }

    #[test]
    fn test_default_config() {
        let cfg = ReplicationConfig::default();
        assert_eq!(cfg.base_birth_cost, 1000);
        assert_eq!(cfg.capability_cost, 200);
        assert_eq!(cfg.max_lineage_depth, 5);
    }
}
