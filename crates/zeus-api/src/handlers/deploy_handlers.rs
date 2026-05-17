//! Deploy handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use crate::SharedState;
use crate::handlers::deploy_pipeline;
use crate::handlers::deploy_store;

/// Decode a base64 string into bytes.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode error: {e}"))
}

/// Capture a screenshot of a URL via Chrome CDP.
async fn capture_screenshot(url: &str) -> Result<String, String> {
    let mut client = zeus_browser::CdpClient::with_default_url();
    let tab = client
        .new_tab(Some(url))
        .await
        .map_err(|e| format!("Failed to open tab: {}", e))?;
    client
        .connect(Some(&tab.id))
        .await
        .map_err(|e| format!("Failed to connect to tab: {}", e))?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let base64_png = client
        .screenshot()
        .await
        .map_err(|e| format!("Screenshot failed: {}", e))?;
    let _ = client.close_tab(&tab.id).await;
    Ok(base64_png)
}

pub async fn deploy_list_targets(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let targets = state_guard.deploy_store.list_targets().await;
    Json(json!({ "targets": targets, "total": targets.len() }))
}

pub async fn deploy_create_target(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name'".to_string()))?;
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'provider'".to_string()))?;

    let id = format!(
        "target-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    let now = chrono::Utc::now().to_rfc3339();
    let target = deploy_store::DeployTargetRow {
        id: id.clone(),
        name: name.to_string(),
        provider: provider.to_string(),
        environment: body
            .get("environment")
            .and_then(|v| v.as_str())
            .unwrap_or("production")
            .to_string(),
        config_json: body
            .get("config")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".to_string()),
        credentials_ref: body
            .get("credentials_ref")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        project_path: body
            .get("project_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        build_command: body
            .get("build_command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        output_dir: body
            .get("output_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        url: body
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        active: true,
        created_at: now.clone(),
        updated_at: now,
    };

    let state_guard = state.read().await;
    if state_guard.deploy_store.create_target(&target).await {
        Ok((
            StatusCode::CREATED,
            Json(json!({ "id": id, "status": "created", "target": target })),
        ))
    } else {
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create target".to_string(),
        ))
    }
}

pub async fn deploy_get_target(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    match state_guard.deploy_store.get_target(&id).await {
        Some(target) => Ok(Json(serde_json::to_value(target).unwrap_or_default())),
        None => Err((StatusCode::NOT_FOUND, format!("Target {} not found", id))),
    }
}

pub async fn deploy_delete_target(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    if state_guard.deploy_store.deactivate_target(&id).await {
        Ok(Json(json!({ "status": "deactivated", "id": id })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Target {} not found", id)))
    }
}

pub async fn deploy_create(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let target_id = body
        .get("target_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'target_id'".to_string()))?;

    let state_guard = state.read().await;
    // Verify target exists
    let target = state_guard
        .deploy_store
        .get_target(target_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Target {} not found", target_id),
            )
        })?;

    let id = format!(
        "deploy-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    let version = body
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0");
    let now = chrono::Utc::now().to_rfc3339();

    let deployment = deploy_store::DeploymentRow {
        id: id.clone(),
        target_id: target_id.to_string(),
        version: version.to_string(),
        status: "pending".to_string(),
        trigger: body
            .get("trigger")
            .and_then(|v| v.as_str())
            .unwrap_or("manual")
            .to_string(),
        commit_hash: body
            .get("commit_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        commit_message: body
            .get("commit_message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        build_log: String::new(),
        deploy_url: String::new(),
        preview_url: String::new(),
        duration_secs: 0,
        error_message: String::new(),
        initiated_by: body
            .get("initiated_by")
            .and_then(|v| v.as_str())
            .unwrap_or("system")
            .to_string(),
        metadata_json: body
            .get("metadata")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".to_string()),
        created_at: now,
        started_at: None,
        completed_at: None,
    };

    if state_guard
        .deploy_store
        .create_deployment(&deployment)
        .await
    {
        let resp =
            deploy_store::DeploymentResponse::from_row(deployment, &target.name, &target.provider);

        // If "execute": true, spawn the pipeline in the background
        let execute = body
            .get("execute")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if execute {
            let store = std::sync::Arc::new(state_guard.deploy_store.clone());
            let pipeline_config = deploy_pipeline::PipelineConfig {
                target,
                deployment_id: id.clone(),
                version: version.to_string(),
                skip_tests: body
                    .get("skip_tests")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                skip_verify: body
                    .get("skip_verify")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                env_vars: vec![],
            };
            tokio::spawn(async move {
                deploy_pipeline::run_pipeline(store, pipeline_config).await;
            });
        }

        Ok((
            StatusCode::CREATED,
            Json(serde_json::to_value(resp).unwrap_or_default()),
        ))
    } else {
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create deployment".to_string(),
        ))
    }
}

pub async fn deploy_execute(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let deployment = state_guard
        .deploy_store
        .get_deployment(&id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Deployment {} not found", id),
            )
        })?;

    if deployment.status != "pending" {
        return Err((
            StatusCode::CONFLICT,
            format!("Deployment {} is already {}", id, deployment.status),
        ));
    }

    let target = state_guard
        .deploy_store
        .get_target(&deployment.target_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Target {} not found", deployment.target_id),
            )
        })?;

    let store = std::sync::Arc::new(state_guard.deploy_store.clone());
    let pipeline_config = deploy_pipeline::PipelineConfig {
        target,
        deployment_id: id.clone(),
        version: deployment.version.clone(),
        skip_tests: body
            .get("skip_tests")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        skip_verify: body
            .get("skip_verify")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        env_vars: vec![],
    };

    tokio::spawn(async move {
        deploy_pipeline::run_pipeline(store, pipeline_config).await;
    });

    Ok(Json(
        json!({ "id": id, "status": "executing", "message": "Pipeline started" }),
    ))
}

