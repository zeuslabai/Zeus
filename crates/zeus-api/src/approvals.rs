//! Exec Approval Manager
//!
//! API-facing approval queue for tool execution confirmation.
//! Tools listed in `require_confirmation_for` are held pending until
//! approved or denied via REST endpoints, with a configurable timeout
//! that auto-denies stale requests.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use tracing::debug;

use crate::SharedState;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied { reason: Option<String> },
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingApproval {
    pub id: String,
    pub tool_name: String,
    pub args: Value,
    pub agent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub status: ApprovalStatus,
}

/// Notification broadcast to WebSocket clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApprovalEvent {
    ApprovalPending { approval: PendingApproval },
    ApprovalResolved { id: String, status: ApprovalStatus },
}

// ============================================================================
// ApprovalQueue
// ============================================================================

pub struct ApprovalQueue {
    /// All approvals (pending + resolved, for history)
    approvals: Vec<PendingApproval>,
    /// Oneshot senders for blocking callers: id -> tx
    waiters: std::collections::HashMap<String, oneshot::Sender<ApprovalStatus>>,
    /// Broadcast channel for WebSocket notifications
    broadcast: broadcast::Sender<ApprovalEvent>,
    /// Tools that require confirmation
    require_confirmation_for: Vec<String>,
    /// Timeout duration in seconds
    timeout_secs: u64,
}

impl ApprovalQueue {
    pub fn new(require_confirmation_for: Vec<String>, timeout_secs: u64) -> Self {
        let (broadcast, _) = broadcast::channel(64);
        Self {
            approvals: Vec::new(),
            waiters: std::collections::HashMap::new(),
            broadcast,
            require_confirmation_for,
            timeout_secs,
        }
    }

    /// Check if a tool requires approval before execution.
    pub fn needs_approval(&self, tool_name: &str) -> bool {
        self.require_confirmation_for
            .iter()
            .any(|t| t == tool_name || t == "*")
    }

    /// Submit a new approval request. Returns the approval ID and a receiver
    /// that the caller can `.await` to block until resolved.
    pub fn submit(
        &mut self,
        tool_name: String,
        args: Value,
        agent_id: Option<String>,
    ) -> (String, oneshot::Receiver<ApprovalStatus>) {
        let id = uuid::Uuid::new_v4().to_string();
        let approval = PendingApproval {
            id: id.clone(),
            tool_name,
            args,
            agent_id,
            created_at: Utc::now(),
            status: ApprovalStatus::Pending,
        };

        let (tx, rx) = oneshot::channel();
        self.waiters.insert(id.clone(), tx);
        let _ = self.broadcast.send(ApprovalEvent::ApprovalPending {
            approval: approval.clone(),
        });
        self.approvals.push(approval);

        (id, rx)
    }

    /// Approve a pending request.
    pub fn approve(&mut self, id: &str) -> Result<(), String> {
        let entry = self
            .approvals
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("Approval '{}' not found", id))?;

        if entry.status != ApprovalStatus::Pending {
            return Err(format!("Approval '{}' is no longer pending", id));
        }

        entry.status = ApprovalStatus::Approved;

        if let Some(tx) = self.waiters.remove(id) {
            let _ = tx.send(ApprovalStatus::Approved);
        }
        let _ = self.broadcast.send(ApprovalEvent::ApprovalResolved {
            id: id.to_string(),
            status: ApprovalStatus::Approved,
        });

        Ok(())
    }

    /// Deny a pending request.
    pub fn deny(&mut self, id: &str, reason: Option<String>) -> Result<(), String> {
        let entry = self
            .approvals
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("Approval '{}' not found", id))?;

        if entry.status != ApprovalStatus::Pending {
            return Err(format!("Approval '{}' is no longer pending", id));
        }

        let status = ApprovalStatus::Denied {
            reason: reason.clone(),
        };
        entry.status = status.clone();

        if let Some(tx) = self.waiters.remove(id) {
            let _ = tx.send(status.clone());
        }
        let _ = self.broadcast.send(ApprovalEvent::ApprovalResolved {
            id: id.to_string(),
            status,
        });

        Ok(())
    }

    /// Return only pending approvals.
    pub fn list_pending(&self) -> Vec<PendingApproval> {
        self.approvals
            .iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .cloned()
            .collect()
    }

    /// Return all approvals (pending + resolved history).
    pub fn list_all(&self) -> Vec<PendingApproval> {
        self.approvals.clone()
    }

    /// Expire approvals that have been pending longer than `timeout_secs`.
    pub fn expire_stale(&mut self) {
        let now = Utc::now();
        let timeout = chrono::Duration::seconds(self.timeout_secs as i64);

        let stale_ids: Vec<String> = self
            .approvals
            .iter()
            .filter(|a| a.status == ApprovalStatus::Pending && now - a.created_at > timeout)
            .map(|a| a.id.clone())
            .collect();

        for id in stale_ids {
            if let Some(entry) = self.approvals.iter_mut().find(|a| a.id == id) {
                entry.status = ApprovalStatus::Expired;
                debug!(id = %entry.id, tool = %entry.tool_name, "Approval expired");
            }
            if let Some(tx) = self.waiters.remove(&id) {
                let _ = tx.send(ApprovalStatus::Expired);
            }
            let _ = self.broadcast.send(ApprovalEvent::ApprovalResolved {
                id: id.clone(),
                status: ApprovalStatus::Expired,
            });
        }
    }

    /// Subscribe to approval events (for WebSocket clients).
    pub fn subscribe(&self) -> broadcast::Receiver<ApprovalEvent> {
        self.broadcast.subscribe()
    }
}

