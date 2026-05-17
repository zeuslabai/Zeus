//! Node Registry — tracks connected WebSocket nodes (fleet agents).
//!
//! Hub-spoke architecture inspired by OpenClaw's NodeRegistry:
//! - Coordinator (.112) runs the hub, accepting persistent WS connections
//! - Fleet agents connect TO the coordinator via `/v1/ws/nodes`
//! - Supports invoke (request/response with UUID tracking + timeout),
//!   event push, and broadcasting

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

// ============================================================================
// Protocol types — messages between hub and connected nodes
// ============================================================================

/// Messages the hub sends to a connected node (over WS).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeServerMessage {
    /// Request-response invoke: hub asks node to do something
    Invoke {
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },
    /// Response to a node's invoke request
    InvokeResult {
        id: String,
        #[serde(default)]
        result: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// One-way event push from hub to node
    Event { event_type: String, data: Value },
    /// Keepalive ping
    Ping,
    /// Acknowledgement of successful registration
    Registered { node_id: String },
}

/// Messages a connected node sends to the hub (over WS).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeClientMessage {
    /// First message: node identifies itself
    Register {
        node_id: String,
        host: String,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    /// Response to a hub invoke request
    InvokeResult {
        id: String,
        #[serde(default)]
        result: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Node invokes something on the hub (reverse invoke)
    Invoke {
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },
    /// One-way event from node to hub
    Event { event_type: String, data: Value },
    /// Keepalive pong
    Pong,
}

// ============================================================================
// Node info (serializable summary without WS sender)
// ============================================================================

/// Summary of a connected node (safe to serialize — no channel handles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub host: String,
    pub connected_at: String,
    pub capabilities: Vec<String>,
}

// ============================================================================
// Connected node (internal — holds WS sender)
// ============================================================================

/// A connected node with its WS sender channel.
pub struct ConnectedNode {
    pub node_id: String,
    pub host: String,
    pub connected_at: DateTime<Utc>,
    pub capabilities: Vec<String>,
    /// Send messages TO this node via its WS connection.
    pub tx: mpsc::UnboundedSender<NodeServerMessage>,
}

impl ConnectedNode {
    /// Create a serializable summary (no channel handle).
    pub fn info(&self) -> NodeInfo {
        NodeInfo {
            node_id: self.node_id.clone(),
            host: self.host.clone(),
            connected_at: self.connected_at.to_rfc3339(),
            capabilities: self.capabilities.clone(),
        }
    }
}

// ============================================================================
// NodeRegistry — the hub's node tracking system
// ============================================================================

/// Registry of all connected WebSocket nodes.
///
/// Thread-safe via DashMap. The coordinator holds this in AppState.
pub struct NodeRegistry {
    /// Connected nodes: node_id -> ConnectedNode
    nodes: DashMap<String, ConnectedNode>,
    /// Pending invoke requests: invoke_id -> oneshot sender for response
    pending_invokes: DashMap<String, oneshot::Sender<InvokeResponse>>,
}

