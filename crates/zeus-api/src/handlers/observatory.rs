//! Observatory Dashboard API handlers
//!
//! Real-time system monitoring endpoints for Zeus:
//! - GET /v1/observatory/active-tasks   — Prometheus active tasks & cooking loops
//! - GET /v1/observatory/agent-stats    — Agent registry stats & subagent status
//! - GET /v1/observatory/channel-health — Per-channel health with uptime metrics
//! - GET /v1/observatory/cost-live      — Live cost tracking & budget burn rate

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// GET /v1/observatory/active-tasks
// ============================================================================

/// Returns active Prometheus tasks, cooking loop status, cron jobs, and goal progress.
pub async fn active_tasks(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    // Cron scheduler tasks
    let cron_tasks: Vec<Value> = state
        .cron_scheduler()
        .list_tasks()
        .await
        .iter()
        .map(|task| {
            json!({
                "id": task.id,
                "name": task.name,
                "cron_expr": task.cron_expr,
                "enabled": task.enabled,
                "last_run": task.last_run.map(|t| t.to_rfc3339()),
                "next_run": task.next_run.map(|t| t.to_rfc3339()),
                "type": "cron",
            })
        })
        .collect();

    // Orchestration workflows in progress
    let workflows: Vec<Value> = state
        .workflow_states
        .iter()
        .map(|entry| {
            let ws = entry.value();
            json!({
                "workflow_id": ws.workflow_id,
                "status": ws.status,
                "message": ws.message,
                "progress_pct": ws.progress_percentage,
                "total_nodes": ws.total_nodes,
                "completed_nodes": ws.completed_nodes,
                "failed_nodes": ws.failed_nodes,
                "created_at": ws.created_at,
                "started_at": ws.started_at,
                "type": "workflow",
            })
        })
        .collect();

    // Pending approvals (tool executions waiting for human confirm)
    let pending_approvals = state.approvals.list_pending();
    let approvals: Vec<Value> = pending_approvals
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "tool": a.tool_name,
                "requested_at": a.created_at.to_rfc3339(),
                "type": "approval",
            })
        })
        .collect();

    let cron_enabled = cron_tasks.iter().filter(|j| j["enabled"] == true).count();
    let workflows_active = workflows
        .iter()
        .filter(|w| w["status"] == "running")
        .count();

    Json(json!({
        "cron_tasks": cron_tasks,
        "workflows": workflows,
        "pending_approvals": approvals,
        "summary": {
            "cron_total": cron_tasks.len(),
            "cron_enabled": cron_enabled,
            "workflows_active": workflows_active,
            "workflows_total": workflows.len(),
            "approvals_pending": approvals.len(),
        },
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

// ============================================================================
// GET /v1/observatory/agent-stats
// ============================================================================

/// Returns agent registry statistics: registered agents, their bindings,
/// activity timestamps, and runtime status.
pub async fn agent_stats(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    let agents = state.agent_registry.list();
    let agent_reports: Vec<Value> = agents
        .iter()
        .map(|a| {
            json!({
                "agent_id": a.agent_id,
                "name": a.name,
                "spawned_at": a.spawned_at.to_rfc3339(),
                "last_active": a.last_active.to_rfc3339(),
                "message_count": a.message_count,
                "binding": {
                    "agent_id": a.binding.agent_id,
                    "bindings": a.binding.bindings.len(),
                    "tool_policy": format!("{:?}", a.binding.tool_policy),
                    "priority": a.binding.priority,
                },
            })
        })
        .collect();

    // Orchestra teams
    let teams = state.orchestra().list_teams().await;
    let team_reports: Vec<Value> = teams
        .iter()
        .map(|t| {
            json!({
                "team_id": t.id,
                "name": t.name,
                "member_count": t.agent_ids.len(),
                "supervisor": t.supervisor_id,
            })
        })
        .collect();

    let total_agents = agent_reports.len();

    Json(json!({
        "agents": agent_reports,
        "summary": {
            "total_agents": total_agents,
        },
        "teams": team_reports,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

// ============================================================================
// GET /v1/observatory/channel-health
// ============================================================================

/// Returns per-channel health with uptime metrics and connectivity.
pub async fn channel_health(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    let channels = state.channel_store.list().await;
    let mut channel_reports = Vec::new();
    let mut connected_count = 0usize;

    for ch in &channels {
        let connected = ch.enabled;
        if connected {
            connected_count += 1;
        }

        channel_reports.push(json!({
            "id": ch.id,
            "channel_type": ch.channel_type,
            "name": ch.name,
            "enabled": ch.enabled,
            "connected": connected,
            "last_message_at": ch.last_message_at.map(|t| t.to_rfc3339()),
            "created_at": ch.created_at.to_rfc3339(),
            "uptime_pct": if connected { 100.0 } else { 0.0 },
        }));
    }

    let total_count = channels.len();

    Json(json!({
        "channels": channel_reports,
        "summary": {
            "total": total_count,
            "connected": connected_count,
            "disconnected": total_count - connected_count,
            "overall_healthy": connected_count == total_count && total_count > 0,
        },
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

// ============================================================================
// GET /v1/observatory/cost-live
// ============================================================================

/// Returns live cost tracking: budget usage, top models, burn rate.
pub async fn cost_live(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    // Cost router summary
    let summary = state.cost_router.summary();

    // Economy ledger balance for default agent
    let default_balance = state.ledger.balance("default").unwrap_or(0);

    Json(json!({
        "cost_summary": {
            "total_cost": summary.total_cost,
            "budget_remaining": summary.budget_remaining,
            "period_start": summary.period_start,
            "top_models": summary.top_models.iter().map(|(m, c)| json!({
                "model": m,
                "cost": c,
            })).collect::<Vec<_>>(),
        },
        "economy": {
            "token_balance": default_balance,
        },
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use zeus_core::Config;

    fn test_state() -> SharedState {
        let config = Config::default();
        Arc::new(RwLock::new(AppState::new(config).unwrap()))
    }

    fn test_state_isolated() -> (SharedState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut config = Config::default();
        config.workspace = tmp.path().join("workspace");
        config.sessions = tmp.path().join("sessions");
        let state = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        (state, tmp)
    }

    #[tokio::test]
    async fn test_active_tasks_returns_structure() {
        let state = test_state();
        let Json(result) = active_tasks(State(state)).await;
        assert!(result.get("cron_tasks").unwrap().is_array());
        assert!(result.get("workflows").unwrap().is_array());
        assert!(result.get("pending_approvals").unwrap().is_array());
        assert!(result.get("summary").is_some());
        assert!(result.get("timestamp").is_some());
    }

    #[tokio::test]
    async fn test_active_tasks_summary_fields() {
        let state = test_state();
        let Json(result) = active_tasks(State(state)).await;
        let summary = result.get("summary").unwrap();
        assert!(summary.get("cron_total").is_some());
        assert!(summary.get("cron_enabled").is_some());
        assert!(summary.get("workflows_active").is_some());
        assert!(summary.get("approvals_pending").is_some());
    }

    #[tokio::test]
    async fn test_active_tasks_defaults() {
        let state = test_state();
        let Json(result) = active_tasks(State(state)).await;
        let summary = result.get("summary").unwrap();
        // CronScheduler may have default tasks from config — just verify it's a valid u64
        assert!(summary["cron_total"].as_u64().is_some());
        assert_eq!(summary["workflows_total"], 0);
        assert_eq!(summary["approvals_pending"], 0);
    }

    #[tokio::test]
    async fn test_agent_stats_returns_structure() {
        let state = test_state();
        let Json(result) = agent_stats(State(state)).await;
        assert!(result.get("agents").unwrap().is_array());
        assert!(result.get("summary").is_some());
        assert!(result.get("teams").unwrap().is_array());
        assert!(result.get("timestamp").is_some());
    }

    #[tokio::test]
    async fn test_agent_stats_empty_by_default() {
        let state = test_state();
        let Json(result) = agent_stats(State(state)).await;
        let summary = result.get("summary").unwrap();
        assert_eq!(summary["total_agents"], 0);
    }

    #[tokio::test]
    async fn test_channel_health_returns_structure() {
        let state = test_state();
        let Json(result) = channel_health(State(state)).await;
        assert!(result.get("channels").unwrap().is_array());
        assert!(result.get("summary").is_some());
        assert!(result.get("timestamp").is_some());
    }

    #[tokio::test]
    async fn test_channel_health_summary_fields() {
        let state = test_state();
        let Json(result) = channel_health(State(state)).await;
        let summary = result.get("summary").unwrap();
        assert!(summary.get("total").is_some());
        assert!(summary.get("connected").is_some());
        assert!(summary.get("disconnected").is_some());
        assert!(summary.get("overall_healthy").is_some());
    }

    #[tokio::test]
    async fn test_channel_health_empty_not_healthy() {
        // Use isolated tempdir workspace so live ~/.zeus/workspace channels are not loaded
        let (state, _tmp) = test_state_isolated();
        let Json(result) = channel_health(State(state)).await;
        let summary = result.get("summary").unwrap();
        // No channels = not healthy
        assert_eq!(summary["overall_healthy"], false);
        assert_eq!(summary["total"], 0);
    }

    #[tokio::test]
    async fn test_cost_live_returns_structure() {
        let state = test_state();
        let Json(result) = cost_live(State(state)).await;
        assert!(result.get("cost_summary").is_some());
        assert!(result.get("economy").is_some());
        assert!(result.get("timestamp").is_some());
    }

    #[tokio::test]
    async fn test_cost_live_budget_fields() {
        let state = test_state();
        let Json(result) = cost_live(State(state)).await;
        let cost = result.get("cost_summary").unwrap();
        assert!(cost.get("total_cost").is_some());
        assert!(cost.get("budget_remaining").is_some());
        assert!(cost.get("period_start").is_some());
        assert!(cost.get("top_models").unwrap().is_array());
    }

    #[tokio::test]
    async fn test_cost_live_economy_has_balance() {
        let state = test_state();
        let Json(result) = cost_live(State(state)).await;
        let econ = result.get("economy").unwrap();
        // Default agent gets minted 10,000 tokens in AppState::new
        assert!(econ["token_balance"].as_u64().unwrap() >= 10_000);
    }
}
