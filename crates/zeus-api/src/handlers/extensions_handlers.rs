use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;
use crate::extensions::info_to_registry_extension;

// ============================================================================
// Extensions Endpoints
// ============================================================================

/// List extensions
pub async fn list_extensions(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let extensions = state.extension_store.list();
    let total = extensions.len();
    Json(json!({
        "extensions": extensions,
        "total": total,
    }))
}

/// Install an extension
pub async fn install_extension(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?
        .to_string();

    let source = body
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'source' field".to_string(),
            )
        })?
        .to_string();

    let version = body
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from);
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let permissions = body.get("permissions").and_then(|v| {
        serde_json::from_value::<crate::extensions::ExtensionPermissions>(v.clone()).ok()
    });

    let state = state.read().await;
    let ext = state
        .extension_store
        .install(name, source, version, description, permissions)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Register with the runtime registry so the extension can be started later.
    // We don't start it here — the caller uses POST /v1/extensions/:id/start for that.
    let reg_ext = info_to_registry_extension(&ext);
    let _ = state.extension_registry.register(reg_ext).await; // no-op if already registered

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(&ext).unwrap_or_default()),
    ))
}

/// Get extension details
pub async fn get_extension(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let ext = state.extension_store.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Extension not found: {}", id),
        )
    })?;
    Ok(Json(serde_json::to_value(&ext).unwrap_or_default()))
}

/// Update extension
pub async fn update_extension(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let enabled = body.get("enabled").and_then(|v| v.as_bool());
    let permissions = body.get("permissions").and_then(|v| {
        serde_json::from_value::<crate::extensions::ExtensionPermissions>(v.clone()).ok()
    });

    let state = state.read().await;
    let ext = state
        .extension_store
        .update(&id, enabled, permissions)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(serde_json::to_value(&ext).unwrap_or_default()))
}

/// Delete extension — stops the Deno subprocess (if running) before removing metadata.
pub async fn delete_extension(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Stop the subprocess before removing — ignore errors (may not be running).
    let _ = state.extension_registry.stop(&id).await;

    state
        .extension_store
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "id": id, "deleted": true })))
}

/// Start an extension — registers with runtime registry if needed, spawns Deno subprocess.
pub async fn start_extension(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Verify the extension exists in the store first.
    let info = state.extension_store.get(&id).ok_or_else(|| {
        (StatusCode::NOT_FOUND, format!("Extension not found: {}", id))
    })?;

    // Ensure the extension is registered in the runtime registry before starting.
    let reg_ext = info_to_registry_extension(&info);
    let _ = state.extension_registry.register(reg_ext).await; // no-op if already registered

    // Spawn the Deno subprocess.
    match state.extension_registry.start(&id).await {
        Ok(()) => {
            let ext = state
                .extension_store
                .set_status(&id, crate::extensions::ExtensionStatus::Running)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            Ok(Json(serde_json::to_value(&ext).unwrap_or_default()))
        }
        Err(e) => {
            // Mark as error in the store so the UI reflects the failure.
            let _ = state
                .extension_store
                .set_status(&id, crate::extensions::ExtensionStatus::Error)
                .await;
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to start extension: {}", e),
            ))
        }
    }
}

/// Stop an extension — kills the Deno subprocess and updates persisted status.
pub async fn stop_extension(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Stop the Deno subprocess (no-op if not running).
    let _ = state.extension_registry.stop(&id).await;

    let ext = state
        .extension_store
        .set_status(&id, crate::extensions::ExtensionStatus::Stopped)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(serde_json::to_value(&ext).unwrap_or_default()))
}

// ============================================================================
// Cost Routing Endpoints
// ============================================================================

/// GET /v1/routing/costs — Returns the cost table for all configured providers
pub async fn routing_costs(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let providers: Vec<Value> = state
        .cost_router
        .providers()
        .iter()
        .map(|p| {
            json!({
                "model": p.model,
                "cost_per_1k_input": p.cost_per_1k_input,
                "cost_per_1k_output": p.cost_per_1k_output,
                "max_tokens": p.max_tokens,
                "latency_ms_estimate": p.latency_ms_estimate,
                "max_tier": p.max_tier,
            })
        })
        .collect();

    Json(json!({ "providers": providers }))
}

