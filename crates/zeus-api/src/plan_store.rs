//! In-memory plan store for Prometheus plan persistence.
//!
//! Stores plans created via the `/v1/prometheus/plan` and `/v1/prometheus/execute`
//! endpoints so they can be retrieved by ID after creation. Uses a ring buffer
//! (bounded `VecDeque`) to cap memory at `max_capacity` entries, evicting oldest
//! plans when full.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Status of a stored plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStoreStatus {
    Created,
    Executing,
    Completed,
    Failed,
}

impl std::fmt::Display for PlanStoreStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Executing => write!(f, "executing"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A plan stored in the PlanStore with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPlan {
    /// Unique plan identifier (UUID).
    pub plan_id: String,
    /// Original goal/task description.
    pub goal: String,
    /// Current plan status.
    pub status: PlanStoreStatus,
    /// When the plan was created.
    pub created_at: DateTime<Utc>,
    /// When the status was last updated.
    pub updated_at: DateTime<Utc>,
    /// The full DAG analysis result (serialized).
    pub dag: Value,
    /// Number of nodes/steps in the plan.
    pub node_count: usize,
    /// Topological execution order.
    pub topological_order: Vec<usize>,
    /// Parallel execution groups.
    pub parallel_groups: Vec<Vec<usize>>,
    /// Critical path step IDs.
    pub critical_path: Vec<usize>,
    /// Estimated total execution time in milliseconds.
    pub estimated_total_ms: u64,
    /// Execution result (populated when completed/failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_result: Option<ExecutionResult>,
    /// Execution mode used (populated when execution starts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
}

/// Summary of plan execution results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Number of steps that completed successfully.
    pub steps_completed: usize,
    /// Number of steps that failed.
    pub steps_failed: usize,
    /// Total execution duration in milliseconds.
    pub duration_ms: u64,
    /// Final status string ("completed", "partial", "failed").
    pub final_status: String,
}

/// Concise plan summary for list responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    pub plan_id: String,
    pub goal: String,
    pub status: PlanStoreStatus,
    pub node_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&StoredPlan> for PlanSummary {
    fn from(plan: &StoredPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            goal: plan.goal.clone(),
            status: plan.status,
            node_count: plan.node_count,
            created_at: plan.created_at,
            updated_at: plan.updated_at,
        }
    }
}

/// Thread-safe in-memory store for Prometheus plans.
///
/// Uses a ring buffer to cap memory usage: when the store exceeds `max_capacity`,
/// the oldest plan is evicted. Access is synchronized via `std::sync::Mutex`.
#[derive(Clone)]
pub struct PlanStore {
    inner: Arc<Mutex<PlanStoreInner>>,
}

struct PlanStoreInner {
    /// Plans indexed by plan_id for O(1) lookup.
    plans: HashMap<String, StoredPlan>,
    /// Insertion order tracking for ring buffer eviction.
    order: VecDeque<String>,
    /// Maximum number of plans to retain.
    max_capacity: usize,
}

