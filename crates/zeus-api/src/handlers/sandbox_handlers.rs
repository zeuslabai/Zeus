use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;

/// List sandbox policies
pub async fn list_sandbox_policies(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let restrictive = zeus_sandbox::SandboxPolicy::restrictive("restrictive");
    let permissive = zeus_sandbox::SandboxPolicy::permissive("permissive");

    let mut policies = vec![
        json!({
            "id": restrictive.id,
            "name": restrictive.name,
            "builtin": true,
            "capabilities": {
                "fs_read": restrictive.capabilities.fs_read,
                "fs_write": restrictive.capabilities.fs_write,
                "net": restrictive.capabilities.net,
                "env": restrictive.capabilities.env,
            },
            "limits": {
                "memory_mb": restrictive.limits.memory_mb,
                "cpu_seconds": restrictive.limits.cpu_seconds,
                "wall_clock_seconds": restrictive.limits.wall_clock_seconds,
            }
        }),
        json!({
            "id": permissive.id,
            "name": permissive.name,
            "builtin": true,
            "capabilities": {
                "fs_read": permissive.capabilities.fs_read,
                "fs_write": permissive.capabilities.fs_write,
                "net": permissive.capabilities.net,
                "env": permissive.capabilities.env,
            },
            "limits": {
                "memory_mb": permissive.limits.memory_mb,
                "cpu_seconds": permissive.limits.cpu_seconds,
                "wall_clock_seconds": permissive.limits.wall_clock_seconds,
            }
        }),
    ];

    // Append user-created policies from persistent store
    for entry in state_guard.sandbox_policies.iter() {
        let mut p = entry.value().clone();
        if let Some(obj) = p.as_object_mut() {
            obj.insert("builtin".to_string(), json!(false));
        }
        policies.push(p);
    }

    Json(json!({ "policies": policies }))
}

/// Create a sandbox policy (persisted to disk)
pub async fn create_sandbox_policy(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let policy = zeus_sandbox::SandboxPolicy::new(name);

    let policy_json = json!({
        "id": policy.id,
        "name": policy.name,
        "created_at": policy.created_at.to_rfc3339(),
    });

    // Persist to DashMap + disk
    let state_guard = state.read().await;
    state_guard
        .sandbox_policies
        .insert(policy.id.clone(), policy_json.clone());

    // Save all custom policies to disk
    let policies: Vec<Value> = state_guard
        .sandbox_policies
        .iter()
        .map(|e| e.value().clone())
        .collect();
    let path = state_guard.sandbox_policies_path.clone();
    drop(state_guard);

    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let json_str = serde_json::to_string_pretty(&policies).unwrap_or_default();
    let _ = tokio::fs::write(&path, &json_str).await;

    Ok((StatusCode::CREATED, Json(policy_json)))
}

/// Execute code in sandbox
pub async fn sandbox_execute(Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, String)> {
    use tracing::debug;

    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'code' field".to_string()))?;

    let language = body
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("wasm");

    let policy_type = body
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or("restrictive");

    debug!(language, policy_type, "Sandbox execution request");

    let lang = match language {
        "typescript" | "ts" => zeus_sandbox::ExecutionLanguage::TypeScript,
        "javascript" | "js" => zeus_sandbox::ExecutionLanguage::JavaScript,
        _ => zeus_sandbox::ExecutionLanguage::Wasm,
    };

    let request = zeus_sandbox::ExecutionRequest {
        code: code.to_string(),
        language: lang,
        policy_id: None,
        stdin: body
            .get("stdin")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    // Actually attempt execution via SandboxEngine
    let engine = zeus_sandbox::SandboxEngine::new().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to initialize sandbox engine: {}", e),
        )
    })?;

    match engine.execute(request).await {
        Ok(result) => Ok(Json(json!({
            "status": format!("{:?}", result.status).to_lowercase(),
            "id": result.id,
            "language": result.language.to_string(),
            "policy": policy_type,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
            "duration_ms": result.duration_ms,
        }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Sandbox execution failed: {}", e),
        )),
    }
}
