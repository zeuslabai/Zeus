//! Cron job REST API handlers
//!
//! Endpoints for managing Prometheus cron scheduler jobs.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::SharedState;

/// GET /v1/cron/jobs — List all cron jobs
pub async fn list_cron_jobs(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let tasks = st.cron_scheduler().list_tasks().await;
    Json(json!({
        "jobs": tasks,
        "count": tasks.len(),
    }))
}

/// POST /v1/cron/jobs — Create a new cron job
///
/// Body: { "name": "...", "cron": "every 5 minutes", "task_type": {...}, "enabled": true }
/// Or:   { "template": "daily_summary" }
pub async fn create_cron_job(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let st = state.read().await;

    // Check if using a template
    if let Some(template_name) = body.get("template").and_then(|v| v.as_str()) {
        let config = zeus_prometheus::CronJobTemplate::get(template_name).ok_or_else(|| {
            let templates: Vec<&str> = zeus_prometheus::CronJobTemplate::all()
                .iter()
                .map(|(name, _)| *name)
                .collect();
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("Unknown template: '{}'. Available: {:?}", template_name, templates),
                })),
            )
        })?;

        let id = st.cron_scheduler().add_task(config).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

        return Ok((
            StatusCode::CREATED,
            Json(json!({ "id": id, "template": template_name })),
        ));
    }

    // Manual job creation
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing 'name' field" })),
            )
        })?
        .to_string();

    let cron = body
        .get("cron")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing 'cron' field (e.g. 'every 5 minutes', 'daily at 9am', '*/5 * * * *')" })),
            )
        })?
        .to_string();

    let task_type: zeus_prometheus::TaskType = serde_json::from_value(
        body.get("task_type")
            .cloned()
            .unwrap_or(json!({"type": "shell", "command": "echo ok"})),
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid task_type: {e}") })),
        )
    })?;

    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Optional one-shot (#174 P1): RFC3339 `run_at` fires once then self-deletes.
    let run_at = match body.get("run_at").and_then(|v| v.as_str()) {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("Invalid 'run_at' (RFC3339): {e}") })),
                    )
                })?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    let run_once = body
        .get("run_once")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Wake-mode (#174 P2): 'now' (default) | 'next_heartbeat'. Unknown → Now.
    let wake_mode = match body.get("wake_mode").and_then(|v| v.as_str()) {
        Some("next_heartbeat") => zeus_prometheus::WakeMode::NextHeartbeat,
        _ => zeus_prometheus::WakeMode::Now,
    };

    // Delivery-mode (#174 P3): 'channel' (default) | 'heartbeat_note' | 'silent_ledger'.
    let delivery_mode = match body.get("delivery_mode").and_then(|v| v.as_str()) {
        Some("heartbeat_note") => zeus_prometheus::DeliveryMode::HeartbeatNote,
        Some("silent_ledger") => zeus_prometheus::DeliveryMode::SilentLedger,
        _ => zeus_prometheus::DeliveryMode::Channel,
    };

    let config = zeus_prometheus::TaskConfig {
        name,
        cron,
        task_type,
        enabled,
        run_at,
        run_once,
        wake_mode,
        delivery_mode,
    };

    let id = st.cron_scheduler().add_task(config).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// DELETE /v1/cron/jobs/:id — Delete a cron job
pub async fn delete_cron_job(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let st = state.read().await;
    st.cron_scheduler().remove_task(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "deleted": id })))
}

/// GET /v1/cron/jobs/:id/history — Get execution history for a job
pub async fn cron_job_history(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let st = state.read().await;
    let history = st
        .cron_scheduler()
        .get_job_history(&id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("No job with id '{}'", id) })),
            )
        })?;
    Ok(Json(json!(history)))
}

/// POST /v1/cron/jobs/:id/abort — Abort a running cron job
pub async fn abort_cron_job(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let st = state.read().await;
    st.cron_scheduler().abort_task(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "aborted": id })))
}

/// GET /v1/cron/jobs/running — List currently running job IDs
pub async fn list_running_cron_jobs(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let running = st.cron_scheduler().running_task_ids().await;
    Json(json!({
        "running": running,
        "count": running.len(),
        "active_jobs": st.cron_scheduler().active_job_count(),
        "max_concurrent": st.cron_scheduler().max_concurrent_jobs(),
        "available_slots": st.cron_scheduler().available_slots(),
    }))
}

/// GET /v1/cron/templates — List available job templates
pub async fn list_cron_templates() -> Json<Value> {
    let templates: Vec<Value> = zeus_prometheus::CronJobTemplate::all()
        .into_iter()
        .map(|(name, config)| {
            json!({
                "id": name,
                "name": config.name,
                "cron": config.cron,
                "task_type": config.task_type,
            })
        })
        .collect();

    Json(json!({
        "templates": templates,
        "count": templates.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state() -> SharedState {
        let tmp = std::env::temp_dir().join(format!("zeus-cron-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&tmp).ok();
        let mut config = zeus_core::Config::default();
        config.workspace = tmp;
        config.onboarding_complete = true;
        Arc::new(RwLock::new(AppState::new(config).unwrap()))
    }

    fn test_app(state: SharedState) -> axum::Router {
        crate::create_test_router(state)
    }

    #[tokio::test]
    async fn test_list_cron_jobs_empty() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/cron/jobs")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["jobs"].is_array());
        // Count reflects whatever the scheduler seeds by default — just verify it's numeric
        assert!(json["count"].is_number());
    }

    #[tokio::test]
    async fn test_create_cron_job_manual() {
        let state = test_state();
        let app = test_app(state);

        let body = serde_json::json!({
            "name": "test-job",
            "cron": "every 5 minutes",
            "task_type": {"type": "shell", "command": "echo ok"}
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/cron/jobs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["id"].is_string());
    }

    #[tokio::test]
    async fn test_create_cron_job_from_template() {
        let state = test_state();
        let app = test_app(state);

        let body = serde_json::json!({
            "template": "daily_summary"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/cron/jobs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["id"].is_string());
        assert_eq!(json["template"], "daily_summary");
    }

    #[tokio::test]
    async fn test_create_cron_job_invalid_template() {
        let state = test_state();
        let app = test_app(state);

        let body = serde_json::json!({
            "template": "nonexistent_template"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/cron/jobs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_cron_job_missing_fields() {
        let state = test_state();
        let app = test_app(state);

        let body = serde_json::json!({
            "name": "test-job"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/cron/jobs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_cron_templates() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/cron/templates")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 6);
        assert!(json["templates"].is_array());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_job() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/cron/jobs/nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_abort_nonexistent_job() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/cron/jobs/nonexistent-id/abort")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_running_jobs_empty() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/cron/jobs/running")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
        assert!(json["running"].is_array());
        assert!(json["max_concurrent"].is_number());
        assert!(json["available_slots"].is_number());
    }

    #[tokio::test]
    async fn test_job_history_nonexistent() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/cron/jobs/nonexistent-id/history")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
