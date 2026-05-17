//! Onboarding backend support handlers — S76 Track C
//!
//! Endpoints:
//!   POST /v1/onboarding/setup   — full onboarding payload (model, provider keys,
//!                                  security level, feature toggles, skills, persona)
//!   GET  /v1/onboarding/config  — return current onboarding-relevant config fields

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Request / Response types
// ============================================================================

/// Full onboarding payload — matches the JSX onboarding wizard output exactly.
#[derive(Debug, Deserialize)]
pub struct OnboardingSetupRequest {
    /// Selected provider id, e.g. "anthropic"
    pub provider: Option<String>,
    /// Selected model id, e.g. "claude-sonnet-4-6"
    pub model: Option<String>,
    /// API keys keyed by provider id
    pub api_keys: Option<std::collections::HashMap<String, String>>,
    /// Security level: "minimal" | "standard" | "strict"
    pub security_level: Option<String>,
    /// Feature toggles keyed by feature name
    pub features: Option<std::collections::HashMap<String, bool>>,
    /// Selected skill ids
    pub skills: Option<Vec<String>>,
    /// Selected persona id/name
    pub persona: Option<String>,
    /// Agent name / identity
    pub name: Option<String>,
    /// Ollama URL (only relevant when provider == "ollama")
    pub ollama_url: Option<String>,
    /// Mark onboarding complete after saving
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Serialize)]
pub struct OnboardingSetupResponse {
    pub success: bool,
    pub saved_keys: Vec<String>,
    pub security_level: String,
    pub model: String,
    pub onboarding_complete: bool,
}

// ============================================================================
// Helpers
// ============================================================================

/// Map a provider id to the canonical env-var name used by zeus-llm.
fn provider_env_key(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic"  => Some("ANTHROPIC_API_KEY"),
        "openai"     => Some("OPENAI_API_KEY"),
        "google"     => Some("GOOGLE_API_KEY"),
        "groq"       => Some("GROQ_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "mistral"    => Some("MISTRAL_API_KEY"),
        "together"   => Some("TOGETHER_API_KEY"),
        "fireworks"  => Some("FIREWORKS_API_KEY"),
        "azure"      => Some("AZURE_OPENAI_API_KEY"),
        "bedrock"    => Some("AWS_ACCESS_KEY_ID"),
        _            => None,
    }
}

/// Map a security level string to an AegisConfig sandbox_level string.
fn security_level_to_sandbox(level: &str) -> &'static str {
    let lower = level.to_lowercase();
    match lower.as_str() {
        "minimal" => "none",
        "standard" => "basic",
        "strict" => "standard",
        "none" | "paranoid" => "none",
        _ => "basic",
    }
}

// ============================================================================
// POST /v1/onboarding/setup
// ============================================================================