/// Response from an invoke call.
#[derive(Debug, Clone)]
pub struct InvokeResponse {
    pub result: Value,
    pub error: Option<String>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            pending_invokes: DashMap::new(),
        }
    }

    /// Register a node. Returns false if node_id is already connected (replaced).
    pub fn register(
        &self,
        node_id: String,
        host: String,
        capabilities: Vec<String>,
        tx: mpsc::UnboundedSender<NodeServerMessage>,
    ) -> bool {
        let was_new = !self.nodes.contains_key(&node_id);
        let node = ConnectedNode {
            node_id: node_id.clone(),
            host: host.clone(),
            connected_at: Utc::now(),
            capabilities,
            tx,
        };
        self.nodes.insert(node_id.clone(), node);
        if was_new {
            info!("Node registered: {} ({})", node_id, host);
        } else {
            info!("Node re-registered (replaced): {} ({})", node_id, host);
        }
        was_new
    }

    /// Unregister a node (called on WS disconnect).
    pub fn unregister(&self, node_id: &str) {
        if self.nodes.remove(node_id).is_some() {
            info!("Node unregistered: {}", node_id);
        }
    }

    /// List all connected nodes (serializable summaries).
    pub fn list_nodes(&self) -> Vec<NodeInfo> {
        self.nodes
            .iter()
            .map(|entry| entry.value().info())
            .collect()
    }

    /// Get info for a specific node.
    pub fn get_node_info(&self, node_id: &str) -> Option<NodeInfo> {
        self.nodes.get(node_id).map(|entry| entry.value().info())
    }

    /// Check if a node is connected.
    pub fn is_connected(&self, node_id: &str) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Number of connected nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Invoke a method on a connected node (request-response with timeout).
    ///
    /// Sends an Invoke message to the node and waits for InvokeResult.
    /// Returns the result or an error if the node doesn't respond within timeout.
    pub async fn invoke(
        &self,
        node_id: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let invoke_id = uuid::Uuid::new_v4().to_string();

        // Get the node's sender
        let tx = self
            .nodes
            .get(node_id)
            .map(|n| n.tx.clone())
            .ok_or_else(|| format!("Node '{}' not connected", node_id))?;

        // Create oneshot channel for the response
        let (resp_tx, resp_rx) = oneshot::channel();
        self.pending_invokes.insert(invoke_id.clone(), resp_tx);

        // Send invoke to node
        let msg = NodeServerMessage::Invoke {
            id: invoke_id.clone(),
            method: method.to_string(),
            params,
        };
        tx.send(msg)
            .map_err(|_| format!("Node '{}' WS channel closed", node_id))?;

        // Wait for response with timeout
        let result = tokio::time::timeout(timeout, resp_rx).await;

        // Clean up pending invoke regardless of outcome
        self.pending_invokes.remove(&invoke_id);

        match result {
            Ok(Ok(response)) => {
                if let Some(err) = response.error {
                    Err(err)
                } else {
                    Ok(response.result)
                }
            }
            Ok(Err(_)) => Err(format!("Node '{}' disconnected before responding", node_id)),
            Err(_) => Err(format!(
                "Invoke to '{}' timed out after {}s",
                node_id,
                timeout.as_secs()
            )),
        }
    }

    /// Handle an invoke result from a node (matches pending invoke by ID).
    pub fn handle_invoke_result(&self, invoke_id: &str, result: Value, error: Option<String>) {
        if let Some((_, sender)) = self.pending_invokes.remove(invoke_id) {
            let response = InvokeResponse { result, error };
            if sender.send(response).is_err() {
                warn!("Invoke {} response dropped (caller gone)", invoke_id);
            }
        } else {
            warn!("No pending invoke for ID: {}", invoke_id);
        }
    }

    /// Send a one-way event to a specific node.
    pub fn send_event(&self, node_id: &str, event_type: &str, data: Value) -> bool {
        if let Some(node) = self.nodes.get(node_id) {
            let msg = NodeServerMessage::Event {
                event_type: event_type.to_string(),
                data,
            };
            node.tx.send(msg).is_ok()
        } else {
            false
        }
    }

    /// Broadcast an event to ALL connected nodes.
    /// Returns the number of nodes that received the message.
    pub fn broadcast_event(&self, event_type: &str, data: Value) -> usize {
        let mut delivered = 0;
        for entry in self.nodes.iter() {
            let msg = NodeServerMessage::Event {
                event_type: event_type.to_string(),
                data: data.clone(),
            };
            if entry.tx.send(msg).is_ok() {
                delivered += 1;
            }
        }
        delivered
    }

    /// Send a raw message to a specific node.
    pub fn send_to_node(&self, node_id: &str, msg: NodeServerMessage) -> bool {
        if let Some(node) = self.nodes.get(node_id) {
            node.tx.send(msg).is_ok()
        } else {
            false
        }
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_list() {
        let registry = NodeRegistry::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(registry.register(
            "@zeus100".to_string(),
            "192.168.1.100".to_string(),
            vec!["rust".to_string()],
            tx,
        ));
        assert_eq!(registry.node_count(), 1);
        let nodes = registry.list_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, "@zeus100");
        assert_eq!(nodes[0].host, "192.168.1.100");
    }

    #[test]
    fn test_unregister() {
        let registry = NodeRegistry::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        registry.register("@test".to_string(), "1.2.3.4".to_string(), vec![], tx);
        assert!(registry.is_connected("@test"));
        registry.unregister("@test");
        assert!(!registry.is_connected("@test"));
        assert_eq!(registry.node_count(), 0);
    }

    #[test]
    fn test_send_event() {
        let registry = NodeRegistry::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        registry.register("@node1".to_string(), "10.0.0.1".to_string(), vec![], tx);

        let sent = registry.send_event(
            "@node1",
            "task_assigned",
            serde_json::json!({"task": "build"}),
        );
        assert!(sent);

        let msg = rx.try_recv().unwrap();
        match msg {
            NodeServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "task_assigned");
                assert_eq!(data["task"], "build");
            }
            _ => panic!("Expected Event message"),
        }
    }

    #[test]
    fn test_broadcast_event() {
        let registry = NodeRegistry::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        registry.register("@a".to_string(), "1.1.1.1".to_string(), vec![], tx1);
        registry.register("@b".to_string(), "2.2.2.2".to_string(), vec![], tx2);

        let count = registry.broadcast_event("ping", serde_json::json!({}));
        assert_eq!(count, 2);
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn test_re_register_replaces() {
        let registry = NodeRegistry::new();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        assert!(registry.register("@bot".to_string(), "1.1.1.1".to_string(), vec![], tx1));
        assert!(!registry.register("@bot".to_string(), "1.1.1.2".to_string(), vec![], tx2));
        assert_eq!(registry.node_count(), 1);
        let info = registry.get_node_info("@bot").unwrap();
        assert_eq!(info.host, "1.1.1.2");
    }

    #[tokio::test]
    async fn test_invoke_result_matching() {
        let registry = std::sync::Arc::new(NodeRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        registry.register("@worker".to_string(), "10.0.0.1".to_string(), vec![], tx);

        let reg = registry.clone();
        let handle = tokio::spawn(async move {
            reg.invoke(
                "@worker",
                "git_status",
                serde_json::json!({}),
                Duration::from_secs(5),
            )
            .await
        });

        // Simulate node receiving invoke and responding
        let msg = rx.recv().await.unwrap();
        if let NodeServerMessage::Invoke { id, .. } = msg {
            registry.handle_invoke_result(&id, serde_json::json!({"clean": true}), None);
        }

        let result: Result<serde_json::Value, String> = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["clean"], true);
    }

    #[tokio::test]
    async fn test_invoke_timeout() {
        let registry = NodeRegistry::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        registry.register("@slow".to_string(), "10.0.0.1".to_string(), vec![], tx);

        let result = registry
            .invoke(
                "@slow",
                "test",
                serde_json::json!({}),
                Duration::from_millis(50),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[tokio::test]
    async fn test_invoke_not_connected() {
        let registry = NodeRegistry::new();
        let result = registry
            .invoke(
                "@ghost",
                "test",
                serde_json::json!({}),
                Duration::from_secs(1),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not connected"));
    }
}
