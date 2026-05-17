//! Configuration API handlers.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::SharedState;

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ConfigUpdateRequest {
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub sessions: Option<String>,
    pub max_iterations: Option<usize>,
    pub tui: Option<Value>,
    pub ollama: Option<Value>,
    pub providers: Option<Value>,
    pub default_provider: Option<String>,
    pub mnemosyne: Option<Value>,
    pub athena: Option<Value>,
    pub aegis: Option<Value>,
    pub hermes: Option<Value>,
    pub prometheus: Option<Value>,
    pub nous: Option<Value>,
    pub talos: Option<Value>,
    pub channels: Option<Value>,
    pub hooks: Option<Value>,
    pub search: Option<Value>,
    pub gateway: Option<Value>,
    pub session_compaction: Option<Value>,
    pub thinking_level: Option<String>,
    pub suppress_tool_errors: Option<bool>,
    pub name: Option<String>,
    pub persona: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TestProviderRequest {
    pub provider: String,
    pub api_key: Option<String>,
    pub url: Option<String>,
    pub model: Option<String>,
}

// ============================================================================
// Config Endpoints
// ============================================================================

/// Sanitize a config Value by replacing sensitive fields with "***"
fn sanitize_config(val: &mut Value) {
    let sensitive_keys = [
        "api_key",
        "api_hash",
        "token",
        "bot_token",
        "app_token",
        "access_token",
        "refresh_token",
        "password",
        "secret",
        "auth_token",
    ];

    match val {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                let key_lower = key.to_lowercase();
                if sensitive_keys.iter().any(|s| key_lower.contains(s)) {
                    if v.is_string() {
                        *v = Value::String("***".to_string());
                    }
                } else {
                    sanitize_config(v);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                sanitize_config(v);
            }
        }
        _ => {}
    }
}

pub async fn get_config(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let mut val = serde_json::to_value(&state.config)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sanitize_config(&mut val);

    // Add runtime embedding fallback state if Mnemosyne is available
    if let Some(ref mn) = state.mnemosyne {
        let active = mn.active_embedding_provider().await;
        let fallback = mn.embedding_fallback_state().await;
        let fallback_entries: Vec<Value> = fallback
            .iter()
            .map(|(name, failures, is_active)| {
                serde_json::json!({
                    "provider": name,
                    "failures": failures,
                    "active": is_active,
                })
            })
            .collect();

        if let Some(obj) = val.as_object_mut() {
            obj.insert(
                "embedding_status".to_string(),
                serde_json::json!({
                    "active_provider": active,
                    "fallback_chain": fallback_entries,
                }),
            );
        }
    }

    Ok(Json(val))
}

