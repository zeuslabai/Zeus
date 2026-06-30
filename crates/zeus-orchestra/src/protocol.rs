//! Inter-Agent Protocol - Typed message layer on top of MessageBus
//!
//! Provides structured protocol messages (work requests, broadcasts,
//! capability queries, heartbeats) serialized as JSON into the existing
//! `MessageBus` `Message.content` field.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::state::AgentStatus;
use crate::{Message, MessageBus, MessageType};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Scope for broadcast messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BroadcastScope {
    All,
    Team(String),
    Capability(String),
}

/// Status of a work request/response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum WorkStatus {
    Accepted,
    Rejected(String),
    InProgress,
    Completed(String),
    Failed(String),
}

/// A typed inter-agent protocol message.
///
/// These are serialized as JSON into the `content` field of a `Message`
/// on the `MessageBus`, with `message_type` set to `System`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case")]
pub enum ProtocolMessage {
    DirectMessage {
        from: String,
        to: String,
        content: String,
        reply_to: Option<String>,
    },
    Broadcast {
        from: String,
        content: String,
        scope: BroadcastScope,
    },
    CapabilityQuery {
        from: String,
        required_capabilities: Vec<String>,
    },
    CapabilityResponse {
        from: String,
        capabilities: Vec<String>,
        load: f32,
    },
    WorkRequest {
        id: String,
        from: String,
        to: String,
        task: String,
        context: Option<String>,
        priority: u8,
        deadline: Option<DateTime<Utc>>,
    },
    WorkResponse {
        from: String,
        request_id: String,
        status: WorkStatus,
    },
    StatusUpdate {
        from: String,
        status: AgentStatus,
        current_task: Option<String>,
    },
    Heartbeat {
        from: String,
        timestamp: DateTime<Utc>,
        health_score: f32,
    },
}

impl ProtocolMessage {
    /// Convert this typed protocol message into a `MessageBus` `Message`.
    pub fn to_bus_message(&self) -> Message {
        let content = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string());
        let from = self.sender();
        let to = self.target();