// ============================================================================
// Background expiry task
// ============================================================================

/// Spawn a background task that expires stale approvals every 60 seconds.
pub fn start_expiry_task(state: SharedState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            state.write().await.approvals.expire_stale();
        }
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_approval_queue_submit_and_list() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let (id, _rx) = q.submit("shell".into(), json!({"command": "rm -rf /"}), None);
        assert!(!id.is_empty());

        let pending = q.list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].tool_name, "shell");
        assert_eq!(pending[0].status, ApprovalStatus::Pending);
    }

    #[test]
    fn test_approval_queue_approve() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let (id, mut rx) = q.submit("shell".into(), json!({}), None);

        q.approve(&id).unwrap();

        assert!(q.list_pending().is_empty());
        assert_eq!(q.list_all()[0].status, ApprovalStatus::Approved);
        assert_eq!(rx.try_recv().unwrap(), ApprovalStatus::Approved);
    }

    #[test]
    fn test_approval_queue_deny() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let (id, mut rx) = q.submit("shell".into(), json!({}), None);

        q.deny(&id, None).unwrap();

        assert!(q.list_pending().is_empty());
        let status = &q.list_all()[0].status;
        assert!(matches!(status, ApprovalStatus::Denied { reason: None }));
        assert!(matches!(
            rx.try_recv().unwrap(),
            ApprovalStatus::Denied { reason: None }
        ));
    }

    #[test]
    fn test_approval_queue_deny_with_reason() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let (id, mut rx) = q.submit("shell".into(), json!({}), None);

        q.deny(&id, Some("too dangerous".into())).unwrap();

        let status = &q.list_all()[0].status;
        match status {
            ApprovalStatus::Denied { reason } => {
                assert_eq!(reason.as_deref(), Some("too dangerous"));
            }
            _ => panic!("Expected Denied"),
        }
        match rx.try_recv().unwrap() {
            ApprovalStatus::Denied { reason } => {
                assert_eq!(reason.as_deref(), Some("too dangerous"));
            }
            _ => panic!("Expected Denied"),
        }
    }

    #[test]
    fn test_approval_queue_needs_approval() {
        let q = ApprovalQueue::new(vec!["shell".into(), "write_file".into()], 1800);
        assert!(q.needs_approval("shell"));
        assert!(q.needs_approval("write_file"));
        assert!(!q.needs_approval("read_file"));
    }

    #[test]
    fn test_approval_queue_needs_approval_wildcard() {
        let q = ApprovalQueue::new(vec!["*".into()], 1800);
        assert!(q.needs_approval("shell"));
        assert!(q.needs_approval("read_file"));
    }

    #[test]
    fn test_approval_queue_expire_stale() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 0); // 0 second timeout
        let (_id, mut rx) = q.submit("shell".into(), json!({}), None);

        // With 0-second timeout, everything is immediately stale
        q.expire_stale();

        assert!(q.list_pending().is_empty());
        assert_eq!(q.list_all()[0].status, ApprovalStatus::Expired);
        assert_eq!(rx.try_recv().unwrap(), ApprovalStatus::Expired);
    }

    #[test]
    fn test_approval_queue_broadcast() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let mut rx = q.subscribe();

        let (_id, _) = q.submit("shell".into(), json!({}), None);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, ApprovalEvent::ApprovalPending { .. }));
    }

    #[test]
    fn test_approval_nonexistent_id() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        assert!(q.approve("nonexistent").is_err());
        assert!(q.deny("nonexistent", None).is_err());
    }

    #[test]
    fn test_approval_config_defaults() {
        let q = ApprovalQueue::new(vec![], 1800);
        assert!(!q.needs_approval("shell"));
        assert!(q.list_pending().is_empty());
        assert!(q.list_all().is_empty());
    }

    #[test]
    fn test_approval_double_approve_fails() {
        let mut q = ApprovalQueue::new(vec!["shell".into()], 1800);
        let (id, _rx) = q.submit("shell".into(), json!({}), None);
        q.approve(&id).unwrap();
        assert!(q.approve(&id).is_err());
    }

    #[test]
    fn test_approval_status_serialization() {
        let status = ApprovalStatus::Denied {
            reason: Some("bad".into()),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["status"], "denied");
        assert_eq!(json["reason"], "bad");
    }
}