/// POST /v1/onboarding/setup
///
/// Accepts the full onboarding wizard payload and persists:
/// - Provider API keys → `[credentials]` in config.toml + injected as env vars
/// - Model selection → `config.model`
/// - Security level  → `config.aegis.sandbox_level`
/// - Feature toggles → `config.tui.disabled_tools` / subsystem enables
/// - Skills          → `config.tui.disabled_tools` (complement of selected skills)
/// - Persona         → `config.tui.persona`
/// - Ollama URL      → `config.ollama.url`
/// - `onboarding_complete = true` when `complete == true`
pub async fn onboarding_setup(
    State(state): State<SharedState>,
    Json(req): Json<OnboardingSetupRequest>,
) -> Result<Json<OnboardingSetupResponse>, (StatusCode, String)> {
    let mut state = state.write().await;
    let mut saved_keys: Vec<String> = Vec::new();

    // ── 1. API keys → credentials + env vars ─────────────────────────────────
    if let Some(keys) = &req.api_keys {
        for (provider, key) in keys {
            if key.is_empty() {
                continue;
            }
            // Store in config [credentials] section
            let env_key = provider_env_key(provider)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_API_KEY", provider.to_uppercase()));

            state.config.credentials.insert(env_key.clone(), key.clone());

            // Inject into process environment so llm clients pick it up immediately
            unsafe { std::env::set_var(&env_key, key); }
            saved_keys.push(env_key);
        }
    }

    // ── 2. Model + provider ───────────────────────────────────────────────────
    if let Some(model) = &req.model {
        if !model.is_empty() {
            let provider = req.provider.as_deref().unwrap_or("");
            if !provider.is_empty() && !model.contains('/') {
                state.config.model = format!("{}/{}", provider, model);
            } else {
                state.config.model = model.clone();
            }
        }
    }

    // ── 3. Ollama URL ─────────────────────────────────────────────────────────
    if let Some(url) = &req.ollama_url {
        if !url.is_empty() {
            state.config.ollama.url = url.clone();
        }
    }

    // ── 4. Security level → aegis.sandbox_level ───────────────────────────────
    let security_level = req.security_level.as_deref().unwrap_or("standard");
    let sandbox = security_level_to_sandbox(security_level).to_string();
    {
        use zeus_core::AegisConfig;
        let aegis = state.config.aegis.get_or_insert_with(AegisConfig::default);
        aegis.sandbox_level = sandbox;
    }

    // ── 5. Feature toggles ────────────────────────────────────────────────────
    // We store disabled features in tui.disabled_tools so the agent loop can
    // read them without pulling in extra dependencies.
    if let Some(features) = &req.features {
        let disabled: Vec<String> = features
            .iter()
            .filter(|(_, enabled)| !**enabled)
            .map(|(name, _)| name.clone())
            .collect();
        // Merge: keep any existing disabled tools that aren't feature names,
        // then add newly-disabled ones.
        let feature_names: std::collections::HashSet<&String> = features.keys().collect();
        state.config.tui.disabled_tools.retain(|t| !feature_names.contains(t));
        state.config.tui.disabled_tools.extend(disabled);
    }

    // ── 6. Skills ─────────────────────────────────────────────────────────────
    // The TUI reads `disabled_tools`; skills not in the selected list are disabled.
    if let Some(skills) = &req.skills {
        // All known skills from JSX — tools not in `skills` get disabled
        const ALL_SKILLS: &[&str] = &[
            "web_search", "deep_research", "read_file", "write_file",
            "edit_file", "shell", "browser", "memory", "spawn",
            "code_review", "git", "devops", "summarize", "writing",
            "research", "tts", "image_gen", "video_gen",
        ];
        let selected: std::collections::HashSet<&String> = skills.iter().collect();
        let newly_disabled: Vec<String> = ALL_SKILLS
            .iter()
            .filter(|s| !selected.contains(&s.to_string()))
            .map(|s| s.to_string())
            .collect();
        // Remove all skill names then add back only disabled ones
        let skill_set: std::collections::HashSet<&str> =
            ALL_SKILLS.iter().copied().collect();
        state.config.tui.disabled_tools.retain(|t| !skill_set.contains(t.as_str()));
        state.config.tui.disabled_tools.extend(newly_disabled);
    }

    // ── 7. Persona ────────────────────────────────────────────────────────────
    if let Some(persona) = &req.persona {
        if !persona.is_empty() {
            state.config.persona = Some(persona.clone());
        }
    }

    // ── 8. Agent name ─────────────────────────────────────────────────────────
    if let Some(name) = &req.name {
        if !name.is_empty() {
            state.config.name = Some(name.clone());
        }
    }

    // ── 9. Complete flag ──────────────────────────────────────────────────────
    if req.complete {
        state.config.onboarding_complete = true;
    }

    let model = state.config.model.clone();
    let onboarding_complete = state.config.onboarding_complete;

    // ── 10. Persist ───────────────────────────────────────────────────────────
    state.config.save().map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save config: {e}"))
    })?;

    Ok(Json(OnboardingSetupResponse {
        success: true,
        saved_keys,
        security_level: security_level.to_string(),
        model,
        onboarding_complete,
    }))
}

// ============================================================================
// GET /v1/onboarding/config
// ============================================================================

/// GET /v1/onboarding/config
///
/// Returns current onboarding-relevant config fields (sanitized — no raw keys).
pub async fn onboarding_config(
    State(state): State<SharedState>,
) -> Json<Value> {
    let state = state.read().await;
    let cfg = &state.config;

    let provider = cfg.model.split('/').next().unwrap_or("").to_string();
    let model_id = cfg.model.splitn(2, '/').nth(1).unwrap_or(&cfg.model).to_string();

    let security_level = cfg.aegis.as_ref()
        .map(|a| match a.sandbox_level.as_str() {
            "none"   => "minimal",
            "basic"  => "standard",
            "standard" | "strict" | "paranoid" => "strict",
            other => other,
        })
        .unwrap_or("standard");

    // Which provider keys are present (names only, not values)
    let providers_with_keys: Vec<&str> = ["anthropic","openai","google","groq",
        "openrouter","mistral","together","fireworks","azure","bedrock"]
        .iter()
        .copied()
        .filter(|p| {
            provider_env_key(p)
                .map(|k| std::env::var(k).is_ok())
                .unwrap_or(false)
        })
        .collect();

    Json(json!({
        "onboarding_complete": cfg.onboarding_complete,
        "provider": provider,
        "model": model_id,
        "security_level": security_level,
        "disabled_tools": cfg.tui.disabled_tools,
        "persona": cfg.persona,
        "name": cfg.name,
        "providers_configured": providers_with_keys,
        "ollama_url": cfg.ollama.url,
    }))
}
