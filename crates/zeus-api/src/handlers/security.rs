//! Security endpoint handlers — threats, permissions, keys, allowlist, audit.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::SharedState;

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdatePermissionsRequest {
    pub shell_access: Option<bool>,
    pub file_write: Option<bool>,
    pub web_access: Option<bool>,
    pub level: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAllowlistRequest {
    pub allowlist: Vec<String>,
}

/// Query parameters for the security audit endpoint.
#[derive(Debug, Deserialize)]
pub struct AuditQueryParams {
    pub tool: Option<String>,
    pub user: Option<String>,
    pub severity: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
    pub alerts: Option<String>,
}

// ============================================================================
// Security Endpoints
// ============================================================================

/// GET /v1/security/threats — Threat log
pub async fn security_threats() -> Json<Value> {
    // Read from aegis audit log if available
    let audit_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("audit.log");

    let mut threats = Vec::new();

    if audit_path.exists()
        && let Ok(content) = tokio::fs::read_to_string(&audit_path).await
    {
        for (i, line) in content.lines().enumerate() {
            // Audit entries are JSON lines — look for blocked/denied events
            if let Ok(entry) = serde_json::from_str::<Value>(line) {
                let event = entry.get("event").cloned().unwrap_or(Value::Null);

                // Check if this is a security-relevant event
                let is_threat = event
                    .get("PermissionCheck")
                    .and_then(|p| p.get("allowed"))
                    .and_then(|a| a.as_bool())
                    == Some(false);

                if is_threat {
                    let detail = event
                        .get("PermissionCheck")
                        .and_then(|p| p.get("operation"))
                        .and_then(|o| o.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let timestamp = entry
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();

                    threats.push(json!({
                        "id": format!("t{}", i + 1),
                        "type": "blocked_permission",
                        "detail": detail,
                        "severity": "medium",
                        "timestamp": timestamp
                    }));
                }
            }
        }
    }

    Json(json!({ "threats": threats }))
}

/// GET /v1/security/permissions — Permission matrix (loaded from JSON file)
pub async fn security_permissions(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    // Read aegis level from config
    let level = state
        .config
        .aegis
        .as_ref()
        .map(|a| a.sandbox_level.clone())
        .unwrap_or_else(|| "standard".to_string());

    // Load persisted permissions if file exists, else defaults
    let perms = if state.permissions_path.exists() {
        match tokio::fs::read_to_string(&state.permissions_path).await {
            Ok(content) => serde_json::from_str::<Value>(&content).unwrap_or_default(),
            Err(_) => Value::Null,
        }
    } else {
        Value::Null
    };

    let shell_access = perms
        .get("shell_access")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let file_write = perms
        .get("file_write")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let web_access = perms
        .get("web_access")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let saved_level = perms
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or(&level);

    Json(json!({
        "global": {
            "shell_access": shell_access,
            "file_write": file_write,
            "web_access": web_access,
            "level": saved_level
        }
    }))
}

/// PUT /v1/security/permissions — Update permissions (persisted to JSON file)
pub async fn update_security_permissions(
    State(state): State<SharedState>,
    Json(req): Json<UpdatePermissionsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Load existing permissions
    let existing = if state.permissions_path.exists() {
        tokio::fs::read_to_string(&state.permissions_path)
            .await
            .ok()
            .and_then(|c| serde_json::from_str::<Value>(&c).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };

    // Merge updates
    let shell = req.shell_access.unwrap_or_else(|| {
        existing
            .get("shell_access")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    });
    let file_w = req.file_write.unwrap_or_else(|| {
        existing
            .get("file_write")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    });
    let web = req.web_access.unwrap_or_else(|| {
        existing
            .get("web_access")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    });
    let level = req.level.clone().unwrap_or_else(|| {
        existing
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("standard")
            .to_string()
    });

    let updated = json!({
        "shell_access": shell,
        "file_write": file_w,
        "web_access": web,
        "level": level,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });

    // Persist to file
    if let Some(parent) = state.permissions_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let json_str = serde_json::to_string_pretty(&updated).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize permissions: {e}"),
        )
    })?;
    tokio::fs::write(&state.permissions_path, &json_str)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write permissions: {e}"),
            )
        })?;

    info!(
        "Security permissions persisted to {}",
        state.permissions_path.display()
    );

    Ok(Json(json!({
        "success": true,
        "global": updated,
        "persisted_to": state.permissions_path.display().to_string(),
    })))
}

/// GET /v1/security/keys — API key inventory (without revealing values)
pub async fn security_keys() -> Json<Value> {
    let known_keys = vec![
        ("Anthropic", "ANTHROPIC_API_KEY"),
        ("OpenAI", "OPENAI_API_KEY"),
        ("OpenRouter", "OPENROUTER_API_KEY"),
        ("Google", "GOOGLE_API_KEY"),
        ("Groq", "GROQ_API_KEY"),
        ("Mistral", "MISTRAL_API_KEY"),
        ("Together", "TOGETHER_API_KEY"),
        ("Fireworks", "FIREWORKS_API_KEY"),
        ("Azure", "AZURE_OPENAI_API_KEY"),
        ("Bedrock", "AWS_ACCESS_KEY_ID"),
        ("Kimi", "MOONSHOT_API_KEY"),
        ("GLM", "ZAI_API_KEY"),
        ("Qwen", "QWEN_API_KEY"),
        ("MiniMax", "MINIMAX_API_KEY"),
        ("Twilio (SID)", "TWILIO_ACCOUNT_SID"),
        ("Twilio (Auth)", "TWILIO_AUTH_TOKEN"),
    ];

    let keys: Vec<Value> = known_keys
        .iter()
        .map(|(provider, env_var)| {
            let configured = std::env::var(env_var).is_ok();
            json!({
                "provider": provider,
                "env_var": env_var,
                "configured": configured,
                "source": if configured { "env" } else { "none" }
            })
        })
        .collect();

    Json(json!({ "keys": keys }))
}

