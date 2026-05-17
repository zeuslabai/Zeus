//! Command authorization flow for exec approvals.
//!
//! Provides an approval gate that sits in front of tool execution.
//! Policies define which tools require approval and which command patterns
//! are auto-approved (e.g. read-only shell commands like `ls`, `cat`).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::SandboxError;

/// Maximum number of approval requests stored in the ring buffer.
const MAX_REQUESTS: usize = 500;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An approval policy that defines which tools require human approval
/// and which command patterns can be auto-approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Unique identifier (UUID).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Tool names that require approval (e.g. `["shell", "write_file"]`).
    pub require_approval_for: Vec<String>,
    /// Regex patterns for commands that are auto-approved
    /// (e.g. `["^ls ", "^cat "]`). Matched against the JSON-serialized
    /// arguments.
    pub auto_approve_patterns: Vec<String>,
    /// Seconds before a pending request expires (default 1800 = 30 min).
    pub timeout_secs: u64,
    /// If `true`, deny the request when it times out. If `false` (default),
    /// it simply expires without an explicit deny.
    pub deny_by_default: bool,
    /// When this policy was created.
    pub created_at: DateTime<Utc>,
}

impl ApprovalPolicy {
    /// Create a new policy with the given name and tools that require approval.
    pub fn new(name: impl Into<String>, require_approval_for: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            require_approval_for,
            auto_approve_patterns: Vec::new(),
            timeout_secs: 1800,
            deny_by_default: false,
            created_at: Utc::now(),
        }
    }
}

/// A pending (or resolved) approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Unique identifier (UUID).
    pub id: String,
    /// The tool that was requested.
    pub tool_name: String,
    /// The arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// The policy that triggered the approval (if any).
    pub policy_id: Option<String>,
    /// When the request was created.
    pub requested_at: DateTime<Utc>,
    /// Current status.
    pub status: ApprovalStatus,
    /// When the request was resolved (approved/denied/expired).
    pub resolved_at: Option<DateTime<Utc>>,
    /// Who resolved the request (e.g. username or "system").
    pub resolved_by: Option<String>,
    /// Optional reason (e.g. why it was denied).
    pub reason: Option<String>,
    /// When this request expires if still pending.
    pub expires_at: DateTime<Utc>,
}

/// Status of an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    AutoApproved,
}

/// The outcome of a `check_approval` call.
#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    /// Whether execution is allowed.
    pub approved: bool,
    /// Optional explanation.
    pub reason: Option<String>,
    /// `true` if this was auto-approved (matched an auto-approve pattern).
    pub auto: bool,
}

// ---------------------------------------------------------------------------
// ExecApprovalManager
// ---------------------------------------------------------------------------

/// Manages approval policies and pending approval requests.
///
/// Requests are stored in a ring buffer capped at [`MAX_REQUESTS`].
pub struct ExecApprovalManager {
    policies: Arc<Mutex<HashMap<String, ApprovalPolicy>>>,
    requests: Arc<Mutex<Vec<ApprovalRequest>>>,
    default_timeout_secs: u64,
}

