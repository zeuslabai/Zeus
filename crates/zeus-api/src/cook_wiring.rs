//! Phase 3e — wiring adapters that plug the gateway's `TaskStore` and
//! `zeus-prometheus::acceptance` runner into `zeus-agent::cook`'s trait
//! surface (`TaskPersistence`, `AcceptanceChecker`).
//!
//! Kept in `zeus-api` (not `zeus-agent`) to avoid a circular dependency:
//! `zeus-api → zeus-agent` already exists, so the adapters live on the
//! downstream side.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use zeus_agent::cook::{
    AcceptanceChecker, AcceptanceResult, TaskOutcome, TaskPersistence, TaskRecord,
};
use zeus_prometheus::acceptance::{AcceptanceCheck, run_all};

use crate::handlers::task_store::{AgentTask, TaskStatus, TaskStore};

// ============================================================================
// TaskPersistence adapter
// ============================================================================

/// Adapter that implements `zeus_agent::cook::TaskPersistence` on top of
/// the gateway's SQLite-backed `TaskStore`.
pub struct TaskStorePersistence {
    store: TaskStore,
}

impl TaskStorePersistence {
    pub fn new(store: TaskStore) -> Self {
        Self { store }
    }

    pub fn arc(store: TaskStore) -> Arc<dyn TaskPersistence> {
        Arc::new(Self::new(store))
    }
}

fn agent_task_to_record(t: &AgentTask) -> TaskRecord {
    TaskRecord {
        id: t.id.clone(),
        agent_id: t.agent_id.clone(),
        description: t.description.clone(),
        scope_json: t.scope_json.clone(),
        iterations_used: t.iterations_used,
        iterations_budget: t.iterations_budget,
        assigned_by: t.assigned_by.clone(),
        source_channel: t.source_channel.clone(),
        branch: t.branch.clone(),
        priority: t.priority,
    }
}

#[async_trait]
impl TaskPersistence for TaskStorePersistence {
    async fn load(&self, task_id: &str) -> Option<TaskRecord> {
        self.store.get(task_id).await.as_ref().map(agent_task_to_record)
    }

    async fn set_status(&self, task_id: &str, status: &str) -> bool {
        let parsed = TaskStatus::parse(status);
        self.store.update(task_id, Some(parsed), None, None).await
    }

    async fn checkpoint_iteration(
        &self,
        task_id: &str,
        _iterations_used: i64,
        checkpoint: &JsonValue,
    ) -> bool {
        self.store
            .update(task_id, None, Some(checkpoint), None)
            .await
    }

    async fn record_outcome(&self, task_id: &str, outcome: &TaskOutcome) -> bool {
        let status = TaskStatus::parse(outcome.as_status_str());
        let checkpoint = serde_json::json!({
            "outcome": outcome.summary(),
            "recorded_at": chrono::Utc::now().to_rfc3339(),
        });
        self.store
            .update(task_id, Some(status), Some(&checkpoint), None)
            .await
    }
}

// ============================================================================
// AcceptanceChecker adapter
// ============================================================================

/// Adapter that runs `zeus_prometheus::acceptance::run_all` against the
/// checks embedded in a task's `scope_json.acceptance_checks` array.
pub struct PrometheusAcceptanceChecker;

impl PrometheusAcceptanceChecker {
    pub fn new() -> Self {
        Self
    }

    pub fn arc() -> Arc<dyn AcceptanceChecker> {
        Arc::new(Self)
    }

    /// Pull the `acceptance_checks` array out of `scope_json`. Returns
    /// an empty Vec if missing or malformed — caller treats that as
    /// "no checks specified, keep progressing until budget".
    fn extract_checks(scope: &JsonValue) -> Vec<AcceptanceCheck> {
        let arr = match scope.get("acceptance_checks").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return Vec::new(),
        };
        arr.iter()
            .filter_map(|v| serde_json::from_value::<AcceptanceCheck>(v.clone()).ok())
            .collect()
    }
}

impl Default for PrometheusAcceptanceChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AcceptanceChecker for PrometheusAcceptanceChecker {
    async fn check(&self, task: &TaskRecord) -> AcceptanceResult {
        let checks = Self::extract_checks(&task.scope_json);
        if checks.is_empty() {
            // No checks declared → rely on iteration/wall budget to terminate.
            return AcceptanceResult::Progressing;
        }

        // run_all shells out (cargo check / test). Offload to blocking pool
        // so we don't stall the async executor.
        let report = tokio::task::spawn_blocking(move || run_all(&checks))
            .await
            .ok();

        match report {
            Some(r) if r.all_passed => AcceptanceResult::Done,
            Some(r) => {
                let failures: Vec<String> = r
                    .results
                    .iter()
                    .filter(|res| !res.passed)
                    .map(|res| format!("{}: {}", res.check, res.detail))
                    .collect();
                if failures.is_empty() {
                    AcceptanceResult::Progressing
                } else {
                    // Failing checks are recoverable — keep looping so
                    // the agent can try again. The budget cap stops
                    // runaway loops.
                    tracing::debug!(
                        task_id = %task.id,
                        failed = failures.len(),
                        "acceptance checks failing, continuing"
                    );
                    AcceptanceResult::Progressing
                }
            }
            None => AcceptanceResult::Blocked("acceptance runner panicked".to_string()),
        }
    }
}

// ============================================================================
// Convenience constructors for the gateway
// ============================================================================

/// Build the pair of trait objects the cooking loop expects, from a
/// `TaskStore` handle. Call at gateway startup.
pub fn build_cook_deps(
    store: TaskStore,
) -> (Arc<dyn TaskPersistence>, Arc<dyn AcceptanceChecker>) {
    (
        TaskStorePersistence::arc(store),
        PrometheusAcceptanceChecker::arc(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_checks_handles_missing_field() {
        let scope = serde_json::json!({ "files": ["foo.rs"] });
        let checks = PrometheusAcceptanceChecker::extract_checks(&scope);
        assert!(checks.is_empty());
    }

    #[test]
    fn extract_checks_skips_malformed_entries() {
        let scope = serde_json::json!({
            "acceptance_checks": [
                { "not_a_valid": "variant" },
                "also_not_valid",
            ]
        });
        let checks = PrometheusAcceptanceChecker::extract_checks(&scope);
        assert!(checks.is_empty());
    }

    #[tokio::test]
    async fn checker_returns_progressing_when_no_checks() {
        let checker = PrometheusAcceptanceChecker::new();
        let task = TaskRecord {
            id: "t1".into(),
            agent_id: "a".into(),
            description: "".into(),
            scope_json: serde_json::json!({}),
            iterations_used: 0,
            iterations_budget: 10,
            assigned_by: "".into(),
            source_channel: "".into(),
            branch: "".into(),
            priority: 1,
        };
        assert_eq!(checker.check(&task).await, AcceptanceResult::Progressing);
    }
}
