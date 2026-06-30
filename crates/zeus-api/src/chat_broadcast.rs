//! Chat token broadcast channel
//!
//! Provides a `broadcast::Sender<StreamToken>` that mirrors the approval-surface
//! `ApprovalQueue::tx` pattern.  Any component that needs to observe the
//! agent's token stream (TUI, WebSocket clients, etc.) can subscribe.
//!
//! The broadcast is *best-effort* — slow consumers are dropped (tokio::sync::broadcast
//! behaviour) so that back-pressure never blocks the SSE response.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// A single token (or thinking segment) emitted by the LLM stream.
///
/// Deliberately minimal: consumers that need full OpenAI chunk shape can
/// reconstruct it from `text` + `is_thinking`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamToken {
    /// Token text (or thinking content when `is_thinking` is true).
    pub text: String,
    /// True when this token belongs to a `<think>` block.
    #[serde(default)]
    pub is_thinking: bool,
    /// Optional tab identifier so multi-tab TUI consumers can route.
    #[serde(default)]
    pub tab: Option<String>,
}

/// Broadcast handle for chat token streams.
///
/// Created once per `AppState` and cloned into handlers that emit tokens.
#[derive(Debug, Clone)]
pub struct ChatBroadcast {
    tx: broadcast::Sender<StreamToken>,
}

impl ChatBroadcast {
    /// Create a new broadcast channel with the given buffer size.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to the token stream.
    pub fn subscribe(&self) -> broadcast::Receiver<StreamToken> {
        self.tx.subscribe()
    }

    /// Send a token to all active subscribers.
    ///
    /// Errors are silently ignored (no subscribers = no-op).
    pub fn send(&self, token: StreamToken) {
        let _ = self.tx.send(token);
    }

    /// Return the number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for ChatBroadcast {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadcast_round_trip() {
        let bc = ChatBroadcast::new(16);
        let mut rx = bc.subscribe();

        bc.send(StreamToken {
            text: "hello".into(),
            is_thinking: false,
            tab: None,
        });

        let tok = rx.try_recv().expect("token received");
        assert_eq!(tok.text, "hello");
        assert!(!tok.is_thinking);
    }

    #[test]
    fn test_broadcast_thinking_flag() {
        let bc = ChatBroadcast::new(16);
        let mut rx = bc.subscribe();

        bc.send(StreamToken {
            text: "reasoning...".into(),
            is_thinking: true,
            tab: Some("chat".into()),
        });

        let tok = rx.try_recv().unwrap();
        assert!(tok.is_thinking);
        assert_eq!(tok.tab, Some("chat".into()));
    }

    #[test]
    fn test_broadcast_no_subscribers_is_no_op() {
        let bc = ChatBroadcast::new(16);
        // No subscribers — send should not panic
        bc.send(StreamToken {
            text: "orphan".into(),
            is_thinking: false,
            tab: None,
        });
        assert_eq!(bc.receiver_count(), 0);
    }
}
