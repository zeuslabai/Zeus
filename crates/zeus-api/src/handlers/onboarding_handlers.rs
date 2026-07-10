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
    /// Whether to use OAuth for Anthropic auth (set per chosen auth method)
    pub use_oauth: Option<bool>,
    /// Ordered fallback model list (provider/model strings)
    pub fallback_models: Option<Vec<String>>,
    /// Gateway configuration overrides
    pub gateway: Option<OnboardingGatewayConfig>,
    /// Voice/TTS provider id (e.g. "elevenlabs", "piper", "openai_tts", "none")
    pub voice_provider: Option<String>,
    /// Image generation provider id (e.g. "openai", "comfyui", "fooocus", "none")
    pub image_gen_provider: Option<String>,
    /// Embedding provider id for Mnemosyne (e.g. "ollama", "openai", "none")
    pub embedding_provider: Option<String>,
    /// Workspace directory path override
    pub workspace_path: Option<String>,
    /// Mark onboarding complete after saving
    #[serde(default)]
    pub complete: bool,
}

/// Gateway config subset sent by the onboarding wizard.
#[derive(Debug, Deserialize)]
pub struct OnboardingGatewayConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub timeout_secs: Option<u64>,
    pub mentions_only: Option<bool>,
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
        // #220: canonical TUI provider set (registry ids + WebUI aliases)
        "moonshot" | "kimi" => Some("MOONSHOT_API_KEY"),
        "zai" | "glm"       => Some("ZAI_API_KEY"),
        "qwen"              => Some("QWEN_API_KEY"),
        "minimax"           => Some("MINIMAX_API_KEY"),
        "xiaomimimo" | "mimo" => Some("XIAOMIMIMO_API_KEY"),
        "deepseek"          => Some("DEEPSEEK_API_KEY"),
        "xai"               => Some("XAI_API_KEY"),
        "cerebras"          => Some("CEREBRAS_API_KEY"),
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

/// #220: Generate starter workspace files (SOUL.md, AGENTS.md, MEMORY.md,
/// HEARTBEAT.md) — shared by `onboarding_setup` (complete=true) and the
/// legacy `POST /v1/onboarding/complete` handler so both paths stay identical.
/// SOUL.md writes delegate to zeus_core::write_onboarding_soul so CLI and WebUI
/// share the same missing/stub/sludge-healing and custom-preserve policy (#326).
pub fn generate_workspace_files(config: &zeus_core::Config) {
    let workspace = &config.workspace;
    if workspace.as_os_str().is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(workspace.join("memory"));
    let _ = std::fs::create_dir_all(workspace.join("daily"));
    seed_workspace_skills(workspace);
    let agent_name = config
        .name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| config.model.split('/').next().unwrap_or("zeus-agent"));

    // #296/#326: write the SELECTED persona's soul, not generic boilerplate.
    // Onboarding is the only persona authority; it heals missing/stub/stock
    // souls but preserves custom operator-authored souls.
    let generic_soul = "A focused, technically sharp Zeus AI agent. Direct, resourceful, gets things done.".to_string();
    let persona_soul: Option<String> = config.persona.as_deref().filter(|p| !p.is_empty()).and_then(|sel| {
        let dir = zeus_core::default_config_dir().join("personalities");
        let reg = zeus_core::PersonaRegistry::load_from_dir(&dir).ok()?;
        reg.find(sel).map(|p| p.render_soul())
    });
    let soul_body = persona_soul.as_deref().unwrap_or(&generic_soul);
    let _ = zeus_core::write_onboarding_soul(&workspace.join("SOUL.md"), agent_name, soul_body, false);

    let files: &[(&str, String)] = &[
        ("AGENTS.md", format!("# AGENTS.md — {}\n\n## Every Session\n\nBefore doing anything else:\n1. Read `SOUL.md` — this is who you are\n2. Read `IDENTITY.md` — your fleet role\n3. Read `memory/` files for recent context\n\n## Quality First\n\n- **Review specs carefully** before writing code.\n- **Push code as you finish sections.**\n- **Careful work saves tons of time.**\n", agent_name)),
        ("MEMORY.md", format!("# MEMORY.md — {}\n\n_No memories stored yet. The agent will populate this over time._\n", agent_name)),
        ("HEARTBEAT.md", format!("# HEARTBEAT.md — {}\n\n_No proactive tasks configured. Add tasks here for the agent to execute periodically._\n", agent_name)),
    ];
    for (name, content) in files {
        let path = workspace.join(name);
        if !path.exists() {
            let _ = std::fs::write(&path, content);
        }
    }
}