/// GET /v1/routing/budget — Returns budget status and cost summary
pub async fn routing_budget(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let summary = state.cost_router.summary();
    let within_budget = state.cost_router.check_budget();

    Json(json!({
        "total_cost": summary.total_cost,
        "budget_remaining": summary.budget_remaining,
        "within_budget": within_budget,
        "monthly_budget": state.cost_router.monthly_budget(),
        "top_models": summary.top_models,
        "period_start": summary.period_start,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RecommendRequest {
    pub task: String,
}

/// POST /v1/routing/recommend — Classify a task description and recommend a model
pub async fn routing_recommend(
    State(state): State<SharedState>,
    Json(payload): Json<RecommendRequest>,
) -> Json<Value> {
    let state = state.read().await;
    let tier = crate::cost_router::CostRouter::classify_task(&payload.task);
    let recommendation = state.cost_router.recommend(tier);

    match recommendation {
        Some(provider) => Json(json!({
            "task": payload.task,
            "tier": tier,
            "recommended_model": provider.model,
            "cost_per_1k_input": provider.cost_per_1k_input,
            "cost_per_1k_output": provider.cost_per_1k_output,
            "max_tokens": provider.max_tokens,
            "latency_ms_estimate": provider.latency_ms_estimate,
        })),
        None => Json(json!({
            "task": payload.task,
            "tier": tier,
            "recommended_model": null,
            "error": "no model available for this tier",
        })),
    }
}

// ============================================================================
// Onboarding
// ============================================================================

/// GET /v1/onboarding/status — Check whether onboarding has been completed
pub async fn onboarding_status(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    if !state.config.onboarding_complete {
        return Json(json!({ "completed": false }));
    }

    let model = &state.config.model;
    let provider = model.split('/').next().unwrap_or("ollama");
    let has_credentials = match provider {
        "ollama" => true,
        "anthropic" => std::env::var("ANTHROPIC_API_KEY").is_ok() || state.config.auth.use_oauth,
        "openai" => std::env::var("OPENAI_API_KEY").is_ok(),
        "google" => std::env::var("GOOGLE_API_KEY").is_ok(),
        "groq" => std::env::var("GROQ_API_KEY").is_ok(),
        "openrouter" => std::env::var("OPENROUTER_API_KEY").is_ok(),
        "mistral" => std::env::var("MISTRAL_API_KEY").is_ok(),
        "together" => std::env::var("TOGETHER_API_KEY").is_ok(),
        "fireworks" => std::env::var("FIREWORKS_API_KEY").is_ok(),
        "azure" => std::env::var("AZURE_OPENAI_API_KEY").is_ok(),
        "bedrock" => std::env::var("AWS_ACCESS_KEY_ID").is_ok(),
        _ => true,
    };

    Json(json!({
        "completed": true,
        "has_credentials": has_credentials,
        "provider": provider,
        "model": model,
    }))
}

/// POST /v1/onboarding/complete — Mark onboarding as completed and persist to config
pub async fn onboarding_complete(
    State(state): State<SharedState>,
    body: Option<Json<Value>>,
) -> Json<Value> {
    let mut state = state.write().await;
    state.config.onboarding_complete = true;

    if let Some(Json(body)) = body {
        if let Some(model) = body.get("model").and_then(|m| m.as_str())
            && !model.is_empty()
        {
            let provider = body.get("provider").and_then(|p| p.as_str()).unwrap_or("");
            if !provider.is_empty() && !model.contains('/') {
                state.config.model = format!("{}/{}", provider, model);
            } else {
                state.config.model = model.to_string();
            }
        }

        if let Some(provider) = body.get("provider").and_then(|p| p.as_str())
            && provider == "ollama"
            && let Some(url) = body.get("url").and_then(|u| u.as_str())
            && !url.is_empty()
        {
            state.config.ollama.url = url.to_string();
        }
    }

    state.config.loaded_from_default = false;

    // #385: Generate API auth token on onboarding completion (legacy path).
    // Mirrors the same logic in onboarding_setup's complete=true branch.
    let generated_token: Option<String> = {
        let gateway = state.config.gateway.get_or_insert_with(Default::default);
        if gateway.api_token.is_none() {
            let token = crate::handlers::onboarding_handlers::generate_api_token();
            tracing::info!("#385: Generated API auth token during onboarding completion (legacy path)");
            gateway.api_token = Some(token.clone());
            Some(token)
        } else {
            None
        }
    };

    let _ = state.config.save();

    // S98: Do NOT process::exit() here — it kills the server before the HTTP response
    // reaches the WebUI client, causing an onboarding redirect loop.
    // The WebUI sets localStorage("zeus_onboarding_complete") on success and navigates
    // to "/". Config is already saved with onboarding_complete=true above.
    // If the user needs a full restart (for credential reload), they can run
    // `zeus daemon restart` manually, or the TUI onboarding handles it.

    // #220: shared with POST /v1/onboarding/setup (complete=true) — single
    // implementation in onboarding_handlers so both paths stay identical.
    crate::handlers::onboarding_handlers::generate_workspace_files(&state.config);

    Json(json!({
        "success": true,
        "auth_token": generated_token,
    }))
}


/// GET /v1/providers — Provider catalog for onboarding and provider selection UI.
///
/// Canonical provider set mirrors the TUI onboarding `PROVIDERS` list
/// (crates/zeus-tui/src/onboarding/mod.rs) — TUI is the source of truth (#216).
/// Models are NEVER hardcoded here: the UI fetches live model lists per
/// provider after credentials are configured (`models` stays empty).
pub async fn list_providers() -> Json<Value> {
    Json(json!({
        "providers": [
            {
                "id": "anthropic",
                "name": "Anthropic",
                "tagline": "Claude models — powerful, safe, steerable",
                "icon": "🧠",
                "color": "#d4a574",
                "auth_methods": ["subscription", "api_key"],
                "env_var": "ANTHROPIC_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "openai",
                "name": "OpenAI",
                "tagline": "GPT and reasoning models",
                "icon": "⚡",
                "color": "#74d4a5",
                "auth_methods": ["subscription", "api_key"],
                "env_var": "OPENAI_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "google",
                "name": "Google",
                "tagline": "Gemini API models",
                "icon": "🔷",
                "color": "#4285f4",
                "auth_methods": ["api_key"],
                "env_var": "GOOGLE_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "ollama",
                "name": "Ollama",
                "tagline": "Local models — no API key needed",
                "icon": "🦙",
                "color": "#74a5d4",
                "auth_methods": ["none"],
                "env_var": "OLLAMA_HOST",
                "requires_url": true,
                "default_url": "http://localhost:11434",
                "models": []
            },
            {
                "id": "google-gemini-cli",
                "name": "Gemini CLI",
                "tagline": "Code assist via Gemini CLI OAuth",
                "icon": "✦",
                "color": "#0f9d58",
                "auth_methods": ["oauth"],
                "env_var": "",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "moonshot",
                "name": "Kimi",
                "tagline": "Moonshot AI — K2.5 series",
                "icon": "🌙",
                "color": "#ff6b35",
                "auth_methods": ["api_key"],
                "env_var": "MOONSHOT_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "kimi-code",
                "name": "Kimi Code",
                "tagline": "Kimi Code subscription — K3 and coding models",
                "icon": "🌙",
                "color": "#ff8c42",
                "auth_methods": ["api_key"],
                "env_var": "KIMI_CODE_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": ["k3", "kimi-for-coding", "kimi-for-coding-highspeed"]
            },
            {
                "id": "glm-coding",
                "name": "GLM Coding",
                "tagline": "GLM Coding subscription — flat-rate coding plan",
                "icon": "🔮",
                "color": "#3fbfbf",
                "auth_methods": ["api_key"],
                "env_var": "GLM_CODING_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": ["glm-5.2", "glm-5-turbo", "glm-4.7"]
            },
            {
                "id": "zai",
                "name": "GLM",
                "tagline": "ZAI — GLM series models",
                "icon": "🔮",
                "color": "#e74c3c",
                "auth_methods": ["api_key"],
                "env_var": "ZAI_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "qwen",
                "name": "Qwen",
                "tagline": "Alibaba — device code OAuth",
                "icon": "🌀",
                "color": "#6c5ce7",
                "auth_methods": ["device_code", "api_key"],
                "env_var": "QWEN_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "qwen-coding",
                "name": "Qwen Coding",
                "tagline": "Qwen Coding Plan subscription — Max Preview, Coder",
                "icon": "🌀",
                "color": "#6c5ce7",
                "auth_methods": ["api_key"],
                "env_var": "QWEN_CODING_API_KEY",
                "requires_url": false,
                "default_url": "https://coding.dashscope.aliyuncs.com/v1",
                "models": ["qwen3.8-max-preview", "qwen3-coder"]
            },
            {
                "id": "minimax",
                "name": "MiniMax",
                "tagline": "Portal OAuth — Anthropic Messages API",
                "icon": "Ⓜ️",
                "color": "#fdcb6e",
                "auth_methods": ["device_code", "api_key"],
                "env_var": "MINIMAX_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "minimax-coding",
                "name": "MiniMax Coding",
                "tagline": "MiniMax Token Plan subscription — M2.5, M3, M2.7",
                "icon": "Ⓜ️",
                "color": "#f5b642",
                "auth_methods": ["api_key"],
                "env_var": "MINIMAX_CODING_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": ["MiniMax-M2.5", "MiniMax-M3", "MiniMax-M2.7"]
            },
            {
                "id": "xiaomimimo",
                "name": "MiMo",
                "tagline": "Xiaomi — MiMo models",
                "icon": "🍊",
                "color": "#ff8800",
                "auth_methods": ["api_key"],
                "env_var": "XIAOMIMIMO_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "openrouter",
                "name": "OpenRouter",
                "tagline": "Multi-provider routing — 200+ models",
                "icon": "🔀",
                "color": "#6c5ce7",
                "auth_methods": ["api_key"],
                "env_var": "OPENROUTER_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "xai",
                "name": "xAI",
                "tagline": "Grok models — xAI platform",
                "icon": "⚡",
                "color": "#ffd43b",
                "auth_methods": ["api_key"],
                "env_var": "XAI_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            },
            {
                "id": "sakana",
                "name": "Sakana",
                "tagline": "Sakana AI — Fugu series models",
                "icon": "🐟",
                "color": "#20c997",
                "auth_methods": ["api_key"],
                "env_var": "SAKANA_API_KEY",
                "requires_url": false,
                "default_url": "",
                "models": []
            }
        ]
    }))
}
