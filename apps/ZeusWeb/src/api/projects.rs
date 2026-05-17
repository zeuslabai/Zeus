// Projects, schedules, approvals, goals, workflows, observatory

use super::*;

// Projects

pub async fn fetch_projects() -> Result<ProjectsResponse, String> {
    fetch_json("/v1/projects").await
}

pub async fn fetch_project(id: &str) -> Result<Project, String> {
    fetch_json(&format!("/v1/projects/{}", id)).await
}

pub async fn create_project(req: &CreateProjectReq) -> Result<MsgResponse, String> {
    post_json("/v1/projects", req).await
}

pub async fn delete_project(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/projects/{}", id)).await
}

pub async fn update_project(id: &str, req: &UpdateProjectReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/projects/{}", id), req).await
}

pub async fn assign_project_agents(id: &str, req: &AssignAgentsReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/projects/{}/agents", id), req).await
}

// Schedules

pub async fn fetch_schedules() -> Result<SchedulesResponse, String> {
    fetch_json("/v1/schedules").await
}

pub async fn create_schedule(req: &CreateScheduleReq) -> Result<MsgResponse, String> {
    post_json("/v1/schedules", req).await
}

pub async fn update_schedule(id: &str, body: &serde_json::Value) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/schedules/{}", id), body).await
}

pub async fn delete_schedule(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/schedules/{}", id)).await
}

pub async fn fetch_schedule_history(id: &str) -> Result<ScheduleHistoryResponse, String> {
    fetch_json(&format!("/v1/schedules/{}/history", id)).await
}

pub async fn pause_schedule(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/schedules/{}/pause", id), &serde_json::json!({})).await
}

pub async fn resume_schedule(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/schedules/{}/resume", id), &serde_json::json!({})).await
}

pub async fn fetch_schedule_runs(id: &str) -> Result<ScheduleRunsResponse, String> {
    fetch_json(&format!("/v1/schedules/{}/runs", id)).await
}

// Approvals

pub async fn fetch_approvals() -> Result<Vec<PendingApproval>, String> {
    fetch_json("/v1/approvals").await
}

pub async fn approve_execution(id: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/approvals/{}/approve", id), &serde_json::json!({})).await
}

pub async fn deny_execution(id: &str, reason: Option<&str>) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/approvals/{}/deny", id), &serde_json::json!({ "reason": reason })).await
}

// Goals

pub async fn fetch_goals() -> Result<GoalsResponse, String> {
    fetch_json("/v1/goals").await
}

pub async fn get_goal(id: &str) -> Result<GoalResponse, String> {
    fetch_json(&format!("/v1/goals/{}", id)).await
}

pub async fn create_goal(req: &CreateGoalRequest) -> Result<GoalResponse, String> {
    post_json("/v1/goals", req).await
}

pub async fn analyze_goal(goal: &str) -> Result<GoalAnalysisResponse, String> {
    let body = serde_json::json!({ "goal": goal });
    post_json("/v1/goals/analyze", &body).await
}

pub async fn analyze_goal_with_provider(
    goal: &str,
    provider: &str,
    model: &str,
    api_key: &str,
    url: &str,
) -> Result<GoalAnalysisResponse, String> {
    let mut body = serde_json::json!({ "goal": goal, "provider": provider, "model": model });
    if !api_key.is_empty() {
        body["api_key"] = serde_json::json!(api_key);
    }
    if !url.is_empty() {
        body["url"] = serde_json::json!(url);
    }
    post_json("/v1/goals/analyze", &body).await
}

pub async fn update_goal_status(id: &str, status: &str) -> Result<GoalResponse, String> {
    let body = serde_json::json!({ "status": status });
    put_json(&format!("/v1/goals/{}/status", id), &body).await
}

// Workflows

pub async fn fetch_workflows() -> Result<WorkflowsListResponse, String> {
    fetch_json("/v1/workflows").await
}