/// #224: Seed `workspace/skills/` on fresh installs so the WebUI onboarding
/// skills step isn't empty. If the workspace skills dir is missing or empty,
/// mirror the global skill library (`~/.zeus/skills/`, honoring `ZEUS_HOME`).
/// No-op when the workspace already has skills or no global library exists.
pub fn seed_workspace_skills(workspace: &std::path::Path) {
    let ws_skills = workspace.join("skills");
    // Already populated? Nothing to do.
    if std::fs::read_dir(&ws_skills)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
    {
        return;
    }
    let Ok(home) = zeus_core::Config::zeus_home() else {
        return;
    };
    let global_skills = home.join("skills");
    if !global_skills.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(&ws_skills);
    let Ok(entries) = std::fs::read_dir(&global_skills) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let src = entry.path();
        // Resolve symlinks (community skills are symlinked into the library)
        let real = std::fs::canonicalize(&src).unwrap_or(src);
        if real.is_dir() {
            let _ = copy_dir_recursive(&real, &ws_skills.join(&name));
        }
    }
}

/// Minimal recursive directory copy (no external deps). Skips entries that
/// fail to copy rather than aborting the whole seed.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let ty = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            let _ = copy_dir_recursive(&entry.path(), &to);
        } else {
            let _ = std::fs::copy(entry.path(), &to);
        }
    }
    Ok(())
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
    // #312 batch①: mirror TUI's OAT-prefix detection (app.rs ~1358).
    // OAuth tokens (sk-ant-oat* or use_oauth=true) route to
    // `[provider_credentials.{provider}]` via set_oauth — the path the
    // gateway's auth read-path consults for OAuth. Plain API keys go to
    // `[credentials]` keyed by env_key.
    if let Some(keys) = &req.api_keys {
        for (provider, key) in keys {
            if key.is_empty() {
                continue;
            }
            let env_key = provider_env_key(provider)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_API_KEY", provider.to_uppercase()));

            // Detect OAuth tokens: explicit use_oauth flag OR OAT-prefix heuristic
            let is_oauth = req.use_oauth.unwrap_or(false)
                || key.starts_with("sk-ant-oat");

            if is_oauth {
                // Route to [provider_credentials.{provider}] — the gateway's
                // OAuth auth read-path (zeus-llm branch-4).
                let p = zeus_core::Provider::from_prefix(provider);
                if !state.config.provider_credentials.set_oauth(p, key) {
                    // Unknown provider for OAuth — fall back to credentials HashMap
                    state.config.credentials.insert(env_key.clone(), key.clone());
                }
            } else {
                // Plain API key → [credentials] HashMap (the gateway bridge
                // exports these to process env at startup).
                state.config.credentials.insert(env_key.clone(), key.clone());
            }

            // #220: also store in CredentialVault (keychain/file) so the key
            // survives outside config.toml — mirrors POST /v1/credentials (#219).
            if let Err(e) = state.credential_vault.store(&env_key, key).await {
                tracing::warn!("onboarding_setup: vault store for {} failed: {}", env_key, e);
            }

            // Inject into process environment so llm clients pick it up immediately
            unsafe { std::env::set_var(&env_key, key); }
            saved_keys.push(env_key);
        }
    }

    // ── 1b. Persist use_oauth per the auth method actually chosen ─────────────
    // #313 follow-up: the WebUI sends use_oauth=true when the user authenticated
    // via OAuth (auth_types["anthropic"] == "oauth_token"), false for API key.
    // Without this, config.auth.use_oauth stays at its default (false) even when
    // the user chose OAuth, causing the S97 guard to inject the wrong credential.
    if let Some(use_oauth) = req.use_oauth {
        state.config.auth.use_oauth = use_oauth;
    }

    // ── 1c. Fallback models ───────────────────────────────────────────────────
    // #312 batch①: WebUI sends fallback_models as a JSON array of "provider/model"
    // strings. Persist to config.fallback_models (mirrors TUI app.rs ~1076).
    if let Some(fallbacks) = &req.fallback_models {
        if fallbacks.is_empty() {
            state.config.fallback_models = None;
        } else {
            state.config.fallback_models = Some(fallbacks.clone());
        }
    }

    // ── 1d. Gateway config ────────────────────────────────────────────────────
    // #312 batch①: WebUI sends gateway host/port/timeout/mentions_only.
    // Persist to config.gateway (mirrors TUI gateway_screen fields).
    if let Some(gw) = &req.gateway {
        let gateway = state.config.gateway.get_or_insert_with(Default::default);
        if let Some(host) = &gw.host {
            if !host.is_empty() {
                gateway.host = host.clone();
            }
        }
        if let Some(port) = gw.port {
            if port > 0 {
                gateway.port = port;
            }
        }
        if let Some(timeout) = gw.timeout_secs {
            if timeout > 0 {
                gateway.timeout_secs = timeout;
            }
        }
        if let Some(mentions) = gw.mentions_only {
            gateway.mentions_only = mentions;
        }
    }

    // ── 1e. Voice / TTS provider ──────────────────────────────────────────────
    // #312 batch②: WebUI sends voice_provider (e.g. "elevenlabs", "piper",
    // "openai_tts", "none"). Persist to config.voice — mirrors TUI
    // app.rs:1122-1133 voice screen handling.
    if let Some(voice_id) = &req.voice_provider {
        if !voice_id.is_empty() {
            let voice = state.config.voice.get_or_insert_with(Default::default);
            if voice_id == "none" {
                voice.enabled = false;
            } else {
                voice.provider = Some(voice_id.clone());
                voice.enabled = true;
            }
        }
    }

    // ── 1f. Image generation provider ─────────────────────────────────────────
    // #312 batch②: WebUI sends image_gen_provider (e.g. "openai", "comfyui",
    // "fooocus", "a1111", "openai-custom", "none"). Persist to config.image_gen
    // — mirrors TUI app.rs:1135-1151 image screen handling.
    if let Some(img_id) = &req.image_gen_provider {
        if !img_id.is_empty() && img_id != "none" {
            use zeus_core::ImageGenProviderType;
            let provider = match img_id.as_str() {
                "openai" => Some(ImageGenProviderType::OpenAi),
                "comfyui" => Some(ImageGenProviderType::ComfyUi),
                "fooocus" => Some(ImageGenProviderType::Fooocus),
                "a1111" => Some(ImageGenProviderType::Automatic1111),
                "openai-custom" => Some(ImageGenProviderType::OpenAiCompatible),
                _ => None,
            };
            if let Some(p) = provider {
                state.config.image_gen.get_or_insert_with(Default::default).provider = p;
            }
        }
    }

    // ── 1g. Embedding provider (Mnemosyne) ────────────────────────────────────
    // #312 batch②: WebUI sends embedding_provider (e.g. "ollama", "openai",
    // "none"). Persist to config.mnemosyne — mirrors TUI app.rs:1155-1168
    // memory screen handling.
    if let Some(emb_id) = &req.embedding_provider {
        if !emb_id.is_empty() && emb_id != "none" {
            use zeus_core::EmbeddingProvider;
            let provider = match emb_id.as_str() {
                "ollama" => Some(EmbeddingProvider::Ollama),
                "openai" => Some(EmbeddingProvider::OpenAI),
                "gemini" => Some(EmbeddingProvider::Gemini),
                "voyage" => Some(EmbeddingProvider::Voyage),
                _ => None,
            };
            if let Some(p) = provider {
                let mn = state.config.mnemosyne.get_or_insert_with(Default::default);
                mn.enable_embeddings = true;
                mn.embedding_providers = vec![p];
            }
        }
    }

    // ── 1h. Workspace path ────────────────────────────────────────────────────
    // #312 batch②: WebUI sends workspace_path. Persist to config.workspace —
    // mirrors TUI app.rs ~workspace screen handling.
    if let Some(ws) = &req.workspace_path {
        if !ws.is_empty() {
            let expanded = if ws.starts_with('~') {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(ws.trim_start_matches("~/"))
            } else {
                std::path::PathBuf::from(ws)
            };
            state.config.workspace = expanded;
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
        // #220: parity with POST /v1/onboarding/complete — make the saved
        // config authoritative and generate starter workspace files, so the
        // wizard needs only this one endpoint to finish onboarding.
        state.config.loaded_from_default = false;
        generate_workspace_files(&state.config);
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

// ============================================================================
// GET /v1/onboarding/skills
// ============================================================================

/// List the skill catalog for the onboarding wizard's Skills step.
///
/// Returns a flat array of `{id, name, description, default}` — exactly the
/// shape the WebUI parser (onboarding_wizard.rs StepSkills) expects. Sourced
/// live from the workspace `skills/` directory (same loader as `/v1/skills`),
/// so the wizard shows the real installed catalog instead of a hardcoded stub.
///
/// `default: true` marks skills in the default-active set: the configured
/// persona's `default_skills` (the force-active wiring from #gap2/f0ab52d4)
/// unioned with the built-in `WorkspaceTemplate` defaults.
pub async fn onboarding_skills(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");
    let details = super::skills::load_all_skill_details(&skills_dir).await;

    // Default-on set: built-in template defaults ∪ configured persona defaults.
    let mut defaults: std::collections::HashSet<String> =
        zeus_core::WorkspaceTemplate::builtins()
            .into_iter()
            .flat_map(|t| t.default_skills)
            .collect();
    if let Some(persona_name) = state.config.persona.as_deref() {
        let personalities_dir = zeus_core::default_config_dir().join("personalities");
        if let Ok(registry) = zeus_core::PersonaRegistry::load_from_dir(&personalities_dir) {
            if let Some(persona) = registry.by_name(persona_name) {
                defaults.extend(persona.default_skills.iter().cloned());
            }
        }
    }

    let list: Vec<Value> = details
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "name": if s.name.is_empty() { s.id.clone() } else { s.name.clone() },
                "description": s.description,
                "default": defaults.contains(&s.id),
            })
        })
        .collect();

    Json(Value::Array(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // #296: a blank or install-stub SOUL.md is overwritable; a real one is not.
    #[test]
    fn soul_stub_detection() {
        let dir = std::env::temp_dir().join(format!("zeus-soul-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("SOUL.md");

        // Missing → overwritable.
        let _ = std::fs::remove_file(&p);
        assert!(zeus_core::soul_is_stub_or_missing(&p));

        // Install stub → overwritable.
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "# SOUL.md — Run 'zeus onboard' to configure").unwrap();
        drop(f);
        assert!(zeus_core::soul_is_stub_or_missing(&p));

        // Blank → overwritable.
        std::fs::write(&p, "   \n").unwrap();
        assert!(zeus_core::soul_is_stub_or_missing(&p));

        // Real persona soul → preserved.
        std::fs::write(&p, "# SOUL.md — The Coordinator\n\nYou are the coordinator...\n").unwrap();
        assert!(!zeus_core::soul_is_stub_or_missing(&p));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
