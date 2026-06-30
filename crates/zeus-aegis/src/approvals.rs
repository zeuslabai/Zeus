//! Execution approval system for dangerous operations
//!
//! Provides an approval workflow for tool executions that match
//! dangerous patterns (e.g., `rm -rf`, `sudo`, `DROP TABLE`).
//! Pending approvals are tracked and can be approved or denied
//! via the TUI, API, or channel messages.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{debug, warn};

/// A pending approval request
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_name: String,
    pub command: Option<String>,
    pub args: Value,
    pub timestamp: DateTime<Utc>,
}

/// Result of an approval check (internal oneshot-based system)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResult {
    Approved,
    Denied(String),
    Timeout,
}

impl ApprovalResult {
    pub fn is_approved(&self) -> bool {
        matches!(self, ApprovalResult::Approved)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, ApprovalResult::Denied(_))
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self, ApprovalResult::Timeout)
    }
}

/// Outcome from the approval queue (external API-facing system).
///
/// Returned by `queue_for_approval()` after the request has been
/// resolved by the API approval endpoint, WebSocket, or timeout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ApprovalOutcome {
    /// Tool does not require approval; execute immediately.
    NotRequired,
    /// Approved by user/admin.
    Approved,
    /// Denied, with optional reason.
    Denied { reason: Option<String> },
    /// No response within timeout window.
    Expired,
}

impl ApprovalOutcome {
    pub fn is_approved(&self) -> bool {
        matches!(
            self,
            ApprovalOutcome::Approved | ApprovalOutcome::NotRequired
        )
    }
}

/// Data sent to the external approval queue for a tool execution.
#[derive(Debug, Clone, Serialize)]
pub struct QueuedApproval {
    pub id: String,
    pub tool_name: String,
    pub args: Value,
    pub agent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Channel type for forwarding approval requests to an external queue
/// (e.g., the API's `ApprovalQueue`).
///
/// The sender side lives in `ApprovalManager`; the receiver side is
/// consumed by the API layer which feeds into `ApprovalQueue::submit()`.
pub type ApprovalSink = mpsc::Sender<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>;

/// Manages execution approvals for dangerous operations
pub struct ApprovalManager {
    /// Pending approval requests: id -> sender channel
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalResult>>>>,
    /// Patterns that require approval (e.g., "rm ", "sudo", "DROP TABLE")
    patterns: Vec<String>,
    /// Tools that always require approval
    tools_requiring_approval: Vec<String>,
    /// Timeout for approval requests in seconds
    timeout_secs: u64,
    /// Optional sink to forward approvals to an external queue (API layer)
    sink: Option<ApprovalSink>,
}

impl ApprovalManager {
    pub fn new(patterns: Vec<String>, tools: Vec<String>) -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            patterns,
            tools_requiring_approval: tools,
            timeout_secs: 300, // 5 minutes default
            sink: None,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Connect an external approval sink (e.g., the API approval queue).
    ///
    /// When set, `queue_for_approval()` forwards requests through this
    /// channel instead of using the internal oneshot-based system.
    pub fn set_approval_sink(&mut self, sink: ApprovalSink) {
        self.sink = Some(sink);
    }

    /// Returns true if an external approval sink is connected.
    pub fn has_sink(&self) -> bool {
        self.sink.is_some()
    }

    /// Check if an operation needs approval
    pub fn needs_approval(&self, tool_name: &str, args: &Value) -> bool {
        // Check if tool always requires approval
        if self.tools_requiring_approval.iter().any(|t| t == tool_name) {
            return true;
        }

        // Check shell commands against patterns
        if tool_name == "shell"
            && let Some(cmd) = args.get("command").and_then(|v| v.as_str())
        {
            return self.patterns.iter().any(|p| cmd.contains(p));
        }

        false
    }