impl ExecApprovalManager {
    /// Create a new manager with the given default timeout for requests
    /// that are not covered by a specific policy timeout.
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            policies: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            default_timeout_secs,
        }
    }

    // -- Policy management --------------------------------------------------

    /// Add a policy. Returns error if a policy with the same ID already exists.
    pub async fn add_policy(&self, policy: ApprovalPolicy) -> Result<(), SandboxError> {
        let mut policies = self.policies.lock().await;
        if policies.contains_key(&policy.id) {
            return Err(SandboxError::PolicyAlreadyExists(policy.id.clone()));
        }
        policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    /// Get a policy by ID.
    pub async fn get_policy(&self, id: &str) -> Option<ApprovalPolicy> {
        self.policies.lock().await.get(id).cloned()
    }

    /// List all policies.
    pub async fn list_policies(&self) -> Vec<ApprovalPolicy> {
        self.policies.lock().await.values().cloned().collect()
    }

    /// Delete a policy by ID. Returns error if not found.
    pub async fn delete_policy(&self, id: &str) -> Result<(), SandboxError> {
        let mut policies = self.policies.lock().await;
        policies
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| SandboxError::PolicyNotFound(id.to_string()))
    }

    // -- Approval checking --------------------------------------------------

    /// Check whether a tool invocation requires approval.
    ///
    /// Returns an [`ApprovalOutcome`] immediately:
    /// - If no policy covers this tool, returns approved.
    /// - If a policy covers the tool but the arguments match an
    ///   auto-approve pattern, returns auto-approved.
    /// - Otherwise returns not-approved (caller should create a request).
    pub async fn check_approval(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> ApprovalOutcome {
        let policies = self.policies.lock().await;

        // Find the first policy that requires approval for this tool.
        let matching_policy = policies
            .values()
            .find(|p| p.require_approval_for.iter().any(|t| t == tool_name));

        let policy = match matching_policy {
            Some(p) => p,
            None => {
                // No policy covers this tool -- auto-allow.
                return ApprovalOutcome {
                    approved: true,
                    reason: Some("no policy requires approval for this tool".to_string()),
                    auto: false,
                };
            }
        };

        // Check auto-approve patterns against stringified arguments.
        let args_str = arguments.to_string();
        for pattern_str in &policy.auto_approve_patterns {
            if let Ok(re) = Regex::new(pattern_str)
                && re.is_match(&args_str)
            {
                return ApprovalOutcome {
                    approved: true,
                    reason: Some(format!("auto-approved by pattern: {pattern_str}")),
                    auto: true,
                };
            }
        }

        // Tool requires approval and no auto-approve pattern matched.
        ApprovalOutcome {
            approved: false,
            reason: Some(format!(
                "tool '{}' requires approval per policy '{}'",
                tool_name, policy.name
            )),
            auto: false,
        }
    }

    // -- Request lifecycle --------------------------------------------------

    /// Create a new pending approval request and store it.
    ///
    /// The ring buffer drops the oldest entry when it exceeds [`MAX_REQUESTS`].
    pub async fn request_approval(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        policy_id: Option<String>,
    ) -> ApprovalRequest {
        let now = Utc::now();

        // Determine timeout from the policy or fall back to default.
        let timeout_secs = if let Some(ref pid) = policy_id {
            let policies = self.policies.lock().await;
            policies
                .get(pid)
                .map(|p| p.timeout_secs)
                .unwrap_or(self.default_timeout_secs)
        } else {
            self.default_timeout_secs
        };

        let request = ApprovalRequest {
            id: Uuid::new_v4().to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            policy_id,
            requested_at: now,
            status: ApprovalStatus::Pending,
            resolved_at: None,
            resolved_by: None,
            reason: None,
            expires_at: now + Duration::seconds(timeout_secs as i64),
        };

        let mut requests = self.requests.lock().await;
        requests.push(request.clone());

        // Ring buffer: drop oldest when over capacity.
        if requests.len() > MAX_REQUESTS {
            let excess = requests.len() - MAX_REQUESTS;
            requests.drain(..excess);
        }

        request
    }

    /// Approve a pending request. Returns the updated request.
    pub async fn approve(
        &self,
        request_id: &str,
        by: Option<String>,
    ) -> Result<ApprovalRequest, SandboxError> {
        let mut requests = self.requests.lock().await;
        let req = requests
            .iter_mut()
            .find(|r| r.id == request_id)
            .ok_or_else(|| SandboxError::RequestNotFound(request_id.to_string()))?;

        req.status = ApprovalStatus::Approved;
        req.resolved_at = Some(Utc::now());
        req.resolved_by = by;

        Ok(req.clone())
    }

    /// Deny a pending request with an optional reason.
    pub async fn deny(
        &self,
        request_id: &str,
        reason: Option<String>,
        by: Option<String>,
    ) -> Result<ApprovalRequest, SandboxError> {
        let mut requests = self.requests.lock().await;
        let req = requests
            .iter_mut()
            .find(|r| r.id == request_id)
            .ok_or_else(|| SandboxError::RequestNotFound(request_id.to_string()))?;

        req.status = ApprovalStatus::Denied;
        req.resolved_at = Some(Utc::now());
        req.resolved_by = by;
        req.reason = reason;

        Ok(req.clone())
    }

    /// Get a single request by ID.
    pub async fn get_request(&self, id: &str) -> Option<ApprovalRequest> {
        self.requests
            .lock()
            .await
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// List only pending, non-expired requests.
    pub async fn list_pending(&self) -> Vec<ApprovalRequest> {
        let now = Utc::now();
        self.requests
            .lock()
            .await
            .iter()
            .filter(|r| r.status == ApprovalStatus::Pending && r.expires_at > now)
            .cloned()
            .collect()
    }

    /// Mark all pending requests whose `expires_at` is in the past as
    /// [`ApprovalStatus::Expired`].
    pub async fn expire_stale(&self) {
        let now = Utc::now();
        let mut requests = self.requests.lock().await;
        for req in requests.iter_mut() {
            if req.status == ApprovalStatus::Pending && req.expires_at <= now {
                req.status = ApprovalStatus::Expired;
                req.resolved_at = Some(now);
            }
        }
    }

    // -- Counts -------------------------------------------------------------

    /// Total number of stored requests (all statuses).
    pub async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }

    /// Total number of policies.
    pub async fn policy_count(&self) -> usize {
        self.policies.lock().await.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- ExecApprovalManager creation ---------------------------------------

    #[tokio::test]
    async fn test_manager_creation() {
        let mgr = ExecApprovalManager::new(1800);
        assert_eq!(mgr.policy_count().await, 0);
        assert_eq!(mgr.request_count().await, 0);
    }

    #[tokio::test]
    async fn test_manager_custom_timeout() {
        let mgr = ExecApprovalManager::new(60);
        assert_eq!(mgr.default_timeout_secs, 60);
    }

    // -- Policy CRUD --------------------------------------------------------

    #[tokio::test]
    async fn test_add_policy() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("strict", vec!["shell".to_string()]);
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");
        assert_eq!(mgr.policy_count().await, 1);
    }

    #[tokio::test]
    async fn test_get_policy() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("strict", vec!["shell".to_string()]);
        let id = policy.id.clone();
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        let retrieved = mgr
            .get_policy(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.name, "strict");
        assert_eq!(retrieved.require_approval_for, vec!["shell"]);
    }

    #[tokio::test]
    async fn test_get_policy_missing() {
        let mgr = ExecApprovalManager::new(1800);
        assert!(mgr.get_policy("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_list_policies() {
        let mgr = ExecApprovalManager::new(1800);
        mgr.add_policy(ApprovalPolicy::new("a", vec!["shell".to_string()]))
            .await
            .expect("should serialize");
        mgr.add_policy(ApprovalPolicy::new("b", vec!["write_file".to_string()]))
            .await
            .expect("should serialize");
        let policies = mgr.list_policies().await;
        assert_eq!(policies.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_policy() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("doomed", vec![]);
        let id = policy.id.clone();
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");
        assert_eq!(mgr.policy_count().await, 1);

        mgr.delete_policy(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(mgr.policy_count().await, 0);
    }

    #[tokio::test]
    async fn test_delete_policy_not_found() {
        let mgr = ExecApprovalManager::new(1800);
        let err = mgr.delete_policy("ghost").await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_duplicate_policy_detection() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("dup", vec!["shell".to_string()]);
        let dup = policy.clone();
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");
        let err = mgr.add_policy(dup).await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyAlreadyExists(_)));
    }

    // -- request_approval ---------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_creates_pending() {
        let mgr = ExecApprovalManager::new(1800);
        let req = mgr
            .request_approval("shell", json!({"command": "rm -rf /"}), None)
            .await;
        assert_eq!(req.status, ApprovalStatus::Pending);
        assert_eq!(req.tool_name, "shell");
        assert!(req.resolved_at.is_none());
        assert_eq!(mgr.request_count().await, 1);
    }

    #[tokio::test]
    async fn test_request_approval_uses_policy_timeout() {
        let mgr = ExecApprovalManager::new(1800);
        let mut policy = ApprovalPolicy::new("fast", vec!["shell".to_string()]);
        policy.timeout_secs = 60;
        let pid = policy.id.clone();
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        let req = mgr.request_approval("shell", json!({}), Some(pid)).await;
        // The expiration should be ~60s from now, not 1800s.
        let diff = req.expires_at - req.requested_at;
        assert_eq!(diff.num_seconds(), 60);
    }

    // -- approve / deny -----------------------------------------------------

    #[tokio::test]
    async fn test_approve_transitions_to_approved() {
        let mgr = ExecApprovalManager::new(1800);
        let req = mgr
            .request_approval("shell", json!({"cmd": "echo hi"}), None)
            .await;
        let approved = mgr
            .approve(&req.id, Some("admin".to_string()))
            .await
            .expect("should serialize");
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert!(approved.resolved_at.is_some());
        assert_eq!(approved.resolved_by.as_deref(), Some("admin"));
    }

    #[tokio::test]
    async fn test_deny_transitions_to_denied_with_reason() {
        let mgr = ExecApprovalManager::new(1800);
        let req = mgr
            .request_approval("write_file", json!({"path": "/etc/passwd"}), None)
            .await;
        let denied = mgr
            .deny(
                &req.id,
                Some("too dangerous".to_string()),
                Some("admin".to_string()),
            )
            .await
            .expect("async operation should succeed");
        assert_eq!(denied.status, ApprovalStatus::Denied);
        assert_eq!(denied.reason.as_deref(), Some("too dangerous"));
        assert_eq!(denied.resolved_by.as_deref(), Some("admin"));
    }

    #[tokio::test]
    async fn test_approve_nonexistent_request() {
        let mgr = ExecApprovalManager::new(1800);
        let err = mgr.approve("fake-id", None).await.unwrap_err();
        assert!(matches!(err, SandboxError::RequestNotFound(_)));
    }

    #[tokio::test]
    async fn test_deny_nonexistent_request() {
        let mgr = ExecApprovalManager::new(1800);
        let err = mgr.deny("fake-id", None, None).await.unwrap_err();
        assert!(matches!(err, SandboxError::RequestNotFound(_)));
    }

    // -- check_approval -----------------------------------------------------

    #[tokio::test]
    async fn test_check_approval_no_policy_returns_approved() {
        let mgr = ExecApprovalManager::new(1800);
        let outcome = mgr
            .check_approval("shell", &json!({"command": "ls -la"}))
            .await;
        assert!(outcome.approved);
        assert!(!outcome.auto);
    }

    #[tokio::test]
    async fn test_check_approval_tool_requires_approval() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("strict", vec!["shell".to_string()]);
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        let outcome = mgr
            .check_approval("shell", &json!({"command": "rm -rf /"}))
            .await;
        assert!(!outcome.approved);
        assert!(!outcome.auto);
    }

    #[tokio::test]
    async fn test_check_approval_auto_approve_pattern() {
        let mgr = ExecApprovalManager::new(1800);
        let mut policy = ApprovalPolicy::new("semi-strict", vec!["shell".to_string()]);
        policy.auto_approve_patterns = vec!["ls ".to_string(), "cat ".to_string()];
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        let outcome = mgr
            .check_approval("shell", &json!({"command": "ls -la /tmp"}))
            .await;
        assert!(outcome.approved);
        assert!(outcome.auto);
    }

    #[tokio::test]
    async fn test_check_approval_auto_approve_no_match() {
        let mgr = ExecApprovalManager::new(1800);
        let mut policy = ApprovalPolicy::new("semi-strict", vec!["shell".to_string()]);
        policy.auto_approve_patterns = vec!["^ls ".to_string()];
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        let outcome = mgr
            .check_approval("shell", &json!({"command": "rm -rf /"}))
            .await;
        assert!(!outcome.approved);
    }

    #[tokio::test]
    async fn test_check_approval_uncovered_tool() {
        let mgr = ExecApprovalManager::new(1800);
        let policy = ApprovalPolicy::new("only-shell", vec!["shell".to_string()]);
        mgr.add_policy(policy)
            .await
            .expect("async operation should succeed");

        // read_file is not covered by the policy -> approved.
        let outcome = mgr
            .check_approval("read_file", &json!({"path": "/etc/hosts"}))
            .await;
        assert!(outcome.approved);
    }

    // -- list_pending -------------------------------------------------------

    #[tokio::test]
    async fn test_list_pending_filters_correctly() {
        let mgr = ExecApprovalManager::new(1800);

        // Create two requests, approve one.
        let r1 = mgr.request_approval("shell", json!({"a": 1}), None).await;
        let _r2 = mgr.request_approval("shell", json!({"a": 2}), None).await;
        mgr.approve(&r1.id, None)
            .await
            .expect("async operation should succeed");

        let pending = mgr.list_pending().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].arguments, json!({"a": 2}));
    }

    // -- expire_stale -------------------------------------------------------

    #[tokio::test]
    async fn test_expire_stale_marks_old_requests() {
        let mgr = ExecApprovalManager::new(1800);

        // Create a request, then manually set its expiry in the past.
        let req = mgr.request_approval("shell", json!({}), None).await;

        {
            let mut requests = mgr.requests.lock().await;
            let r = requests
                .iter_mut()
                .find(|r| r.id == req.id)
                .expect("find should succeed");
            r.expires_at = Utc::now() - Duration::seconds(1);
        }

        mgr.expire_stale().await;

        let updated = mgr
            .get_request(&req.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(updated.status, ApprovalStatus::Expired);
        assert!(updated.resolved_at.is_some());
    }

    #[tokio::test]
    async fn test_expire_stale_does_not_touch_resolved() {
        let mgr = ExecApprovalManager::new(1800);
        let req = mgr.request_approval("shell", json!({}), None).await;
        mgr.approve(&req.id, None)
            .await
            .expect("async operation should succeed");

        // Even if we set the expiry in the past, it should stay Approved.
        {
            let mut requests = mgr.requests.lock().await;
            let r = requests
                .iter_mut()
                .find(|r| r.id == req.id)
                .expect("find should succeed");
            r.expires_at = Utc::now() - Duration::seconds(1);
        }

        mgr.expire_stale().await;

        let updated = mgr
            .get_request(&req.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(updated.status, ApprovalStatus::Approved);
    }

    // -- Serialization ------------------------------------------------------

    #[test]
    fn test_approval_status_serialization() {
        let statuses = [
            (ApprovalStatus::Pending, "\"pending\""),
            (ApprovalStatus::Approved, "\"approved\""),
            (ApprovalStatus::Denied, "\"denied\""),
            (ApprovalStatus::Expired, "\"expired\""),
            (ApprovalStatus::AutoApproved, "\"auto_approved\""),
        ];
        for (status, expected) in &statuses {
            let json = serde_json::to_string(status).expect("should serialize to JSON");
            assert_eq!(&json, expected);
            let de: ApprovalStatus =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(&de, status);
        }
    }

    #[test]
    fn test_approval_policy_serialization() {
        let mut policy = ApprovalPolicy::new("test", vec!["shell".to_string()]);
        policy.auto_approve_patterns = vec!["^ls ".to_string()];
        policy.timeout_secs = 300;
        policy.deny_by_default = true;

        let json = serde_json::to_string(&policy).expect("should serialize to JSON");
        let de: ApprovalPolicy = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "test");
        assert_eq!(de.require_approval_for, vec!["shell"]);
        assert_eq!(de.auto_approve_patterns, vec!["^ls "]);
        assert_eq!(de.timeout_secs, 300);
        assert!(de.deny_by_default);
        assert_eq!(de.id, policy.id);
    }

    #[test]
    fn test_approval_request_serialization() {
        let now = Utc::now();
        let req = ApprovalRequest {
            id: "req-1".to_string(),
            tool_name: "shell".to_string(),
            arguments: json!({"command": "ls"}),
            policy_id: Some("pol-1".to_string()),
            requested_at: now,
            status: ApprovalStatus::Pending,
            resolved_at: None,
            resolved_by: None,
            reason: None,
            expires_at: now + Duration::seconds(1800),
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: ApprovalRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "req-1");
        assert_eq!(de.tool_name, "shell");
        assert_eq!(de.status, ApprovalStatus::Pending);
        assert_eq!(de.policy_id.as_deref(), Some("pol-1"));
    }

    // -- Counts -------------------------------------------------------------

    #[tokio::test]
    async fn test_request_count_tracking() {
        let mgr = ExecApprovalManager::new(1800);
        assert_eq!(mgr.request_count().await, 0);

        mgr.request_approval("shell", json!({}), None).await;
        mgr.request_approval("write_file", json!({}), None).await;
        assert_eq!(mgr.request_count().await, 2);
    }

    #[tokio::test]
    async fn test_policy_count_tracking() {
        let mgr = ExecApprovalManager::new(1800);
        assert_eq!(mgr.policy_count().await, 0);

        mgr.add_policy(ApprovalPolicy::new("a", vec![]))
            .await
            .expect("ApprovalPolicy::new should succeed");
        mgr.add_policy(ApprovalPolicy::new("b", vec![]))
            .await
            .expect("ApprovalPolicy::new should succeed");
        assert_eq!(mgr.policy_count().await, 2);
    }

    // -- Ring buffer --------------------------------------------------------

    #[tokio::test]
    async fn test_ring_buffer_limit() {
        let mgr = ExecApprovalManager::new(1800);

        // Insert MAX_REQUESTS + 10 requests.
        for i in 0..(MAX_REQUESTS + 10) {
            mgr.request_approval("shell", json!({"i": i}), None).await;
        }

        // Should be capped at MAX_REQUESTS.
        assert_eq!(mgr.request_count().await, MAX_REQUESTS);

        // Oldest requests should have been dropped.
        // The first surviving request should have i == 10.
        let requests = mgr.requests.lock().await;
        let first_i = requests[0].arguments["i"]
            .as_u64()
            .expect("should be a number");
        assert_eq!(first_i, 10);
    }

    // -- get_request --------------------------------------------------------

    #[tokio::test]
    async fn test_get_request() {
        let mgr = ExecApprovalManager::new(1800);
        let req = mgr
            .request_approval("shell", json!({"cmd": "whoami"}), None)
            .await;
        let retrieved = mgr
            .get_request(&req.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.id, req.id);
        assert_eq!(retrieved.tool_name, "shell");
    }

    #[tokio::test]
    async fn test_get_request_missing() {
        let mgr = ExecApprovalManager::new(1800);
        assert!(mgr.get_request("no-such-id").await.is_none());
    }
}