/// GET /v1/security/allowlist — Shell command allowlist
pub async fn security_allowlist() -> Json<Value> {
    // Default sensible allowlist
    let default_allowlist = vec![
        "ls", "cat", "grep", "find", "git", "cargo", "npm", "node", "python", "ruby", "which",
        "echo", "pwd", "date", "wc", "sort", "head", "tail", "diff",
    ];

    // Try reading from aegis config (honors ZEUS_HOME for test/alt-home isolation)
    let allowlist_path = zeus_core::Config::zeus_home()
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".zeus"))
        .join("allowlist.json");

    let (allowlist, mode) = if allowlist_path.exists() {
        match tokio::fs::read_to_string(&allowlist_path).await {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(val) => {
                    let list = val
                        .get("allowlist")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| {
                            default_allowlist.iter().map(|s| s.to_string()).collect()
                        });
                    let mode = val
                        .get("mode")
                        .and_then(|m| m.as_str())
                        .unwrap_or("allowlist")
                        .to_string();
                    (list, mode)
                }
                Err(_) => (
                    default_allowlist.iter().map(|s| s.to_string()).collect(),
                    "allowlist".to_string(),
                ),
            },
            Err(_) => (
                default_allowlist.iter().map(|s| s.to_string()).collect(),
                "allowlist".to_string(),
            ),
        }
    } else {
        (
            default_allowlist.iter().map(|s| s.to_string()).collect(),
            "allowlist".to_string(),
        )
    };

    Json(json!({
        "allowlist": allowlist,
        "mode": mode
    }))
}

/// PUT /v1/security/allowlist — Update allowlist
pub async fn update_security_allowlist(
    Json(req): Json<UpdateAllowlistRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate allowlist entries
    if req.allowlist.len() > 500 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Allowlist too large (max 500 entries)".to_string(),
        ));
    }
    for entry in &req.allowlist {
        if entry.len() > 200 || entry.contains('\0') || entry.contains("..") {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "Invalid allowlist entry: '{}'",
                    &entry[..entry.len().min(50)]
                ),
            ));
        }
    }

    let allowlist_path = zeus_core::Config::zeus_home()
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".zeus"))
        .join("allowlist.json");

    if let Some(parent) = allowlist_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let data = json!({
        "allowlist": req.allowlist,
        "mode": "allowlist"
    });

    tokio::fs::write(
        &allowlist_path,
        serde_json::to_string_pretty(&data).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Updated security allowlist: {} commands",
        data["allowlist"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    Ok(Json(json!({
        "success": true,
        "message": "Allowlist updated"
    })))
}

/// GET /v1/security/audit — Query audit log with filtering
///
/// Query params:
///   - tool: filter by tool name (for ToolExecution events)
///   - user: filter by user identifier
///   - severity: minimum severity (info, warning, error, critical)
///   - since: ISO 8601 timestamp to filter entries after
///   - limit: max entries to return (default 100, max 1000)
///   - alerts: if "true", also run suspicious pattern detection
pub async fn security_audit(
    axum::extract::Query(params): axum::extract::Query<AuditQueryParams>,
) -> Json<Value> {
    let audit_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("audit.log");

    // Build the audit log reader
    let log = match zeus_aegis::AuditLog::new(&audit_path).await {
        Ok(l) => l,
        Err(e) => {
            return Json(json!({
                "entries": [],
                "error": format!("Failed to open audit log: {}", e),
                "total": 0
            }));
        }
    };

    // Parse severity filter
    let min_severity = params.severity.as_deref().and_then(|s| match s {
        "info" => Some(zeus_aegis::Severity::Info),
        "warning" => Some(zeus_aegis::Severity::Warning),
        "error" => Some(zeus_aegis::Severity::Error),
        "critical" => Some(zeus_aegis::Severity::Critical),
        _ => None,
    });

    // Parse since timestamp
    let since = params
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let limit = params.limit.unwrap_or(100).min(1000);

    let query = zeus_aegis::AuditQuery {
        tool: params.tool.clone(),
        user: params.user.clone(),
        min_severity,
        since,
        limit: Some(limit),
    };

    let entries = match log.query_entries(&query).await {
        Ok(e) => e,
        Err(e) => {
            return Json(json!({
                "entries": [],
                "error": format!("Failed to query audit log: {}", e),
                "total": 0
            }));
        }
    };

    // Serialize entries
    let entry_values: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "sequence": e.sequence,
                "timestamp": e.timestamp.to_rfc3339(),
                "severity": e.severity,
                "user": e.user,
                "event": e.event,
                "hash": &e.hash[..16], // truncated for display
            })
        })
        .collect();

    let total = entry_values.len();

    // Optionally detect suspicious patterns
    let alerts = if params.alerts.as_deref() == Some("true") {
        match log.detect_suspicious_patterns().await {
            Ok(a) => serde_json::to_value(&a).unwrap_or(json!([])),
            Err(_) => json!([]),
        }
    } else {
        json!([])
    };

    Json(json!({
        "entries": entry_values,
        "total": total,
        "alerts": alerts,
        "entry_count": log.entry_count(),
    }))
}