pub async fn update_config(
    State(state): State<SharedState>,
    Json(req): Json<ConfigUpdateRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Input validation
    if let Some(ref model) = req.model
        && (model.len() > 200 || model.contains('\0'))
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid model string".to_string()));
    }
    if let Some(ref workspace) = req.workspace {
        if workspace.contains('\0') {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid workspace path".to_string(),
            ));
        }
        // Resolve symlinks/.. via canonicalize when the path already exists
        let wp = std::path::Path::new(workspace.as_str());
        if let Ok(resolved) = tokio::fs::canonicalize(wp).await {
            if resolved.to_string_lossy().contains("..") {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid workspace path".to_string(),
                ));
            }
        } else if workspace.contains("..") {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid workspace path".to_string(),
            ));
        }
    }
    if let Some(ref sessions) = req.sessions {
        if sessions.contains('\0') {
            return Err((StatusCode::BAD_REQUEST, "Invalid sessions path".to_string()));
        }
        let sp = std::path::Path::new(sessions.as_str());
        if let Ok(resolved) = tokio::fs::canonicalize(sp).await {
            if resolved.to_string_lossy().contains("..") {
                return Err((StatusCode::BAD_REQUEST, "Invalid sessions path".to_string()));
            }
        } else if sessions.contains("..") {
            return Err((StatusCode::BAD_REQUEST, "Invalid sessions path".to_string()));
        }
    }
    if let Some(max_iter) = req.max_iterations
        && max_iter > 1000
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "max_iterations too high (max 1000)".to_string(),
        ));
    }
    if let Some(ref name) = req.name
        && (name.len() > 200 || name.contains('\0'))
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid agent name".to_string()));
    }
    if let Some(ref persona) = req.persona
        && (persona.len() > 2000 || persona.contains('\0'))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid persona string".to_string(),
        ));
    }

    let mut state = state.write().await;

    if let Some(model) = &req.model {
        if model.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Model cannot be empty".to_string()));
        }
        state.config.model = model.clone();
    }
    if let Some(workspace) = &req.workspace {
        let expanded = if workspace.starts_with('~') {
            dirs::home_dir()
                .unwrap_or_default()
                .join(workspace.trim_start_matches("~/"))
        } else {
            std::path::PathBuf::from(workspace)
        };
        state.config.workspace = expanded;
    }
    if let Some(sessions) = &req.sessions {
        let expanded = if sessions.starts_with('~') {
            dirs::home_dir()
                .unwrap_or_default()
                .join(sessions.trim_start_matches("~/"))
        } else {
            std::path::PathBuf::from(sessions)
        };
        state.config.sessions = expanded;
    }
    if let Some(max_iter) = req.max_iterations {
        state.config.max_iterations = max_iter;
    }
    if let Some(tui) = &req.tui {
        if let Some(theme) = tui.get("theme").and_then(|v| v.as_str()) {
            state.config.tui.theme = theme.to_string();
        }
        if let Some(vim) = tui.get("vim_mode").and_then(|v| v.as_bool()) {
            state.config.tui.vim_mode = vim;
        }
    }
    if let Some(ollama) = &req.ollama
        && let Some(url) = ollama.get("url").and_then(|v| v.as_str())
    {
        state.config.ollama.url = url.to_string();
    }

    // Subsystem configs: deserialize from JSON Value into typed config structs
    if let Some(v) = &req.mnemosyne {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.mnemosyne = Some(cfg),
            Err(e) => warn!("Invalid mnemosyne config: {}", e),
        }
    }
    if let Some(v) = &req.athena {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.athena = Some(cfg),
            Err(e) => warn!("Invalid athena config: {}", e),
        }
    }
    if let Some(v) = &req.aegis {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.aegis = Some(cfg),
            Err(e) => warn!("Invalid aegis config: {}", e),
        }
    }
    if let Some(v) = &req.hermes {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.hermes = Some(cfg),
            Err(e) => warn!("Invalid hermes config: {}", e),
        }
    }
    if let Some(v) = &req.prometheus {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.prometheus = Some(cfg),
            Err(e) => warn!("Invalid prometheus config: {}", e),
        }
    }
    if let Some(v) = &req.nous {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.nous = Some(cfg),
            Err(e) => warn!("Invalid nous config: {}", e),
        }
    }
    if let Some(v) = &req.talos {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.talos = Some(cfg),
            Err(e) => warn!("Invalid talos config: {}", e),
        }
    }
    if let Some(v) = &req.channels {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.channels = Some(cfg),
            Err(e) => warn!("Invalid channels config: {}", e),
        }
    }
    if let Some(v) = &req.hooks {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.hooks = Some(cfg),
            Err(e) => warn!("Invalid hooks config: {}", e),
        }
    }
    if let Some(v) = &req.search {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.search = Some(cfg),
            Err(e) => warn!("Invalid search config: {}", e),
        }
    }
    if let Some(v) = &req.gateway {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.gateway = Some(cfg),
            Err(e) => warn!("Invalid gateway config: {}", e),
        }
    }
    if let Some(v) = &req.session_compaction {
        match serde_json::from_value(v.clone()) {
            Ok(cfg) => state.config.session_compaction = Some(cfg),
            Err(e) => warn!("Invalid session_compaction config: {}", e),
        }
    }
    if let Some(v) = &req.thinking_level {
        state.config.thinking_level = Some(v.clone());
    }
    if let Some(suppress) = req.suppress_tool_errors {
        state.config.suppress_tool_errors = suppress;
    }
    if let Some(ref name) = req.name {
        state.config.name = Some(name.clone());
    }
    if let Some(ref persona) = req.persona {
        state.config.persona = Some(persona.clone());
    }

    // Save providers config to separate file
    if req.providers.is_some() || req.default_provider.is_some() {
        let providers_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join("providers.json");

        let mut providers_data = if providers_path.exists() {
            let content = tokio::fs::read_to_string(&providers_path)
                .await
                .unwrap_or_default();
            serde_json::from_str::<Value>(&content).unwrap_or(json!({}))
        } else {
            json!({})
        };

        if let Some(providers) = &req.providers {
            providers_data["providers"] = providers.clone();
        }
        if let Some(dp) = &req.default_provider {
            providers_data["default_provider"] = Value::String(dp.clone());
            // Also update the main config model to use the selected provider
            if let Some(providers) = providers_data.get("providers")
                && let Some(provider_cfg) = providers.get(dp.as_str())
                && let Some(model) = provider_cfg.get("model").and_then(|m| m.as_str())
            {
                let model_str = format!("{}/{}", dp, model);
                if model_str.ends_with('/') || model.is_empty() {
                    return Err((StatusCode::BAD_REQUEST, "Provider model cannot be empty".to_string()));
                }
                state.config.model = model_str;
            }
        }

        // Sync Ollama URL from providers payload to config.toml
        if let Some(providers_obj) = providers_data.get("providers")
            && let Some(ollama_cfg) = providers_obj.get("ollama")
            && let Some(url) = ollama_cfg.get("url").and_then(|u| u.as_str())
            && !url.is_empty()
        {
            state.config.ollama.url = url.to_string();
        }

        if let Some(parent) = providers_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        tokio::fs::write(
            &providers_path,
            serde_json::to_string_pretty(&providers_data).unwrap_or_default(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Persist config — guard errors (temp workspace, loaded-from-default) are
    // expected in test environments and should not fail the request.
    if let Err(e) = state.config.save() {
        let msg = e.to_string();
        if msg.contains("temp directory")
            || msg.contains("loaded from defaults")
            || msg.contains("onboarding_complete")
        {
            warn!("Config save skipped (guard): {}", msg);
        } else {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    }

    Ok(Json(json!({
        "success": true,
        "message": "Configuration updated"
    })))
}

/// GET /v1/config/providers — Get saved provider configuration
pub async fn get_providers() -> Json<Value> {
    let providers_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("providers.json");

    if providers_path.exists() {
        let content = tokio::fs::read_to_string(&providers_path)
            .await
            .unwrap_or_default();
        let data: Value = serde_json::from_str(&content).unwrap_or(json!({
            "providers": {},
            "default_provider": "ollama"
        }));
        Json(data)
    } else {
        // Return defaults based on environment variables
        let mut providers = json!({});
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            providers["anthropic"] = json!({ "configured": true });
        }
        if std::env::var("OPENAI_API_KEY").is_ok() {
            providers["openai"] = json!({ "configured": true });
        }
        if std::env::var("GOOGLE_API_KEY").is_ok() {
            providers["google"] = json!({ "configured": true });
        }
        Json(json!({
            "providers": providers,
            "default_provider": "ollama"
        }))
    }
}

/// GET /v1/providers/ollama/health — Check Ollama availability, models, and version
pub async fn ollama_health() -> Json<Value> {
    let config = zeus_core::Config::load().unwrap_or_default();
    let client = zeus_llm::ollama::OllamaClient::new(&config.ollama.url);

    let available = client.is_available().await;
    let version = if available {
        client.version().await.ok()
    } else {
        None
    };
    let models = if available {
        client.list_models().await.unwrap_or_default()
    } else {
        vec![]
    };
    let loaded: Vec<String> = models.iter()
        .map(|m| m.name.clone())
        .collect();

    Json(serde_json::json!({
        "provider": "ollama",
        "url": config.ollama.url,
        "available": available,
        "version": version,
        "models": loaded,
        "model_count": loaded.len(),
    }))
}

/// POST /v1/config/reload — Re-read config.toml from disk and update running state
pub async fn reload_config(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    match crate::config_watcher::reload_config(&state, "api_request").await {
        Ok(changed) => Ok(Json(json!({
            "success": true,
            "changed_keys": changed,
            "message": if changed.is_empty() {
                "No changes detected".to_string()
            } else {
                format!("Config reloaded, {} key(s) changed", changed.len())
            }
        }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// GET /v1/config/history — Return last 10 config changes with timestamps
pub async fn config_history(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let entries: Vec<Value> = state
        .config_history
        .entries()
        .iter()
        .rev() // most recent first
        .map(|e| {
            json!({
                "timestamp": e.timestamp.to_rfc3339(),
                "source": e.source,
                "changed_keys": e.changed_keys,
            })
        })
        .collect();

    Json(json!({
        "history": entries,
        "count": entries.len(),
    }))
}

/// POST /v1/config/test — Test a provider connection
pub async fn test_provider(
    State(state): State<SharedState>,
    Json(req): Json<TestProviderRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    match req.provider.as_str() {
        "anthropic" => {
            let key = req
                .api_key
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .unwrap_or_default();
            if key.is_empty() {
                return Ok(Json(json!({
                    "success": false,
                    "provider": "anthropic",
                    "error": "No API key provided or ANTHROPIC_API_KEY not set"
                })));
            }
            // Test with a minimal API call — detect OAuth vs API key by prefix
            let is_oauth = key.starts_with("sk-ant-oat");
            let client = {
                let s = state.read().await;
                s.http_client.clone()
            };
            let mut request = client
                .post("https://api.anthropic.com/v1/messages")
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");
            // OAuth tokens use Bearer auth; API keys use x-api-key
            if is_oauth {
                request = request
                    .header("Authorization", format!("Bearer {}", key))
                    .header("anthropic-beta", "oauth-2025-04-20");
            } else {
                request = request.header("x-api-key", &key);
            }
            let resp = request
                .body(r#"{"model":"claude-haiku-4-5-20251001","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#)
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if status == 200 {
                        // Return default Anthropic models so WebUI can populate the picker
                        Ok(Json(json!({
                            "success": true,
                            "provider": "anthropic",
                            "status": "connected",
                            "auth_type": if is_oauth { "oauth" } else { "api_key" },
                            "models": [
                                "anthropic/claude-sonnet-4-6",
                                "anthropic/claude-opus-4-6",
                                "anthropic/claude-haiku-4-5-20251001"
                            ]
                        })))
                    } else if status == 401 {
                        Ok(Json(
                            json!({ "success": false, "provider": "anthropic", "error": "Invalid API key" }),
                        ))
                    } else {
                        Ok(Json(
                            json!({ "success": false, "provider": "anthropic", "error": format!("HTTP {}", status) }),
                        ))
                    }
                }
                Err(e) => Ok(Json(
                    json!({ "success": false, "provider": "anthropic", "error": e.to_string() }),
                )),
            }
        }
        "openai" => {
            let key = req
                .api_key
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_default();
            if key.is_empty() {
                return Ok(Json(
                    json!({ "success": false, "provider": "openai", "error": "No API key" }),
                ));
            }
            let client = {
                let s = state.read().await;
                s.http_client.clone()
            };
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if status == 200 {
                        Ok(Json(
                            json!({ "success": true, "provider": "openai", "status": "connected" }),
                        ))
                    } else {
                        Ok(Json(
                            json!({ "success": false, "provider": "openai", "error": format!("HTTP {}", status) }),
                        ))
                    }
                }
                Err(e) => Ok(Json(
                    json!({ "success": false, "provider": "openai", "error": e.to_string() }),
                )),
            }
        }
        "google" => {
            let key = req
                .api_key
                .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
                .unwrap_or_default();
            if key.is_empty() {
                return Ok(Json(
                    json!({ "success": false, "provider": "google", "error": "No API key" }),
                ));
            }
            let client = {
                let s = state.read().await;
                s.http_client.clone()
            };
            let resp = client
                .get(format!(
                    "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                    key
                ))
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if status == 200 {
                        Ok(Json(
                            json!({ "success": true, "provider": "google", "status": "connected" }),
                        ))
                    } else {
                        Ok(Json(
                            json!({ "success": false, "provider": "google", "error": format!("HTTP {}", status) }),
                        ))
                    }
                }
                Err(e) => Ok(Json(
                    json!({ "success": false, "provider": "google", "error": e.to_string() }),
                )),
            }
        }
        "ollama" => {
            let configured_url = state.read().await.config.ollama.url.clone();
            let url = req.url.unwrap_or(configured_url).trim().to_string();
            let client = {
                let s = state.read().await;
                s.http_client.clone()
            };
            let resp = client
                .get(format!("{}/api/tags", url))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if status == 200 {
                        // Parse models list
                        let body: Value = r.json().await.unwrap_or(json!({}));
                        let models: Vec<String> = body
                            .get("models")
                            .and_then(|m| m.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|m| {
                                        m.get("name").and_then(|n| n.as_str()).map(String::from)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Ok(Json(json!({
                            "success": true,
                            "provider": "ollama",
                            "status": "connected",
                            "models": models
                        })))
                    } else {
                        Ok(Json(
                            json!({ "success": false, "provider": "ollama", "error": format!("HTTP {}", status) }),
                        ))
                    }
                }
                Err(e) => Ok(Json(
                    json!({ "success": false, "provider": "ollama", "error": e.to_string() }),
                )),
            }
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            format!("Unknown provider: {}", req.provider),
        )),
    }
}

// ============================================================================
// Model Listing (for native app onboarding)
// ============================================================================

/// POST /v1/config/models — Fetch available models from a provider using supplied credentials
///
/// Request: `{ "provider": "anthropic", "api_key": "sk-..." }`
/// Response: `{ "models": [{ "id": "claude-sonnet-4-20250514" }, ...] }`
pub async fn fetch_provider_models(
    State(state): State<SharedState>,
    Json(req): Json<TestProviderRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let client = {
        let s = state.read().await;
        s.http_client.clone()
    };
    let key = req.api_key.unwrap_or_default();
    let timeout = std::time::Duration::from_secs(10);

    match req.provider.as_str() {
        "anthropic" => {
            if key.is_empty() {
                return Ok(Json(json!({ "models": [], "error": "No API key provided" })));
            }
            let is_oauth = key.starts_with("sk-ant-oat01-");
            let mut r = client.get("https://api.anthropic.com/v1/models")
                .header("anthropic-version", "2023-06-01")
                .timeout(timeout);
            if is_oauth {
                r = r.header("Authorization", format!("Bearer {}", key))
                    .header("anthropic-beta", "oauth-2025-04-20");
            } else {
                r = r.header("x-api-key", &key);
            }
            match r.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(|s| json!({ "id": s })))
                            .collect())
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "openai" => {
            if key.is_empty() {
                return Ok(Json(json!({ "models": [], "error": "No API key provided" })));
            }
            match client.get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {}", key))
                .timeout(timeout)
                .send().await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            let mut ids: Vec<Value> = arr.iter()
                                .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(|s| s.to_string()))
                                .filter(|id| !id.contains("embedding") && !id.contains("tts") && !id.contains("whisper") && !id.contains("dall-e"))
                                .map(|s| json!({ "id": s }))
                                .collect();
                            ids.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
                            ids.dedup_by(|a, b| a["id"] == b["id"]);
                            ids
                        })
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "google" => {
            if key.is_empty() {
                return Ok(Json(json!({ "models": [], "error": "No API key provided" })));
            }
            match client.get(format!("https://generativelanguage.googleapis.com/v1beta/models?key={}", key))
                .timeout(timeout)
                .send().await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("models")
                        .and_then(|d| d.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|m| m.get("name").and_then(|n| n.as_str())
                                .map(|s| json!({ "id": s.strip_prefix("models/").unwrap_or(s) })))
                            .collect())
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "groq" | "mistral" | "together" | "fireworks" | "openrouter" => {
            if key.is_empty() {
                return Ok(Json(json!({ "models": [], "error": "No API key provided" })));
            }
            let url = match req.provider.as_str() {
                "groq" => "https://api.groq.com/openai/v1/models",
                "mistral" => "https://api.mistral.ai/v1/models",
                "together" => "https://api.together.xyz/v1/models",
                "fireworks" => "https://api.fireworks.ai/inference/v1/models",
                "openrouter" => "https://openrouter.ai/api/v1/models",
                _ => unreachable!(),
            };
            match client.get(url)
                .header("Authorization", format!("Bearer {}", key))
                .timeout(timeout)
                .send().await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(|s| json!({ "id": s })))
                            .collect())
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "ollama" => {
            let configured_url = state.read().await.config.ollama.url.clone();
            let url = req.url.unwrap_or(configured_url).trim().to_string();
            match client.get(format!("{}/api/tags", url))
                .timeout(timeout)
                .send().await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("models")
                        .and_then(|m| m.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| json!({ "id": s })))
                            .collect())
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "azure" => {
            let endpoint = req.url.unwrap_or_default();
            if key.is_empty() || endpoint.is_empty() {
                return Ok(Json(json!({ "models": [], "error": "Missing API key or endpoint URL" })));
            }
            match client.get(format!("{}/openai/models?api-version=2024-02-01", endpoint))
                .header("api-key", &key)
                .timeout(timeout)
                .send().await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await.unwrap_or(json!({}));
                    let models: Vec<Value> = body.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(|s| json!({ "id": s })))
                            .collect())
                        .unwrap_or_default();
                    Ok(Json(json!({ "models": models })))
                }
                Ok(resp) => Ok(Json(json!({ "models": [], "error": format!("HTTP {}", resp.status()) }))),
                Err(e) => Ok(Json(json!({ "models": [], "error": e.to_string() }))),
            }
        }
        "bedrock" => {
            // Bedrock requires AWS SDK — return empty with instruction
            Ok(Json(json!({ "models": [], "error": "Bedrock model listing requires AWS SDK. Enter model ID manually." })))
        }
        _ => Err((StatusCode::BAD_REQUEST, format!("Unknown provider: {}", req.provider))),
    }
}