        Message {
            id: uuid::Uuid::new_v4().to_string(),
            from_agent: from,
            to_agent: to,
            content,
            timestamp: Utc::now(),
            message_type: MessageType::System,
        }
    }

    /// Try to deserialize a `MessageBus` `Message` back into a `ProtocolMessage`.
    /// Returns `None` for legacy/non-protocol messages.
    pub fn from_bus_message(msg: &Message) -> Option<Self> {
        serde_json::from_str(&msg.content).ok()
    }

    /// Extract the sender agent ID.
    pub fn sender(&self) -> String {
        match self {
            Self::DirectMessage { from, .. }
            | Self::Broadcast { from, .. }
            | Self::CapabilityQuery { from, .. }
            | Self::CapabilityResponse { from, .. }
            | Self::WorkRequest { from, .. }
            | Self::WorkResponse { from, .. }
            | Self::StatusUpdate { from, .. }
            | Self::Heartbeat { from, .. } => from.clone(),
        }
    }

    /// Extract the target agent ID (if any).
    pub fn target(&self) -> Option<String> {
        match self {
            Self::DirectMessage { to, .. } | Self::WorkRequest { to, .. } => Some(to.clone()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ProtocolHandler
// ---------------------------------------------------------------------------

/// High-level handler that wraps a `MessageBus` for typed protocol messages.
pub struct ProtocolHandler {
    bus: Arc<MessageBus>,
}

impl ProtocolHandler {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self { bus }
    }

    /// Send a typed protocol message through the bus.
    pub async fn send(&self, msg: ProtocolMessage) -> Result<(), crate::OrchestraError> {
        self.bus.send(msg.to_bus_message()).await
    }

    /// Subscribe to the underlying bus (caller filters for protocol messages).
    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.bus.subscribe()
    }

    // -- Convenience methods ------------------------------------------------

    /// Send a direct message between two agents.
    pub async fn send_direct(
        &self,
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<(), crate::OrchestraError> {
        self.send(ProtocolMessage::DirectMessage {
            from: from.into(),
            to: to.into(),
            content: content.into(),
            reply_to: None,
        })
        .await
    }

    /// Broadcast a message to a given scope.
    pub async fn broadcast(
        &self,
        from: impl Into<String>,
        content: impl Into<String>,
        scope: BroadcastScope,
    ) -> Result<(), crate::OrchestraError> {
        self.send(ProtocolMessage::Broadcast {
            from: from.into(),
            content: content.into(),
            scope,
        })
        .await
    }

    /// Send a work request to a specific agent.
    pub async fn request_work(
        &self,
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        task: impl Into<String>,
        priority: u8,
    ) -> Result<(), crate::OrchestraError> {
        self.send(ProtocolMessage::WorkRequest {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            task: task.into(),
            context: None,
            priority,
            deadline: None,
        })
        .await
    }

    /// Respond to a work request.
    pub async fn respond_work(
        &self,
        from: impl Into<String>,
        request_id: impl Into<String>,
        status: WorkStatus,
    ) -> Result<(), crate::OrchestraError> {
        self.send(ProtocolMessage::WorkResponse {
            from: from.into(),
            request_id: request_id.into(),
            status,
        })
        .await
    }

    /// Send a heartbeat.
    pub async fn send_heartbeat(
        &self,
        from: impl Into<String>,
        health_score: f32,
    ) -> Result<(), crate::OrchestraError> {
        self.send(ProtocolMessage::Heartbeat {
            from: from.into(),
            timestamp: Utc::now(),
            health_score,
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_message_serde() {
        let msg = ProtocolMessage::DirectMessage {
            from: "a".into(),
            to: "b".into(),
            content: "hello".into(),
            reply_to: None,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "a");
        assert_eq!(de.target(), Some("b".into()));
    }

    #[test]
    fn test_broadcast_serde() {
        let msg = ProtocolMessage::Broadcast {
            from: "a".into(),
            content: "hey all".into(),
            scope: BroadcastScope::All,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "a");
        assert!(de.target().is_none());
    }

    #[test]
    fn test_capability_query_serde() {
        let msg = ProtocolMessage::CapabilityQuery {
            from: "coordinator".into(),
            required_capabilities: vec!["code".into(), "review".into()],
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "coordinator");
    }

    #[test]
    fn test_capability_response_serde() {
        let msg = ProtocolMessage::CapabilityResponse {
            from: "worker".into(),
            capabilities: vec!["code".into()],
            load: 0.3,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "worker");
    }

    #[test]
    fn test_work_request_serde() {
        let msg = ProtocolMessage::WorkRequest {
            id: "wr-1".into(),
            from: "boss".into(),
            to: "worker".into(),
            task: "build feature".into(),
            context: Some("repo: zeus".into()),
            priority: 5,
            deadline: None,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "boss");
        assert_eq!(de.target(), Some("worker".into()));
    }

    #[test]
    fn test_work_response_serde() {
        let msg = ProtocolMessage::WorkResponse {
            from: "worker".into(),
            request_id: "wr-1".into(),
            status: WorkStatus::Completed("done!".into()),
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "worker");
    }

    #[test]
    fn test_status_update_serde() {
        let msg = ProtocolMessage::StatusUpdate {
            from: "a1".into(),
            status: AgentStatus::Busy("coding".into()),
            current_task: Some("implement feature X".into()),
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "a1");
    }

    #[test]
    fn test_heartbeat_serde() {
        let msg = ProtocolMessage::Heartbeat {
            from: "a1".into(),
            timestamp: Utc::now(),
            health_score: 0.95,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        let de: ProtocolMessage = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.sender(), "a1");
    }

    #[test]
    fn test_to_bus_message_and_back() {
        let proto = ProtocolMessage::DirectMessage {
            from: "a".into(),
            to: "b".into(),
            content: "round-trip".into(),
            reply_to: None,
        };
        let bus_msg = proto.to_bus_message();
        assert_eq!(bus_msg.from_agent, "a");
        assert_eq!(bus_msg.to_agent.as_deref(), Some("b"));
        assert_eq!(bus_msg.message_type, MessageType::System);

        let decoded =
            ProtocolMessage::from_bus_message(&bus_msg).expect("operation should succeed");
        assert_eq!(decoded.sender(), "a");
    }

    #[test]
    fn test_from_bus_message_legacy() {
        let legacy = Message::direct("x", "y", "plain text, not protocol JSON");
        assert!(ProtocolMessage::from_bus_message(&legacy).is_none());
    }

    #[test]
    fn test_broadcast_scope_serde() {
        let scopes = vec![
            BroadcastScope::All,
            BroadcastScope::Team("alpha".into()),
            BroadcastScope::Capability("code".into()),
        ];
        for scope in scopes {
            let json = serde_json::to_string(&scope).expect("should serialize to JSON");
            let de: BroadcastScope =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(de, scope);
        }
    }

    #[test]
    fn test_work_status_serde() {
        let statuses = vec![
            WorkStatus::Accepted,
            WorkStatus::Rejected("no capacity".into()),
            WorkStatus::InProgress,
            WorkStatus::Completed("done".into()),
            WorkStatus::Failed("timeout".into()),
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).expect("should serialize to JSON");
            let de: WorkStatus = serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(de, status);
        }
    }

    #[tokio::test]
    async fn test_protocol_handler_send_direct() {
        let bus = Arc::new(MessageBus::new(32));
        let handler = ProtocolHandler::new(bus.clone());
        let mut rx = handler.subscribe();

        handler
            .send_direct("a", "b", "hello")
            .await
            .expect("async operation should succeed");

        let received = rx.recv().await.expect("async operation should succeed");
        let decoded =
            ProtocolMessage::from_bus_message(&received).expect("operation should succeed");
        assert_eq!(decoded.sender(), "a");
    }

    #[tokio::test]
    async fn test_protocol_handler_broadcast() {
        let bus = Arc::new(MessageBus::new(32));
        let handler = ProtocolHandler::new(bus.clone());
        let mut rx = handler.subscribe();

        handler
            .broadcast("a", "announcement", BroadcastScope::All)
            .await
            .expect("async operation should succeed");

        let received = rx.recv().await.expect("async operation should succeed");
        let decoded =
            ProtocolMessage::from_bus_message(&received).expect("operation should succeed");
        assert!(decoded.target().is_none());
    }

    #[tokio::test]
    async fn test_protocol_handler_work_roundtrip() {
        let bus = Arc::new(MessageBus::new(32));
        let handler = ProtocolHandler::new(bus.clone());
        let mut rx = handler.subscribe();

        handler
            .request_work("wr-1", "boss", "worker", "build it", 5)
            .await
            .expect("async operation should succeed");
        let _ = rx.recv().await.expect("async operation should succeed");

        handler
            .respond_work("worker", "wr-1", WorkStatus::Completed("built".into()))
            .await
            .expect("async operation should succeed");
        let resp = rx.recv().await.expect("async operation should succeed");
        let decoded = ProtocolMessage::from_bus_message(&resp).expect("operation should succeed");
        assert_eq!(decoded.sender(), "worker");
    }

    #[tokio::test]
    async fn test_protocol_handler_heartbeat() {
        let bus = Arc::new(MessageBus::new(32));
        let handler = ProtocolHandler::new(bus.clone());
        let mut rx = handler.subscribe();

        handler
            .send_heartbeat("a1", 0.95)
            .await
            .expect("async operation should succeed");

        let received = rx.recv().await.expect("async operation should succeed");
        let decoded =
            ProtocolMessage::from_bus_message(&received).expect("operation should succeed");
        assert_eq!(decoded.sender(), "a1");
    }
}