pub async fn deploy_get(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let deployment = state_guard
        .deploy_store
        .get_deployment(&id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Deployment {} not found", id),
            )
        })?;
    let target = state_guard
        .deploy_store
        .get_target(&deployment.target_id)
        .await;
    let (target_name, provider) = target.map(|t| (t.name, t.provider)).unwrap_or_default();
    let resp = deploy_store::DeploymentResponse::from_row(deployment, &target_name, &provider);
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

pub async fn deploy_update_status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'status'".to_string()))?;

    let valid = [
        "pending",
        "building",
        "deploying",
        "live",
        "failed",
        "cancelled",
        "rolled_back",
    ];
    if !valid.contains(&status) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid status. Must be one of: {:?}", valid),
        ));
    }

    let state_guard = state.read().await;
    let updated = state_guard
        .deploy_store
        .update_deployment_status(
            &id,
            status,
            body.get("deploy_url").and_then(|v| v.as_str()),
            body.get("preview_url").and_then(|v| v.as_str()),
            body.get("error_message").and_then(|v| v.as_str()),
            body.get("duration_secs").and_then(|v| v.as_u64()),
        )
        .await;

    if updated {
        Ok(Json(json!({ "id": id, "status": status })))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            format!("Deployment {} not found", id),
        ))
    }
}

pub async fn deploy_logs(State(state): State<SharedState>, Path(id): Path<String>) -> Json<Value> {
    let state_guard = state.read().await;
    let logs = state_guard.deploy_store.get_logs(&id).await;
    Json(json!({ "deployment_id": id, "logs": logs, "total": logs.len() }))
}

pub async fn deploy_history(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20);
    let state_guard = state.read().await;
    let deployments = state_guard
        .deploy_store
        .list_recent_deployments(limit)
        .await;
    Json(json!({ "deployments": deployments, "total": deployments.len() }))
}

pub async fn deploy_target_history(
    State(state): State<SharedState>,
    Path(target_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20);
    let state_guard = state.read().await;
    let deployments = state_guard
        .deploy_store
        .list_deployments(&target_id, limit)
        .await;
    Json(json!({ "target_id": target_id, "deployments": deployments, "total": deployments.len() }))
}

pub async fn deploy_rollback(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let snapshot_id = body
        .get("snapshot_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'snapshot_id'".to_string()))?;

    let state_guard = state.read().await;
    match state_guard.deploy_store.rollback_to(snapshot_id).await {
        Some(url) => {
            // Mark current deployment as rolled back
            state_guard
                .deploy_store
                .update_deployment_status(&id, "rolled_back", None, None, None, None)
                .await;
            Ok(Json(
                json!({ "status": "rolled_back", "deployment_id": id, "snapshot_id": snapshot_id, "deploy_url": url }),
            ))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Snapshot {} not found", snapshot_id),
        )),
    }
}

pub async fn deploy_snapshots(
    State(state): State<SharedState>,
    Path(target_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10);
    let state_guard = state.read().await;
    let snapshots = state_guard
        .deploy_store
        .list_snapshots(&target_id, limit)
        .await;
    Json(json!({ "target_id": target_id, "snapshots": snapshots, "total": snapshots.len() }))
}

pub async fn deploy_stats(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let stats = state_guard.deploy_store.stats().await;
    Json(serde_json::to_value(stats).unwrap_or_default())
}

pub async fn deploy_preview(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let deployment = state_guard
        .deploy_store
        .get_deployment(&id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Deployment {} not found", id),
            )
        })?;

    let url = if !deployment.deploy_url.is_empty() {
        deployment.deploy_url.clone()
    } else if !deployment.preview_url.is_empty() {
        deployment.preview_url.clone()
    } else {
        // Fall back to the target's configured URL
        let target = state_guard
            .deploy_store
            .get_target(&deployment.target_id)
            .await;
        target.map(|t| t.url).unwrap_or_default()
    };

    if url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Deployment has no URL to preview".to_string(),
        ));
    }

    // Try to connect to Chrome CDP and take a screenshot
    match capture_screenshot(&url).await {
        Ok(base64_png) => {
            // Save to workspace for caching
            let screenshot_dir = state_guard.config.workspace.join("screenshots");
            let _ = std::fs::create_dir_all(&screenshot_dir);
            let filename = format!("{}.png", id);
            let filepath = screenshot_dir.join(&filename);
            if let Ok(bytes) = base64_decode(&base64_png) {
                let _ = std::fs::write(&filepath, &bytes);
            }

            Ok(Json(json!({
                "deployment_id": id,
                "url": url,
                "screenshot": base64_png,
                "format": "png",
                "cached_path": filepath.to_string_lossy(),
            })))
        }
        Err(e) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Screenshot capture failed (is Chrome running with --remote-debugging-port=9222?): {}",
                e
            ),
        )),
    }
}

