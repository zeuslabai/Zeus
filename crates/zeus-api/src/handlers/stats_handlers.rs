//! Stats, activity, diagnostics, and pipeline handlers

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use crate::handlers::chat_handlers::ActivityQuery;
use serde::Deserialize;
use serde_json::{Value, json};
use zeus_llm::LlmClient;
use zeus_session::Session;

use crate::SharedState;

#[derive(Deserialize)]
pub struct TokenCountRequest {
    #[serde(default)]
    pub messages: Vec<zeus_core::Message>,
    #[serde(default)]
    pub text: Option<String>,
}

pub async fn count_tokens(Json(req): Json<TokenCountRequest>) -> Json<Value> {
    let count = if let Some(text) = &req.text {
        LlmClient::count_tokens_str(text)
    } else {
        LlmClient::count_tokens(&req.messages)
    };

    Json(json!({
        "tokens": count,
        "method": "estimate",
        "chars_per_token": 4
    }))
}

pub async fn get_activity(
    State(state): State<SharedState>,
    Query(params): Query<ActivityQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let limit = params.limit.unwrap_or(50).min(zeus_core::MAX_PAGE_LIMIT);

    let daily = state.workspace.get_daily().await.unwrap_or_default();

    let mut events: Vec<Value> = Vec::new();
    let today = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    for (i, line) in daily.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("---") {
            continue;
        }

        let (event_type, message) = if let Some(content) = line.strip_prefix("- ") {
            if content.contains("[Webhook]") {
                ("webhook", content.to_string())
            } else if content.contains("tool") || content.contains("Tool") {
                ("tool_call", content.to_string())
            } else {
                ("note", content.to_string())
            }
        } else {
            continue;
        };

        events.push(json!({
            "id": format!("evt-{}", i + 1),
            "type": event_type,
            "message": message,
            "timestamp": today
        }));

        if events.len() >= limit {
            break;
        }
    }

    Ok(Json(json!({ "events": events })))
}

pub async fn get_stats(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Never block on a cook's write guard — return "busy" immediately on
    // contention rather than starving the reader (tokio RwLock is
    // writer-preferring). Mirrors the /v1/status lock-free fix.
    let Ok(state) = state.try_read() else {
        return Ok(Json(json!({ "status": "busy" })));
    };

    let (provider, model) = state.config.parse_model();

    // Snapshot what we need, then drop the guard before the FS scans so
    // Session::list()/workspace IO never run while holding the lock.
    let sessions_path = state.config.sessions.clone();
    let workspace = state.workspace.clone();
    let tools_count = state.tools.schemas().len();
    drop(state);

    let sessions = Session::list(&sessions_path)
        .await
        .unwrap_or_default();

    let workspace_files = workspace.list("").await.unwrap_or_default().len();

    let memory_size = workspace
        .get_memory()
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(Json(json!({
        "sessions": {
            "total": sessions.len(),
            "active": 0
        },
        "tools": {
            "total": tools_count,
            "custom": 0
        },
        "memory": {
            "workspace_files": workspace_files,
            "memory_size_bytes": memory_size
        },
        "model": format!("{}/{}", format!("{:?}", provider).to_lowercase(), model),
        "provider": format!("{:?}", provider)
    })))
}

pub async fn doctor(State(state): State<SharedState>) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let mut checks: Vec<Value> = Vec::new();
    let mut has_error = false;
    let mut has_warning = false;

    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("config.toml");
    if config_path.exists() {
        checks.push(json!({
            "name": "Config file",
            "status": "ok",
            "detail": config_path.display().to_string()
        }));
    } else {
        has_warning = true;
        checks.push(json!({
            "name": "Config file",
            "status": "warning",
            "detail": "No config file found, using defaults"
        }));
    }

    let ws_root = state.workspace.root();
    if ws_root.exists() {
        let file_count = state.workspace.list("").await.unwrap_or_default().len();
        checks.push(json!({
            "name": "Workspace",
            "status": "ok",
            "detail": format!("{} files", file_count)
        }));
    } else {
        has_warning = true;
        checks.push(json!({
            "name": "Workspace",
            "status": "warning",
            "detail": format!("{} does not exist", ws_root.display())
        }));
    }

    if state.config.sessions.exists() {
        let count = Session::list(&state.config.sessions)
            .await
            .map(|s| s.len())
            .unwrap_or(0);
        checks.push(json!({
            "name": "Sessions",
            "status": "ok",
            "detail": format!("{} sessions", count)
        }));
    } else {
        has_warning = true;
        checks.push(json!({
            "name": "Sessions",
            "status": "warning",
            "detail": format!("{} does not exist", state.config.sessions.display())
        }));
    }

    let (provider, _) = state.config.parse_model();
    let cred_check = match provider {
        zeus_core::Provider::Anthropic => {
            if state.config.auth.use_oauth {
                ("ok", "OAuth enabled for Anthropic")
            } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                ("ok", "ANTHROPIC_API_KEY is set")
            } else {
                has_warning = true;
                (
                    "warning",
                    "No API key for Anthropic (ANTHROPIC_API_KEY not set, OAuth not enabled)",
                )
            }
        }
        zeus_core::Provider::Ollama => ("ok", "Ollama does not require API key"),
        _ => {
            let warnings = state.config.validate();
            if warnings.iter().any(|w| w.contains("not set")) {
                has_warning = true;
                ("warning", "API key not set for selected provider")
            } else {
                ("ok", "API key configured")
            }
        }
    };
    checks.push(json!({
        "name": "Credentials",
        "status": cred_check.0,
        "detail": cred_check.1
    }));

    if state.config.model.is_empty() {
        has_error = true;
        checks.push(json!({
            "name": "Model",
            "status": "error",
            "detail": "Model string is empty"
        }));
    } else if !state.config.model.contains('/') {
        has_warning = true;
        checks.push(json!({
            "name": "Model",
            "status": "warning",
            "detail": format!("Model '{}' has no provider prefix", state.config.model)
        }));
    } else {
        checks.push(json!({
            "name": "Model",
            "status": "ok",
            "detail": state.config.model.clone()
        }));
    }

    if matches!(provider, zeus_core::Provider::Ollama) {
        match reqwest_check_ollama(&state.config.ollama.url).await {
            true => {
                checks.push(json!({
                    "name": "Ollama",
                    "status": "ok",
                    "detail": format!("Reachable at {}", state.config.ollama.url)
                }));
            }
            false => {
                has_error = true;
                checks.push(json!({
                    "name": "Ollama",
                    "status": "error",
                    "detail": format!("Connection refused at {}", state.config.ollama.url)
                }));
            }
        }
    }

    let overall = if has_error {
        "error"
    } else if has_warning {
        "warning"
    } else {
        "ok"
    };

    Ok(Json(json!({
        "checks": checks,
        "overall": overall
    })))
}

async fn reqwest_check_ollama(url: &str) -> bool {
    let addr = url
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    tokio::net::TcpStream::connect(addr).await.is_ok()
}

/// GET /v1/pipeline/stats — Pipeline stage metrics
pub async fn pipeline_stats() -> Json<Value> {
    let stages = vec![
        json!({ "name": "input", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
        json!({ "name": "context", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
        json!({ "name": "llm", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
        json!({ "name": "tool_exec", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
        json!({ "name": "memory", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
        json!({ "name": "output", "messages_processed": 0, "avg_latency_ms": 0, "error_count": 0 }),
    ];
    Json(json!({
        "stages": stages,
        "total_messages": 0,
        "uptime_seconds": 0
    }))
}