    /// Request approval (returns a future that resolves when approved/denied/timed out)
    pub async fn request_approval(&self, req: ApprovalRequest) -> ApprovalResult {
        let (tx, rx) = oneshot::channel();
        let id = req.id.clone();

        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), tx);
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(self.timeout_secs), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // Channel dropped - clean up
                self.pending.write().await.remove(&id);
                ApprovalResult::Denied("Approval request cancelled".to_string())
            }
            Err(_) => {
                // Timeout
                self.pending.write().await.remove(&id);
                ApprovalResult::Timeout
            }
        }
    }

    /// Approve a pending request
    pub async fn approve(&self, id: &str) -> Result<(), String> {
        let mut pending = self.pending.write().await;
        if let Some(tx) = pending.remove(id) {
            tx.send(ApprovalResult::Approved)
                .map_err(|_| "Failed to send approval".to_string())
        } else {
            Err(format!("No pending approval with id: {}", id))
        }
    }

    /// Deny a pending request
    pub async fn deny(&self, id: &str, reason: &str) -> Result<(), String> {
        let mut pending = self.pending.write().await;
        if let Some(tx) = pending.remove(id) {
            tx.send(ApprovalResult::Denied(reason.to_string()))
                .map_err(|_| "Failed to send denial".to_string())
        } else {
            Err(format!("No pending approval with id: {}", id))
        }
    }

    /// Get the count of pending approval requests
    pub async fn pending_count(&self) -> usize {
        self.pending.read().await.len()
    }

    /// Get the configured timeout in seconds
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// Get the patterns that trigger approval
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Get the tools that always require approval
    pub fn tools_requiring_approval(&self) -> &[String] {
        &self.tools_requiring_approval
    }

    /// Queue a tool call for approval if needed.
    ///
    /// This is the main entry point for the approval flow:
    /// 1. Checks if the tool/args combination requires approval
    /// 2. If no approval needed, returns `ApprovalOutcome::NotRequired`
    /// 3. If approval is needed and a sink is connected, forwards to the
    ///    external API approval queue and awaits the result
    /// 4. If no sink is connected, falls back to the internal oneshot system
    ///
    /// The caller should check `outcome.is_approved()` before executing.
    pub async fn queue_for_approval(
        &self,
        tool_name: &str,
        args: &Value,
        agent_id: Option<String>,
    ) -> ApprovalOutcome {
        if !self.needs_approval(tool_name, args) {
            return ApprovalOutcome::NotRequired;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(String::from);

        debug!(
            approval_id = %id,
            tool = %tool_name,
            command = ?command,
            "Tool requires approval, queuing"
        );

        let queued = QueuedApproval {
            id: id.clone(),
            tool_name: tool_name.to_string(),
            args: args.clone(),
            agent_id: agent_id.clone(),
            timestamp: Utc::now(),
        };

        if let Some(ref sink) = self.sink {
            // Forward to external approval queue (API layer)
            let (tx, rx) = oneshot::channel();

            if sink.send((queued, tx)).await.is_err() {
                warn!(approval_id = %id, "Approval sink disconnected");
                return ApprovalOutcome::Denied {
                    reason: Some("Approval queue disconnected".to_string()),
                };
            }

            debug!(approval_id = %id, "Waiting for external approval");

            match tokio::time::timeout(std::time::Duration::from_secs(self.timeout_secs), rx).await
            {
                Ok(Ok(outcome)) => {
                    debug!(approval_id = %id, ?outcome, "Approval resolved");
                    outcome
                }
                Ok(Err(_)) => {
                    warn!(approval_id = %id, "Approval channel closed");
                    ApprovalOutcome::Denied {
                        reason: Some("Approval channel closed".to_string()),
                    }
                }
                Err(_) => {
                    warn!(approval_id = %id, "Approval timed out");
                    ApprovalOutcome::Expired
                }
            }
        } else {
            // No external queue: fall back to internal oneshot-based system
            let req = ApprovalRequest {
                id,
                tool_name: tool_name.to_string(),
                command,
                args: args.clone(),
                timestamp: Utc::now(),
            };

            match self.request_approval(req).await {
                ApprovalResult::Approved => ApprovalOutcome::Approved,
                ApprovalResult::Denied(reason) => ApprovalOutcome::Denied {
                    reason: Some(reason),
                },
                ApprovalResult::Timeout => ApprovalOutcome::Expired,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_approval_detects_dangerous_patterns() {
        let mgr = ApprovalManager::new(
            vec![
                "rm -rf".to_string(),
                "sudo".to_string(),
                "DROP TABLE".to_string(),
                "mkfs".to_string(),
            ],
            vec![],
        );

        // Shell commands matching patterns
        assert!(mgr.needs_approval("shell", &serde_json::json!({"command": "rm -rf /tmp/foo"})));
        assert!(mgr.needs_approval(
            "shell",
            &serde_json::json!({"command": "sudo apt install foo"})
        ));
        assert!(mgr.needs_approval(
            "shell",
            &serde_json::json!({"command": "psql -c 'DROP TABLE users'"})
        ));

        // Safe commands should not need approval
        assert!(!mgr.needs_approval("shell", &serde_json::json!({"command": "ls -la"})));
        assert!(!mgr.needs_approval("shell", &serde_json::json!({"command": "echo hello"})));

        // Non-shell tools don't match patterns
        assert!(!mgr.needs_approval("read_file", &serde_json::json!({"path": "/etc/passwd"})));
    }

    #[test]
    fn test_needs_approval_detects_tools_requiring_approval() {
        let mgr = ApprovalManager::new(
            vec![],
            vec!["write_file".to_string(), "edit_file".to_string()],
        );

        // Tools that always require approval
        assert!(mgr.needs_approval(
            "write_file",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"})
        ));
        assert!(mgr.needs_approval("edit_file", &serde_json::json!({"path": "/tmp/test.txt"})));

        // Other tools don't need approval
        assert!(!mgr.needs_approval("read_file", &serde_json::json!({"path": "/tmp/test.txt"})));
        assert!(!mgr.needs_approval("list_dir", &serde_json::json!({"path": "."})));
    }

    #[tokio::test]
    async fn test_approve_deny_flow() {
        let mgr = Arc::new(ApprovalManager::new(vec!["rm".to_string()], vec![]).with_timeout(5));

        // Test approve flow
        let mgr_clone = mgr.clone();
        let handle = tokio::spawn(async move {
            let req = ApprovalRequest {
                id: "req-1".to_string(),
                tool_name: "shell".to_string(),
                command: Some("rm /tmp/foo".to_string()),
                args: serde_json::json!({"command": "rm /tmp/foo"}),
                timestamp: Utc::now(),
            };
            mgr_clone.request_approval(req).await
        });

        // Give the spawned task a moment to register the pending request
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.pending_count().await, 1);

        mgr.approve("req-1")
            .await
            .expect("async operation should succeed");
        let result = handle.await.expect("async operation should succeed");
        assert_eq!(result, ApprovalResult::Approved);
        assert_eq!(mgr.pending_count().await, 0);

        // Test deny flow
        let mgr_clone = mgr.clone();
        let handle = tokio::spawn(async move {
            let req = ApprovalRequest {
                id: "req-2".to_string(),
                tool_name: "shell".to_string(),
                command: Some("rm -rf /".to_string()),
                args: serde_json::json!({"command": "rm -rf /"}),
                timestamp: Utc::now(),
            };
            mgr_clone.request_approval(req).await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        mgr.deny("req-2", "Too dangerous")
            .await
            .expect("async operation should succeed");
        let result = handle.await.expect("async operation should succeed");
        assert_eq!(result, ApprovalResult::Denied("Too dangerous".to_string()));
        assert_eq!(mgr.pending_count().await, 0);
    }

    #[tokio::test]
    async fn test_timeout_behavior() {
        let mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(1); // 1 second timeout

        let req = ApprovalRequest {
            id: "req-timeout".to_string(),
            tool_name: "shell".to_string(),
            command: Some("echo hi".to_string()),
            args: serde_json::json!({"command": "echo hi"}),
            timestamp: Utc::now(),
        };

        // Nobody approves or denies, so it should time out
        let result = mgr.request_approval(req).await;
        assert_eq!(result, ApprovalResult::Timeout);
        assert_eq!(mgr.pending_count().await, 0);
    }

    #[tokio::test]
    async fn test_approve_nonexistent_request() {
        let mgr = ApprovalManager::new(vec![], vec![]);

        let result = mgr.approve("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending approval"));
    }

    #[tokio::test]
    async fn test_deny_nonexistent_request() {
        let mgr = ApprovalManager::new(vec![], vec![]);

        let result = mgr.deny("nonexistent", "reason").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending approval"));
    }

    #[test]
    fn test_shell_without_command_field() {
        let mgr = ApprovalManager::new(vec!["rm".to_string()], vec![]);

        // Shell tool with no "command" field in args should not need approval
        assert!(!mgr.needs_approval("shell", &serde_json::json!({})));
        assert!(!mgr.needs_approval("shell", &serde_json::json!({"other_field": "rm -rf /"})));
    }

    // ========================================================================
    // queue_for_approval tests
    // ========================================================================

    #[tokio::test]
    async fn test_queue_not_required_for_safe_tool() {
        let mgr = ApprovalManager::new(vec!["rm -rf".to_string()], vec!["shell".to_string()]);

        // read_file is not in the approval list
        let outcome = mgr
            .queue_for_approval("read_file", &serde_json::json!({"path": "/tmp/test"}), None)
            .await;
        assert_eq!(outcome, ApprovalOutcome::NotRequired);
        assert!(outcome.is_approved());
    }

    #[tokio::test]
    async fn test_queue_with_sink_approved() {
        let (sink_tx, mut sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        assert!(mgr.has_sink());

        // Spawn a task to auto-approve
        tokio::spawn(async move {
            if let Some((queued, tx)) = sink_rx.recv().await {
                assert_eq!(queued.tool_name, "shell");
                tx.send(ApprovalOutcome::Approved).ok();
            }
        });

        let outcome = mgr
            .queue_for_approval(
                "shell",
                &serde_json::json!({"command": "echo hello"}),
                Some("agent-1".to_string()),
            )
            .await;
        assert_eq!(outcome, ApprovalOutcome::Approved);
        assert!(outcome.is_approved());
    }

    #[tokio::test]
    async fn test_queue_with_sink_denied() {
        let (sink_tx, mut sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        // Spawn a task to deny
        tokio::spawn(async move {
            if let Some((_queued, tx)) = sink_rx.recv().await {
                tx.send(ApprovalOutcome::Denied {
                    reason: Some("too dangerous".to_string()),
                })
                .ok();
            }
        });

        let outcome = mgr
            .queue_for_approval("shell", &serde_json::json!({"command": "rm -rf /"}), None)
            .await;
        assert_eq!(
            outcome,
            ApprovalOutcome::Denied {
                reason: Some("too dangerous".to_string())
            }
        );
        assert!(!outcome.is_approved());
    }

    #[tokio::test]
    async fn test_queue_with_sink_timeout() {
        let (sink_tx, mut sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(1);
        mgr.set_approval_sink(sink_tx);

        // Receive but never respond — causes timeout
        tokio::spawn(async move {
            let _req = sink_rx.recv().await;
            // Hold the receiver open but never send a response
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        });

        let outcome = mgr
            .queue_for_approval("shell", &serde_json::json!({"command": "echo hi"}), None)
            .await;
        assert_eq!(outcome, ApprovalOutcome::Expired);
        assert!(!outcome.is_approved());
    }

    #[tokio::test]
    async fn test_queue_with_disconnected_sink() {
        let (sink_tx, sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        // Drop the receiver immediately — sink is disconnected
        drop(sink_rx);

        let outcome = mgr
            .queue_for_approval("shell", &serde_json::json!({"command": "echo hello"}), None)
            .await;
        assert!(
            matches!(outcome, ApprovalOutcome::Denied { reason: Some(r) } if r.contains("disconnected"))
        );
    }

    #[tokio::test]
    async fn test_queue_with_pattern_match_via_sink() {
        let (sink_tx, mut sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec!["sudo".to_string()], vec![]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        // Shell command containing "sudo" should trigger approval
        tokio::spawn(async move {
            if let Some((queued, tx)) = sink_rx.recv().await {
                assert_eq!(queued.tool_name, "shell");
                assert!(queued.args["command"].as_str().unwrap().contains("sudo"));
                tx.send(ApprovalOutcome::Approved).ok();
            }
        });

        let outcome = mgr
            .queue_for_approval(
                "shell",
                &serde_json::json!({"command": "sudo apt install foo"}),
                None,
            )
            .await;
        assert_eq!(outcome, ApprovalOutcome::Approved);
    }

    #[tokio::test]
    async fn test_queue_safe_shell_not_queued() {
        let (sink_tx, _sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec!["sudo".to_string()], vec![]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        // Shell command without "sudo" should not need approval
        let outcome = mgr
            .queue_for_approval("shell", &serde_json::json!({"command": "ls -la"}), None)
            .await;
        assert_eq!(outcome, ApprovalOutcome::NotRequired);
    }

    #[test]
    fn test_approval_outcome_serialization() {
        let approved = ApprovalOutcome::Approved;
        let json = serde_json::to_value(&approved).unwrap();
        assert_eq!(json["outcome"], "approved");

        let denied = ApprovalOutcome::Denied {
            reason: Some("bad".to_string()),
        };
        let json = serde_json::to_value(&denied).unwrap();
        assert_eq!(json["outcome"], "denied");
        assert_eq!(json["reason"], "bad");

        let not_required = ApprovalOutcome::NotRequired;
        let json = serde_json::to_value(&not_required).unwrap();
        assert_eq!(json["outcome"], "not_required");

        let expired = ApprovalOutcome::Expired;
        let json = serde_json::to_value(&expired).unwrap();
        assert_eq!(json["outcome"], "expired");
    }

    #[test]
    fn test_approval_outcome_deserialization() {
        let json = serde_json::json!({"outcome": "approved"});
        let outcome: ApprovalOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(outcome, ApprovalOutcome::Approved);

        let json = serde_json::json!({"outcome": "denied", "reason": "nope"});
        let outcome: ApprovalOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(
            outcome,
            ApprovalOutcome::Denied {
                reason: Some("nope".to_string())
            }
        );
    }

    #[test]
    fn test_approval_manager_accessors() {
        let mgr = ApprovalManager::new(vec!["rm -rf".to_string()], vec!["shell".to_string()])
            .with_timeout(600);

        assert_eq!(mgr.timeout_secs(), 600);
        assert_eq!(mgr.patterns(), &["rm -rf".to_string()]);
        assert_eq!(mgr.tools_requiring_approval(), &["shell".to_string()]);
        assert!(!mgr.has_sink());
    }

    #[tokio::test]
    async fn test_queue_carries_agent_id() {
        let (sink_tx, mut sink_rx) =
            mpsc::channel::<(QueuedApproval, oneshot::Sender<ApprovalOutcome>)>(16);

        let mut mgr = ApprovalManager::new(vec![], vec!["shell".to_string()]).with_timeout(5);
        mgr.set_approval_sink(sink_tx);

        tokio::spawn(async move {
            if let Some((queued, tx)) = sink_rx.recv().await {
                assert_eq!(queued.agent_id, Some("sub-agent-42".to_string()));
                assert!(!queued.id.is_empty());
                tx.send(ApprovalOutcome::Approved).ok();
            }
        });

        let outcome = mgr
            .queue_for_approval(
                "shell",
                &serde_json::json!({"command": "echo test"}),
                Some("sub-agent-42".to_string()),
            )
            .await;
        assert_eq!(outcome, ApprovalOutcome::Approved);
    }
}