impl PlanStore {
    /// Create a new PlanStore with the given capacity limit.
    pub fn new(max_capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(PlanStoreInner {
                plans: HashMap::new(),
                order: VecDeque::new(),
                max_capacity,
            })),
        }
    }

    /// Store a new plan. If the store is at capacity, evicts the oldest plan.
    pub fn store(&self, plan: StoredPlan) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let plan_id = plan.plan_id.clone();

        // Evict oldest if at capacity
        while inner.order.len() >= inner.max_capacity {
            if let Some(oldest_id) = inner.order.pop_front() {
                inner.plans.remove(&oldest_id);
            }
        }

        inner.plans.insert(plan_id.clone(), plan);
        inner.order.push_back(plan_id);
    }

    /// Retrieve a plan by ID. Returns `None` if not found.
    pub fn get(&self, plan_id: &str) -> Option<StoredPlan> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.plans.get(plan_id).cloned()
    }

    /// List all stored plans as summaries, ordered by creation time (newest first).
    ///
    /// Supports pagination via `offset` and `limit`.
    pub fn list(&self, offset: usize, limit: usize) -> (Vec<PlanSummary>, usize) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let total = inner.order.len();

        // Iterate in reverse (newest first)
        let summaries: Vec<PlanSummary> = inner
            .order
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .filter_map(|id| inner.plans.get(id).map(PlanSummary::from))
            .collect();

        (summaries, total)
    }

    /// Update the status of an existing plan.
    pub fn update_status(&self, plan_id: &str, status: PlanStoreStatus) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(plan) = inner.plans.get_mut(plan_id) {
            plan.status = status;
            plan.updated_at = Utc::now();
        }
    }

    /// Set the execution result on a plan.
    pub fn set_execution_result(&self, plan_id: &str, result: ExecutionResult) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(plan) = inner.plans.get_mut(plan_id) {
            let final_status = result.final_status.clone();
            plan.execution_result = Some(result);
            plan.status = if final_status == "completed" {
                PlanStoreStatus::Completed
            } else {
                PlanStoreStatus::Failed
            };
            plan.updated_at = Utc::now();
        }
    }

    /// Set the execution mode on a plan.
    pub fn set_execution_mode(&self, plan_id: &str, mode: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(plan) = inner.plans.get_mut(plan_id) {
            plan.execution_mode = Some(mode.to_string());
        }
    }

    /// Return the number of stored plans.
    pub fn len(&self) -> usize {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.plans.len()
    }

    /// Returns true if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for PlanStore {
    fn default() -> Self {
        Self::new(100)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_stored_plan(id: &str, goal: &str) -> StoredPlan {
        let now = Utc::now();
        StoredPlan {
            plan_id: id.to_string(),
            goal: goal.to_string(),
            status: PlanStoreStatus::Created,
            created_at: now,
            updated_at: now,
            dag: json!({}),
            node_count: 2,
            topological_order: vec![1, 2],
            parallel_groups: vec![vec![1], vec![2]],
            critical_path: vec![1, 2],
            estimated_total_ms: 10000,
            execution_result: None,
            execution_mode: None,
        }
    }

    #[test]
    fn test_store_and_get() {
        let store = PlanStore::new(10);
        let plan = make_stored_plan("plan-1", "Deploy app");
        store.store(plan);

        let retrieved = store.get("plan-1").unwrap();
        assert_eq!(retrieved.plan_id, "plan-1");
        assert_eq!(retrieved.goal, "Deploy app");
        assert_eq!(retrieved.status, PlanStoreStatus::Created);
    }

    #[test]
    fn test_get_nonexistent() {
        let store = PlanStore::new(10);
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_update_status() {
        let store = PlanStore::new(10);
        store.store(make_stored_plan("plan-1", "Test"));

        store.update_status("plan-1", PlanStoreStatus::Executing);
        let plan = store.get("plan-1").unwrap();
        assert_eq!(plan.status, PlanStoreStatus::Executing);

        store.update_status("plan-1", PlanStoreStatus::Completed);
        let plan = store.get("plan-1").unwrap();
        assert_eq!(plan.status, PlanStoreStatus::Completed);
    }

    #[test]
    fn test_update_status_nonexistent() {
        let store = PlanStore::new(10);
        // Should not panic
        store.update_status("missing", PlanStoreStatus::Failed);
    }

    #[test]
    fn test_set_execution_result() {
        let store = PlanStore::new(10);
        store.store(make_stored_plan("plan-1", "Test"));

        store.set_execution_result(
            "plan-1",
            ExecutionResult {
                steps_completed: 3,
                steps_failed: 0,
                duration_ms: 5000,
                final_status: "completed".to_string(),
            },
        );

        let plan = store.get("plan-1").unwrap();
        assert_eq!(plan.status, PlanStoreStatus::Completed);
        let result = plan.execution_result.unwrap();
        assert_eq!(result.steps_completed, 3);
        assert_eq!(result.steps_failed, 0);
        assert_eq!(result.duration_ms, 5000);
    }

    #[test]
    fn test_set_execution_result_failed() {
        let store = PlanStore::new(10);
        store.store(make_stored_plan("plan-1", "Test"));

        store.set_execution_result(
            "plan-1",
            ExecutionResult {
                steps_completed: 1,
                steps_failed: 2,
                duration_ms: 3000,
                final_status: "failed".to_string(),
            },
        );

        let plan = store.get("plan-1").unwrap();
        assert_eq!(plan.status, PlanStoreStatus::Failed);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let store = PlanStore::new(3);

        store.store(make_stored_plan("plan-1", "First"));
        store.store(make_stored_plan("plan-2", "Second"));
        store.store(make_stored_plan("plan-3", "Third"));
        assert_eq!(store.len(), 3);

        // Adding a 4th should evict plan-1
        store.store(make_stored_plan("plan-4", "Fourth"));
        assert_eq!(store.len(), 3);
        assert!(store.get("plan-1").is_none());
        assert!(store.get("plan-2").is_some());
        assert!(store.get("plan-3").is_some());
        assert!(store.get("plan-4").is_some());
    }

    #[test]
    fn test_list_pagination() {
        let store = PlanStore::new(10);
        for i in 0..5 {
            store.store(make_stored_plan(&format!("plan-{i}"), &format!("Goal {i}")));
        }

        // Default: newest first
        let (plans, total) = store.list(0, 10);
        assert_eq!(total, 5);
        assert_eq!(plans.len(), 5);
        assert_eq!(plans[0].plan_id, "plan-4"); // newest first
        assert_eq!(plans[4].plan_id, "plan-0");

        // Pagination: skip 2, take 2
        let (plans, total) = store.list(2, 2);
        assert_eq!(total, 5);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].plan_id, "plan-2");
        assert_eq!(plans[1].plan_id, "plan-1");
    }

    #[test]
    fn test_list_empty() {
        let store = PlanStore::new(10);
        let (plans, total) = store.list(0, 10);
        assert_eq!(total, 0);
        assert!(plans.is_empty());
    }

    #[test]
    fn test_len_and_is_empty() {
        let store = PlanStore::new(10);
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        store.store(make_stored_plan("plan-1", "Test"));
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_default_capacity() {
        let store = PlanStore::default();
        // Default capacity is 100
        for i in 0..100 {
            store.store(make_stored_plan(&format!("plan-{i}"), "test"));
        }
        assert_eq!(store.len(), 100);

        store.store(make_stored_plan("plan-overflow", "test"));
        assert_eq!(store.len(), 100);
        assert!(store.get("plan-0").is_none());
        assert!(store.get("plan-overflow").is_some());
    }

    #[test]
    fn test_set_execution_mode() {
        let store = PlanStore::new(10);
        store.store(make_stored_plan("plan-1", "Test"));

        store.set_execution_mode("plan-1", "agent");
        let plan = store.get("plan-1").unwrap();
        assert_eq!(plan.execution_mode.as_deref(), Some("agent"));
    }

    #[test]
    fn test_plan_store_status_display() {
        assert_eq!(PlanStoreStatus::Created.to_string(), "created");
        assert_eq!(PlanStoreStatus::Executing.to_string(), "executing");
        assert_eq!(PlanStoreStatus::Completed.to_string(), "completed");
        assert_eq!(PlanStoreStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_plan_store_status_serialization() {
        let statuses = vec![
            PlanStoreStatus::Created,
            PlanStoreStatus::Executing,
            PlanStoreStatus::Completed,
            PlanStoreStatus::Failed,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deser: PlanStoreStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deser, status);
        }
    }

    #[test]
    fn test_stored_plan_serialization() {
        let plan = make_stored_plan("plan-1", "Test");
        let json = serde_json::to_string(&plan).unwrap();
        let deser: StoredPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.plan_id, "plan-1");
        assert_eq!(deser.goal, "Test");
        assert!(deser.execution_result.is_none());
    }

    #[test]
    fn test_plan_summary_from_stored_plan() {
        let plan = make_stored_plan("plan-1", "Deploy");
        let summary = PlanSummary::from(&plan);
        assert_eq!(summary.plan_id, "plan-1");
        assert_eq!(summary.goal, "Deploy");
        assert_eq!(summary.status, PlanStoreStatus::Created);
        assert_eq!(summary.node_count, 2);
    }

    #[test]
    fn test_clone_plan_store() {
        let store = PlanStore::new(10);
        store.store(make_stored_plan("plan-1", "Test"));

        let cloned = store.clone();
        assert_eq!(cloned.len(), 1);
        assert!(cloned.get("plan-1").is_some());

        // Modifications through one clone should be visible through the other
        store.update_status("plan-1", PlanStoreStatus::Executing);
        let plan = cloned.get("plan-1").unwrap();
        assert_eq!(plan.status, PlanStoreStatus::Executing);
    }
}
