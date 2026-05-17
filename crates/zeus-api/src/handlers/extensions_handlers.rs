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
    let _ = state.config.save();

    // S98: Do NOT process::exit() here — it kills the server before the HTTP response
    // reaches the WebUI client, causing an onboarding redirect loop.
    // The WebUI sets localStorage("zeus_onboarding_complete") on success and navigates
    // to "/". Config is already saved with onboarding_complete=true above.
    // If the user needs a full restart (for credential reload), they can run
    // `zeus daemon restart` manually, or the TUI onboarding handles it.

    let workspace = &state.config.workspace;
    if !workspace.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(workspace.join("memory"));
        let _ = std::fs::create_dir_all(workspace.join("daily"));
        let agent_name = state.config.model.split('/').next().unwrap_or("zeus-agent");
        let files: &[(&str, &str)] = &[
            ("SOUL.md", &format!("# SOUL.md — {}\n\n_A focused, technically sharp Zeus AI agent. Direct, resourceful, gets things done._\n\n## Core Truths\n\n**Be genuinely helpful, not performatively helpful.** Skip filler — just help.\n\n**Have opinions.** You're allowed to disagree, prefer things, find stuff interesting.\n\n**Be resourceful before asking.** Try to figure it out. Read the file. Check the context.\n", agent_name)),
            ("AGENTS.md", &format!("# AGENTS.md — {}\n\n## Every Session\n\nBefore doing anything else:\n1. Read `SOUL.md` — this is who you are\n2. Read `IDENTITY.md` — your fleet role\n3. Read `memory/` files for recent context\n\n## Quality First\n\n- **Review specs carefully** before writing code.\n- **Push code as you finish sections.**\n- **Careful work saves tons of time.**\n", agent_name)),
            ("MEMORY.md", &format!("# MEMORY.md — {}\n\n_No memories stored yet. The agent will populate this over time._\n", agent_name)),
            ("HEARTBEAT.md", &format!("# HEARTBEAT.md — {}\n\n_No proactive tasks configured. Add tasks here for the agent to execute periodically._\n", agent_name)),
        ];
        for (name, content) in files {
            let path = workspace.join(name);
            if !path.exists() {
                let _ = std::fs::write(&path, content);
            }
        }
    }

    Json(json!({ "success": true }))
}

/// GET /v1/providers — Provider catalog for onboarding and provider selection UI
pub async fn list_providers() -> Json<Value> {
    Json(json!({
        "providers": [
            {
                "id": "anthropic",
                "name": "Anthropic",
                "tagline": "Claude models — powerful, safe, steerable",
                "icon": "🧠",
                "color": "#D97706",
                "auth_methods": ["subscription", "api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "claude-sonnet-4-6", "name": "Claude Sonnet 4.6", "tier": "recommended" },
                    { "id": "claude-opus-4-6", "name": "Claude Opus 4.6", "tier": "premium" },
                    { "id": "claude-haiku-4-5-20251001", "name": "Claude Haiku 4.5", "tier": "fast" }
                ]
            },
            {
                "id": "openai",
                "name": "OpenAI",
                "tagline": "GPT-5.2, o3-pro, and the latest reasoning models",
                "icon": "⚡",
                "color": "#A855F7",
                "auth_methods": ["subscription", "api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "gpt-5.2", "name": "GPT-5.2", "tier": "recommended" },
                    { "id": "gpt-5.3-codex", "name": "GPT-5.3 Codex", "tier": "code" },
                    { "id": "o3-pro", "name": "o3 Pro", "tier": "reasoning" },
                    { "id": "o4-mini", "name": "o4 Mini", "tier": "fast" },
                    { "id": "gpt-4.1", "name": "GPT-4.1", "tier": "fast" }
                ]
            },
            {
                "id": "ollama",
                "name": "Ollama",
                "tagline": "Run models locally — free, no API key",
                "icon": "🦙",
                "color": "#10B981",
                "auth_methods": ["none"],
                "requires_url": true,
                "default_url": "http://localhost:11434",
                "models": [
                    { "id": "deepseek-r1", "name": "DeepSeek R1", "tier": "reasoning" },
                    { "id": "qwen3", "name": "Qwen 3", "tier": "recommended" },
                    { "id": "llama4", "name": "Llama 4", "tier": "recommended" },
                    { "id": "gemma3", "name": "Gemma 3", "tier": "fast" }
                ]
            },
            {
                "id": "openrouter",
                "name": "OpenRouter",
                "tagline": "Access 200+ models through one API",
                "icon": "🔀",
                "color": "#6366F1",
                "auth_methods": ["api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "anthropic/claude-sonnet-4-6", "name": "Claude Sonnet 4.6", "tier": "recommended" },
                    { "id": "openai/gpt-5.2", "name": "GPT-5.2", "tier": "recommended" },
                    { "id": "meta-llama/llama-4-maverick", "name": "Llama 4 Maverick", "tier": "fast" }
                ]
            },
            {
                "id": "google",
                "name": "Google",
                "tagline": "Gemini models — multimodal AI",
                "icon": "💎",
                "color": "#3B82F6",
                "auth_methods": ["api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tier": "recommended" },
                    { "id": "gemini-2.5-pro", "name": "Gemini 2.5 Pro", "tier": "premium" }
                ]
            },
            {
                "id": "groq",
                "name": "Groq",
                "tagline": "Ultra-fast inference on LPU hardware",
                "icon": "🚀",
                "color": "#22D3EE",
                "auth_methods": ["api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "llama-4-scout-17b-16e-instruct", "name": "Llama 4 Scout", "tier": "recommended" },
                    { "id": "deepseek-r1-distill-llama-70b", "name": "DeepSeek R1 70B", "tier": "reasoning" }
                ]
            },
            {
                "id": "mistral",
                "name": "Mistral AI",
                "tagline": "Open-weight European AI models",
                "icon": "🌊",
                "color": "#F97316",
                "auth_methods": ["api_key"],
                "requires_url": false,
                "default_url": "",
                "models": [
                    { "id": "mistral-large-latest", "name": "Mistral Large 3", "tier": "premium" },
                    { "id": "codestral-latest", "name": "Codestral", "tier": "code" },
                    { "id": "mistral-small-latest", "name": "Mistral Small 3", "tier": "fast" }
                ]
            }
        ]
    }))
}