pub async fn create_workflow(body: &serde_json::Value) -> Result<WorkflowCreateResponse, String> {
    post_json("/v1/workflows", body).await
}

pub async fn fetch_workflow(id: &str) -> Result<WorkflowDetail, String> {
    fetch_json(&format!("/v1/workflows/{}", id)).await
}

pub async fn fetch_workflow_artifacts(id: &str) -> Result<WorkflowArtifacts, String> {
    fetch_json(&format!("/v1/workflows/{}/artifacts", id)).await
}

// Observatory

pub async fn fetch_observatory_active_tasks() -> Result<ObservatoryActiveTasks, String> {
    fetch_json("/v1/observatory/active-tasks").await
}

pub async fn fetch_observatory_agent_stats() -> Result<ObservatoryAgentStats, String> {
    fetch_json("/v1/observatory/agent-stats").await
}

pub async fn fetch_observatory_channel_health() -> Result<ObservatoryChannelHealth, String> {
    fetch_json("/v1/observatory/channel-health").await
}

pub async fn fetch_observatory_cost_live() -> Result<ObservatoryCostLive, String> {
    fetch_json("/v1/observatory/cost-live").await
}

// Cron

pub async fn fetch_cron_jobs() -> Result<CronJobsResponse, String> {
    fetch_json("/v1/cron/jobs").await
}

pub async fn create_cron_job(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/cron/jobs", body).await
}

pub async fn delete_cron_job(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/cron/jobs/{}", id)).await
}

pub async fn fetch_cron_job_history(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/cron/jobs/{}/history", id)).await
}

pub async fn fetch_cron_templates() -> Result<CronTemplatesResponse, String> {
    fetch_json("/v1/cron/templates").await
}

// Webhooks

pub async fn fetch_webhook_health() -> Result<WebhookHealthResponse, String> {
    fetch_json("/v1/webhooks").await
}

pub async fn fetch_webhook_triggers() -> Result<WebhookTriggersResponse, String> {
    fetch_json("/v1/webhooks/triggers").await
}

pub async fn create_webhook_trigger(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/webhooks/triggers", body).await
}

pub async fn delete_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/webhooks/triggers/{}", id)).await
}

pub async fn enable_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/webhooks/triggers/{}/enable", id), &serde_json::json!({})).await
}

pub async fn disable_webhook_trigger(id: &str) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/webhooks/triggers/{}/disable", id), &serde_json::json!({})).await
}

pub async fn fetch_outbound_webhooks() -> Result<OutboundWebhooksResponse, String> {
    fetch_json("/v1/webhooks/outbound").await
}

pub async fn register_outbound_webhook(body: &serde_json::Value) -> Result<OutboundWebhook, String> {
    post_json("/v1/webhooks/outbound", body).await
}

pub async fn delete_outbound_webhook(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/webhooks/outbound/{}", id)).await
}

// Outcome Templates

pub async fn fetch_templates(category: Option<&str>, limit: Option<u64>) -> Result<TemplatesListResponse, String> {
    let mut url = "/v1/templates?".to_string();
    if let Some(cat) = category { url.push_str(&format!("category={}&", cat)); }
    if let Some(l) = limit { url.push_str(&format!("limit={}&", l)); }
    fetch_json(&url).await
}

pub async fn fetch_template_categories() -> Result<Vec<String>, String> {
    fetch_json::<CategoriesResponse>("/v1/templates/categories").await.map(|r| r.categories)
}

pub async fn search_templates(q: &str) -> Result<Vec<OutcomeTemplate>, String> {
    fetch_json::<TemplatesListResponse>(&format!("/v1/templates/search?q={}", q)).await.map(|r| r.templates)
}

pub async fn apply_template(id: &str, goal: &str) -> Result<AppliedTemplate, String> {
    post_json::<serde_json::Value, AppliedTemplate>(
        &format!("/v1/templates/{}/apply", id),
        &serde_json::json!({ "goal": goal }),
    ).await
}
