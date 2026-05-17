//! Zeus LLM - Unified LLM provider
//!
//! Supports Anthropic, OpenAI, Ollama, and OpenRouter with OAuth and API key auth.

pub mod capabilities;
pub mod codex;
pub mod cost;
pub mod fallback;
pub mod json_healing;
pub mod middle_out;
pub mod model_variants;
pub mod pipeline;
pub mod provider_health;
pub use fallback::FallbackProvider;
pub mod minimax;
pub mod multimodal;
pub mod qwen_oauth;
pub mod oauth;
pub mod ollama;
pub mod response_cache;
pub mod router;
pub use response_cache::{
    CacheConfig, CacheEntry, CacheLookup, CacheStats, ResponseCache, cache_key,
};

use base64::Engine;
use futures::StreamExt;
use reqwest::Client;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use zeus_core::{Config, Error, Message, Provider, Result, Role, ToolCall, ToolSchema};

/// Build an HTTP client with a generous timeout for LLM inference.
///
/// Ollama requests behind reverse proxies (Cloudflare/nginx) can hit
/// proxy timeouts (HTTP 524) if inference takes >100s. We set a 5-minute
/// timeout to outlast typical proxy limits while still preventing hangs.
///
/// `redirect::Policy::none()` is set so that redirect responses (301/302)
/// are surfaced to calling code instead of being followed automatically.
/// `reqwest` downgrades POST→GET when auto-following 301/302 per the HTTP
/// spec, which causes Ollama servers behind HTTPS proxies to return 405.
/// The `post_following_redirect` helper re-issues POST correctly.
fn build_llm_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// POST `body` to `url`, re-issuing the POST to the `Location` header on
/// 301/302/307/308 redirects.
///
/// Because `build_llm_client` disables automatic redirect following, a
/// redirect response reaches this function intact. We re-issue the same POST
/// to the redirect target, which handles Ollama servers served behind an
/// HTTP→HTTPS reverse proxy (e.g. Cloudflare, nginx).
async fn post_following_redirect(
    client: &Client,
    url: &str,
    body: &serde_json::Value,
) -> std::result::Result<reqwest::Response, reqwest::Error> {
    let resp = client
        .post(url)
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await?;

    if matches!(resp.status().as_u16(), 301 | 302 | 307 | 308)
        && let Some(location) = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    {
        warn!(
            "Ollama redirected {} → {}; re-issuing POST to preserve method",
            url, location
        );
        return client
            .post(&location)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await;
    }

    Ok(resp)
}

/// Check if an HTTP status code is retryable (transient server error).
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 524)
}

/// Random jitter (0–500ms) to prevent thundering herd on retry.
fn rand_jitter_ms() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    h.finish() % 500
}

pub use oauth::{
    CredentialKind, CredentialStore, OAuthManager, OAuthTokens, PkceChallenge, StoredCredential,
    exchange_authorization_code, validate_setup_token,
};
pub use ollama::{OllamaClient, OllamaModel, normalize_ollama_url};
pub use router::{
    Complexity, ModelRouter, ModelRoutingConfig, RouteSelection, TaskType, parse_model_string,
};

/// Sprint 2 P2-Qwen (2026-04-10): resolve the Qwen/Alibaba DashScope
/// base URL based on environment config.
///
/// Qwen exposes four OpenAI-compatible endpoints that vary by plan
/// (Standard pay-as-you-go vs Coding Plan subscription) and region
/// (CN vs Global). OpenClaw auto-detects from the API key shape;
/// Zeus prefers explicit env vars so `config.toml` stays the source
/// of truth and users don't get surprised by silent region switches.
///
/// Resolution order:
/// 1. `QWEN_BASE_URL` — explicit full-URL override (matches the
///    `AZURE_OPENAI_ENDPOINT` pattern we already use for Azure).
/// 2. `QWEN_REGION` ∈ {`cn`, `global`} × `QWEN_PLAN` ∈ {`standard`,
///    `coding`} — structured override.
/// 3. Default: Standard + Global (`dashscope-intl.aliyuncs.com`).
///
/// | plan        | region | base URL                                              |
/// |-------------|--------|-------------------------------------------------------|
/// | Standard    | Global | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` |
/// | Standard    | CN     | `https://dashscope.aliyuncs.com/compatible-mode/v1`      |
/// | Coding Plan | Global | `https://coding-intl.dashscope.aliyuncs.com/v1`          |
/// | Coding Plan | CN     | `https://coding.dashscope.aliyuncs.com/v1`               |
///
/// Note the path suffix difference: Standard endpoints use
/// `/compatible-mode/v1` (DashScope's OpenAI-compat shim), Coding Plan
/// endpoints use `/v1` directly (already OpenAI-shaped).
pub fn resolve_qwen_base_url() -> String {
    // 1. Explicit full-URL override.
    if let Ok(url) = std::env::var("QWEN_BASE_URL") {
        if !url.trim().is_empty() {
            return url.trim_end_matches('/').to_string();
        }
    }
    // 2. Structured region + plan.
    let region = std::env::var("QWEN_REGION")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let plan = std::env::var("QWEN_PLAN")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    match (plan.as_str(), region.as_str()) {
        ("oauth", _) => "https://portal.qwen.ai/v1".to_string(), // Qwen OAuth subscription
        ("coding", "cn") => "https://coding.dashscope.aliyuncs.com/v1".to_string(),
        ("coding", _) => "https://coding-intl.dashscope.aliyuncs.com/v1".to_string(),
        (_, "cn") => "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        _ => "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string(),
    }
}

/// Sprint 2 P2-Qwen: resolve the Qwen API key with the three-tier
/// compatibility chain that OpenClaw's bundled provider uses.
///
/// `QWEN_API_KEY` is the canonical env var. `DASHSCOPE_API_KEY` and
/// `MODELSTUDIO_API_KEY` are accepted as compatibility aliases so
/// existing users who already exported one of those (from an earlier
/// DashScope/ModelStudio integration) don't need to rename anything.
///
/// First non-empty value wins. Returns `None` when all three are
/// unset or empty.
pub fn resolve_qwen_api_key() -> Option<String> {
    for var in ["QWEN_API_KEY", "DASHSCOPE_API_KEY", "MODELSTUDIO_API_KEY"] {
        if let Ok(v) = std::env::var(var) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Endpoint scope for a Qwen model entry in the bundled catalog.
///
/// Sprint 2 P2-Qwen: the Qwen Coding Plan endpoint catalog is a
/// strict subset of the Standard endpoint catalog. `qwen3.6-plus`,
/// multimodal video understanding, and video generation models are
/// only served on Standard; offering them to Coding Plan users
/// produces confusing "unsupported model" errors. Filter at catalog
/// build time instead of at request time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwenEndpointScope {
    /// Works on any Qwen endpoint (Standard or Coding Plan).
    Any,
    /// Only served on the Standard (pay-as-you-go) DashScope endpoint.
    StandardOnly,
    /// Only served on the Coding Plan subscription endpoint.
    CodingOnly,
}

/// One entry in the bundled Qwen model catalog. See
/// `qwen_bundled_catalog()` for the list.
#[derive(Debug, Clone)]
pub struct QwenBundledModel {
    pub id: &'static str,
    pub context_window: usize,
    pub supports_vision: bool,
    pub reasoning: bool,
    pub scope: QwenEndpointScope,
    pub note: &'static str,
}

/// Sprint 2 P2-Qwen: the bundled Qwen model catalog, mirrored from
/// OpenClaw's `qwen` provider documentation (2026-04-10). Keep this
/// list in sync with `docs/kimi-glm-qwen-research.md`.
///
/// `qwen_filtered_catalog(base_url)` returns the subset that's valid
/// for the given endpoint — call that instead of using this directly
/// so Coding Plan users don't see Standard-only models like
/// `qwen3.6-plus` or the video-gen `wan2.*` family.
pub fn qwen_bundled_catalog() -> Vec<QwenBundledModel> {
    vec![
        QwenBundledModel {
            id: "qwen3.5-plus",
            context_window: 1_000_000,
            supports_vision: true,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "default — text + image, 1M context",
        },
        QwenBundledModel {
            id: "qwen3.6-plus",
            context_window: 1_000_000,
            supports_vision: true,
            reasoning: false,
            scope: QwenEndpointScope::StandardOnly,
            note: "prefer Standard endpoints",
        },
        QwenBundledModel {
            id: "qwen3-max-2026-01-23",
            context_window: 262_144,
            supports_vision: false,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "Qwen Max line",
        },
        QwenBundledModel {
            id: "qwen3-coder-next",
            context_window: 262_144,
            supports_vision: false,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "coding",
        },
        QwenBundledModel {
            id: "qwen3-coder-plus",
            context_window: 1_000_000,
            supports_vision: false,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "coding, 1M ctx",
        },
        QwenBundledModel {
            id: "MiniMax-M2.5",
            context_window: 1_000_000,
            supports_vision: false,
            reasoning: true,
            scope: QwenEndpointScope::Any,
            note: "MiniMax reasoning via Alibaba",
        },
        QwenBundledModel {
            id: "glm-5",
            context_window: 202_752,
            supports_vision: false,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "GLM via Alibaba",
        },
        QwenBundledModel {
            id: "glm-4.7",
            context_window: 202_752,
            supports_vision: false,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "GLM via Alibaba",
        },
        QwenBundledModel {
            id: "kimi-k2.5",
            context_window: 262_144,
            supports_vision: true,
            reasoning: false,
            scope: QwenEndpointScope::Any,
            note: "Moonshot via Alibaba",
        },
    ]
}

/// Filter the bundled Qwen catalog against the currently-configured
/// endpoint. Coding Plan hosts (`coding*.dashscope.aliyuncs.com`)
/// drop `StandardOnly` entries; Standard hosts
/// (`*dashscope*.aliyuncs.com/compatible-mode/v1`) drop `CodingOnly`
/// entries. Unknown hosts (user override via `QWEN_BASE_URL`) keep
/// the full catalog since we can't know what the custom proxy serves.
pub fn qwen_filtered_catalog(base_url: &str) -> Vec<QwenBundledModel> {
    let is_coding = base_url.contains("coding.dashscope") || base_url.contains("coding-intl.dashscope");
    let is_standard =
        base_url.contains("compatible-mode/v1") && base_url.contains("dashscope");
    qwen_bundled_catalog()
        .into_iter()
        .filter(|m| match m.scope {
            QwenEndpointScope::Any => true,
            QwenEndpointScope::StandardOnly => !is_coding,
            QwenEndpointScope::CodingOnly => is_coding || !is_standard,
        })
        .collect()
}

// ============================================================================
// Streaming types (~30 lines)
// ============================================================================

pub type StreamChunk = std::result::Result<String, Error>;
pub type ResponseStream = Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>>;

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    /// Token usage (0 if provider doesn't report)
    pub input_tokens: usize,
    pub output_tokens: usize,
    /// Cached input tokens (Anthropic prompt caching)
    pub cached_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Error,
}

/// Embedding response from an embedding API call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingResponse {
    pub model: String,
    pub data: Vec<Embedding>,
    pub usage: EmbeddingUsage,
}

/// A single embedding vector.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Embedding {
    pub index: usize,
    pub embedding: Vec<f64>,
}

/// Token usage for embedding requests.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

/// Response format for structured outputs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFormat {
    /// Regular text response (default)
    Text,
    /// Force JSON object output
    JsonObject,
    /// JSON output conforming to a specific schema
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        strict: bool,
    },
}

// ============================================================================
// LLM Client (~350 lines)
// ============================================================================

/// Authentication method for API calls
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// API key authentication
    ApiKey(String),
    /// OAuth bearer token
    OAuth(String),
    /// No authentication (for local services like Ollama)
    None,
}

/// Credential status with helpful suggestions
#[derive(Debug, Clone)]
pub enum CredentialStatus {
    /// Credentials are valid
    Valid(String),
    /// Credentials are missing with suggestions to fix
    Missing {
        provider: String,
        suggestions: Vec<String>,
    },
}

#[derive(Clone)]
pub struct LlmClient {
    provider: Provider,
    model: String,
    client: Client,
    auth: AuthMethod,
    base_url: String,
    /// Thinking level for extended thinking (Anthropic only): low/medium/high/xhigh
    pub thinking_level: Option<String>,
    /// Response format for structured outputs (OpenAI-compatible providers)
    response_format: Option<ResponseFormat>,
    /// Seed for deterministic outputs (OpenAI-compatible providers)
    seed: Option<u64>,
    /// Fallback model string ("provider/model") used after primary exhausts all retries.
    /// Parsed into (Provider, model_name) on first use.
    fallback_model: Option<String>,
    /// Persistent retry: keep retrying with 5-min max backoff instead of giving up.
    /// For long-running background tasks that must eventually succeed.
    persistent_retry: bool,
    /// Ollama-specific options (temperature, num_predict, keep_alive, etc.)
    ollama_temperature: f64,
    ollama_num_predict: u32,
    ollama_num_predict_tools: u32,
    ollama_keep_alive: String,
    ollama_top_p: Option<f64>,
    ollama_top_k: Option<u32>,
    ollama_repeat_penalty: Option<f64>,
}

/// Returns true if `model` is in the OpenAI "reasoning" family (o1/o3/o4/gpt-5.5).
///
/// These models route via `max_completion_tokens` (not `max_tokens`) and accept
/// `reasoning_effort` instead of `temperature`. Single source of truth for the
/// predicate previously duplicated across `inject_openai_sampling`,
/// `complete_openai`, and `stream_openai`.
pub(crate) fn is_reasoning_model(model: &str) -> bool {
    model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("gpt-5.5")
}

/// Returns true if `model` is GPT-5.5 invoked with tools — a combination that
/// rejects BOTH `temperature` AND `reasoning_effort` server-side.
///
/// GPT-5.5 without tools still accepts `reasoning_effort`. This carve-out is
/// specific to the `gpt-5.5 + tools` intersection.
pub(crate) fn gpt55_rejects_sampling(model: &str, has_tools: bool) -> bool {
    model.starts_with("gpt-5.5") && has_tools
}

impl LlmClient {
    fn import_gemini_cli_oauth() -> Result<Option<StoredCredential>> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Config("Could not find home directory".to_string()))?;
        let roots = [home.join(".gemini"), home.join(".config").join("gemini")];

        for root in roots {
            if !root.exists() {
                continue;
            }
            if let Some(cred) = Self::scan_gemini_dir(&root)? {
                let mut store = CredentialStore::load()?;
                store.store(cred.clone())?;
                info!("Imported Gemini CLI OAuth credentials from {}", root.display());
                return Ok(Some(cred));
            }
        }

        Ok(None)
    }

    fn scan_gemini_dir(root: &Path) -> Result<Option<StoredCredential>> {
        let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if let Some(cred) = Self::parse_gemini_credential_file(&path)? {
                    return Ok(Some(cred));
                }
            }
        }
        Ok(None)
    }

    fn parse_gemini_credential_file(path: &Path) -> Result<Option<StoredCredential>> {
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => return Ok(None),
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(json) => json,
            Err(_) => return Ok(None),
        };

        let access_token = Self::find_string_field(&json, &["access_token", "accessToken", "token"]);
        let refresh_token = Self::find_string_field(&json, &["refresh_token", "refreshToken"]);

        let Some(refresh_token) = refresh_token else {
            return Ok(None);
        };

        let expires_at = Self::find_expiry(&json)
            .unwrap_or_else(|| Utc::now() - ChronoDuration::minutes(1));

        Ok(Some(StoredCredential {
            provider: "google".to_string(),
            kind: CredentialKind::OAuthToken,
            token: access_token.unwrap_or_default(),
            refresh_token,
            expires_at,
            stored_at: Utc::now(),
        }))
    }

    fn find_string_field(json: &serde_json::Value, keys: &[&str]) -> Option<String> {
        match json {
            serde_json::Value::Object(map) => {
                for key in keys {
                    if let Some(value) = map.get(*key).and_then(|v| v.as_str())
                        && !value.is_empty()
                    {
                        return Some(value.to_string());
                    }
                }
                for value in map.values() {
                    if let Some(found) = Self::find_string_field(value, keys) {
                        return Some(found);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => items
                .iter()
                .find_map(|value| Self::find_string_field(value, keys)),
            _ => None,
        }
    }

    fn find_i64_field(json: &serde_json::Value, keys: &[&str]) -> Option<i64> {
        match json {
            serde_json::Value::Object(map) => {
                for key in keys {
                    if let Some(value) = map.get(*key) {
                        if let Some(v) = value.as_i64() {
                            return Some(v);
                        }
                        if let Some(v) = value.as_u64() {
                            return Some(v as i64);
                        }
                        if let Some(v) = value.as_str().and_then(|s| s.parse::<i64>().ok()) {
                            return Some(v);
                        }
                    }
                }
                for value in map.values() {
                    if let Some(found) = Self::find_i64_field(value, keys) {
                        return Some(found);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => {
                items.iter().find_map(|value| Self::find_i64_field(value, keys))
            }
            _ => None,
        }
    }

    fn find_expiry(json: &serde_json::Value) -> Option<DateTime<Utc>> {
        if let Some(ts) = Self::find_i64_field(json, &["expires_at", "expiry_date", "expiry", "expiresAt"]) {
            let secs = if ts > 10_000_000_000 { ts / 1000 } else { ts };
            return DateTime::<Utc>::from_timestamp(secs, 0);
        }
        if let Some(expires_in) = Self::find_i64_field(json, &["expires_in"]) {
            return Some(Utc::now() + ChronoDuration::seconds(expires_in));
        }
        None
    }

    /// Get a valid OpenAI Codex OAuth token, auto-refreshing if expired.
    async fn codex_bearer_token(&self) -> Result<String> {
        match &self.auth {
            AuthMethod::OAuth(token) => {
                // Check if token is a JWT and parse expiry
                if let Some(identity) = codex::parse_codex_jwt_identity(token) {
                    if let Some(exp) = identity.expires_at {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        // Refresh 5 minutes before expiry
                        if now + 300 >= exp {
                            // Try to get refresh token from config oauth section or credential store
                            if let Ok(Some(cred)) = OAuthManager::get_credential("openai") {
                                if !cred.refresh_token.is_empty() {
                                    match codex::refresh_codex_token_lenient(
                                        &self.client, &cred.refresh_token, Some(token),
                                    ).await {
                                        Ok((new_token, _)) => {
                                            info!("Codex OAuth token refreshed");
                                            return Ok(new_token);
                                        }
                                        Err(e) => warn!("Codex token refresh failed: {}, using existing", e),
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(token.clone())
            }
            _ => Err(Error::Llm("OpenAI OAuth not configured".to_string())),
        }
    }

    async fn google_bearer_token(&self) -> Result<String> {
        match &self.auth {
            AuthMethod::OAuth(token) => {
                if let Ok(Some(cred)) = OAuthManager::get_credential("google")
                    && Utc::now() + ChronoDuration::minutes(5) >= cred.expires_at
                    && !cred.refresh_token.is_empty()
                    && let Some(tokens) = OAuthManager::refresh_token("google").await?
                {
                    return Ok(tokens.access_token);
                }
                Ok(token.clone())
            }
            _ => Err(Error::Llm("Google OAuth not configured".to_string())),
        }
    }

    fn google_generate_url(&self, api_key: Option<&str>) -> String {
        let model = Self::resolve_google_model_alias(&self.model);
        match api_key {
            Some(key) => format!(
                "{}/v1beta/models/{}:generateContent?key={}",
                self.base_url, model, key
            ),
            None => format!("{}/v1beta/models/{}:generateContent", self.base_url, model),
        }
    }

    fn google_stream_url(&self, api_key: Option<&str>) -> String {
        let model = Self::resolve_google_model_alias(&self.model);
        match api_key {
            Some(key) => format!(
                "{}/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
                self.base_url, model, key
            ),
            None => format!(
                "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
                self.base_url, model
            ),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    pub fn new(provider: Provider, model: String) -> Result<Self> {
        if model.is_empty() {
            return Err(Error::Llm(
                "No model configured. Please set a model in ~/.zeus/config.toml \
                 (e.g. model = \"anthropic/claude-sonnet-4-20250514\") or configure a provider in Settings."
                    .to_string(),
            ));
        }

        // P2-Qwen: Qwen uses a 3-tier env var compat chain
        // (QWEN_API_KEY → DASHSCOPE_API_KEY → MODELSTUDIO_API_KEY) so
        // users with existing DashScope/ModelStudio exports don't need
        // to rename anything. Other providers use their single canonical
        // env_key() directly.
        let api_key = if provider == Provider::Qwen {
            resolve_qwen_api_key()
        } else {
            env::var(provider.env_key()).ok()
        };
        let base_url = match provider {
            Provider::Anthropic => "https://api.anthropic.com".to_string(),
            Provider::OpenAI => "https://api.openai.com".to_string(),
            Provider::Ollama => ollama::normalize_ollama_url(
                &env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string()),
            ),
            Provider::OpenRouter => "https://openrouter.ai/api".to_string(),
            Provider::Google => "https://generativelanguage.googleapis.com".to_string(),
            Provider::Groq => "https://api.groq.com/openai".to_string(),
            Provider::Mistral => "https://api.mistral.ai".to_string(),
            Provider::Together => "https://api.together.xyz".to_string(),
            Provider::Fireworks => "https://api.fireworks.ai/inference".to_string(),
            Provider::Azure => env::var("AZURE_OPENAI_ENDPOINT")
                .unwrap_or_else(|_| "https://your-resource.openai.azure.com".to_string()),
            Provider::Bedrock => {
                let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                format!("https://bedrock-runtime.{}.amazonaws.com", region)
            }
            Provider::DeepSeek => "https://api.deepseek.com".to_string(),
            Provider::XAI => "https://api.x.ai".to_string(),
            Provider::Cerebras => "https://api.cerebras.ai".to_string(),
            Provider::Moonshot => "https://api.moonshot.ai".to_string(),
            Provider::Zai => "https://open.bigmodel.cn/api/paas/v4".to_string(),
            Provider::Qwen => resolve_qwen_base_url(),
            Provider::Minimax => minimax::MINIMAX_INFERENCE_BASE_GLOBAL.to_string(),
            Provider::XiaomiMimo => "https://api.xiaomimimo.com/v1".to_string(),
            Provider::GoogleGeminiCli => "https://cloudcode-pa.googleapis.com".to_string(),
        };

        let auth = if provider == Provider::Ollama {
            // OLLAMA_HOST is a URL, not an API key — Ollama doesn't use auth
            AuthMethod::None
        } else {
            match &api_key {
                Some(key) => AuthMethod::ApiKey(key.clone()),
                None => AuthMethod::None,
            }
        };

        Ok(Self {
            provider,
            model,
            client: build_llm_client(),
            auth,
            base_url,
            thinking_level: None,
            response_format: None,
            seed: None,
            fallback_model: None, persistent_retry: false,
            ollama_temperature: 0.3,
            ollama_num_predict: 1024,
            ollama_num_predict_tools: 4096,
            ollama_keep_alive: "30m".to_string(),
            ollama_top_p: None,
            ollama_top_k: None,
            ollama_repeat_penalty: None,
        })
    }

    /// Create from config with OAuth support
    pub fn from_config(config: &Config) -> Result<Self> {
        // F5: resolve model variant suffixes (:fast / :cheap / :quality)
        // against the configured [model_routing] table before parsing.
        let default_routing = zeus_core::ModelRoutingCoreConfig::default();
        let routing_ref = config.model_routing.as_ref().unwrap_or(&default_routing);
        let resolved_model_str =
            model_variants::resolve_variant(&config.model, routing_ref);
        let (provider, model) = if resolved_model_str == config.model {
            config.parse_model()
        } else {
            let mut temp = config.clone();
            temp.model = resolved_model_str;
            temp.parse_model()
        };

        if model.is_empty() {
            return Err(Error::Llm(
                "No model configured. Please set a model in ~/.zeus/config.toml \
                 (e.g. model = \"anthropic/claude-sonnet-4-20250514\") or configure a provider in Settings."
                    .to_string(),
            ));
        }

        // Determine base URL
        let base_url = match provider {
            Provider::Anthropic => "https://api.anthropic.com".to_string(),
            Provider::OpenAI => "https://api.openai.com".to_string(),
            Provider::Ollama => ollama::normalize_ollama_url(&config.ollama.url),
            Provider::OpenRouter => "https://openrouter.ai/api".to_string(),
            Provider::Google => "https://generativelanguage.googleapis.com".to_string(),
            Provider::Groq => "https://api.groq.com/openai".to_string(),
            Provider::Mistral => "https://api.mistral.ai".to_string(),
            Provider::Together => "https://api.together.xyz".to_string(),
            Provider::Fireworks => "https://api.fireworks.ai/inference".to_string(),
            Provider::Azure => env::var("AZURE_OPENAI_ENDPOINT")
                .unwrap_or_else(|_| "https://your-resource.openai.azure.com".to_string()),
            Provider::Bedrock => {
                let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                format!("https://bedrock-runtime.{}.amazonaws.com", region)
            }
            Provider::DeepSeek => "https://api.deepseek.com".to_string(),
            Provider::XAI => "https://api.x.ai".to_string(),
            Provider::Cerebras => "https://api.cerebras.ai".to_string(),
            Provider::Moonshot => "https://api.moonshot.ai".to_string(),
            Provider::Zai => "https://open.bigmodel.cn/api/paas/v4".to_string(),
            Provider::Qwen => resolve_qwen_base_url(),
            Provider::Minimax => minimax::MINIMAX_INFERENCE_BASE_GLOBAL.to_string(),
            Provider::XiaomiMimo => "https://api.xiaomimimo.com/v1".to_string(),
            Provider::GoogleGeminiCli => "https://cloudcode-pa.googleapis.com".to_string(),
        };

        // Determine auth method — env vars take priority over credential store.
        // This ensures a key set in .env or the environment is always used,
        // even if credentials.json holds a stale or revoked entry for the
        // same provider (S16 bug #2).
        //
        // Priority order:
        //   1. Env var (ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.) — explicit, always wins
        //   2. Credential store (credentials.json) — stored via /login
        //   3. Legacy oauth_tokens.json — backward compat for Anthropic OAuth
        let provider_name = provider.name().to_lowercase();
        // P2-Qwen: Qwen check uses the 3-tier compat chain. Other
        // providers keep the single env_key() lookup.
        let env_key_value: Option<String> = if provider == Provider::Qwen {
            resolve_qwen_api_key()
        } else {
            env::var(provider.env_key()).ok()
        };
        let auth = if provider == Provider::Ollama {
            // Ollama uses no auth token — connection is handled by base_url alone.
            AuthMethod::None
        } else if let Some(key) = env_key_value {
            // Env var explicitly set — takes priority over credential store.
            info!("Using env var {} for {}", provider.env_key(), provider_name);
            AuthMethod::ApiKey(key)
        } else if let Ok(Some(token)) = OAuthManager::get_stored_token(&provider_name) {
            // Credential store has a token and no env var is set.
            let kind = OAuthManager::get_credential(&provider_name)
                .ok()
                .flatten()
                .map(|c| c.kind);
            match kind {
                Some(CredentialKind::SetupToken) | Some(CredentialKind::OAuthToken) => {
                    info!("Using stored OAuth/setup-token for {}", provider_name);
                    AuthMethod::OAuth(token)
                }
                Some(CredentialKind::ApiKey) => {
                    info!("Using stored API key for {}", provider_name);
                    AuthMethod::ApiKey(token)
                }
                None => AuthMethod::ApiKey(token),
            }
        } else {
            // Check per-provider credentials [credentials.{provider}] (new format)
            let cred = match provider {
                Provider::OpenAI => config.provider_credentials.openai.as_ref(),
                Provider::Anthropic => config.provider_credentials.anthropic.as_ref(),
                Provider::Google => config.provider_credentials.google.as_ref(),
                Provider::GoogleGeminiCli => config.provider_credentials.google_gemini_cli.as_ref(),
                Provider::Qwen => config.provider_credentials.qwen.as_ref(),
                Provider::Minimax => config.provider_credentials.minimax.as_ref(),
                Provider::XiaomiMimo => config.provider_credentials.xiaomimimo.as_ref(),
                _ => None,
            };
            if let Some(c) = cred {
                if !c.token.is_empty() {
                    let method = if c.cred_type == "oauth" {
                        info!("Using OAuth credential from [credentials.{}]", provider.name());
                        AuthMethod::OAuth(c.token.clone())
                    } else {
                        info!("Using API key from [credentials.{}]", provider.name());
                        AuthMethod::ApiKey(c.token.clone())
                    };
                    method
                } else {
                    AuthMethod::None
                }
            } else if config.auth.use_oauth {
                // Legacy: check [oauth] section (single-provider format)
                let oauth_provider = config.oauth.provider.as_deref().unwrap_or("");
                let oauth_token = config.oauth.token.as_deref().unwrap_or("");
                let provider_name = provider.name();

                if !oauth_token.is_empty() && (oauth_provider == provider_name || oauth_provider.is_empty()) {
                    info!("Using OAuth token from [oauth] section for {}", provider_name);
                    AuthMethod::OAuth(oauth_token.to_string())
                } else if provider == Provider::Anthropic {
                    // Legacy: check oauth_tokens.json
                    match OAuthTokens::load() {
                        Ok(Some(tokens)) if !tokens.is_expired() => {
                            info!("Using legacy OAuth token for Anthropic");
                            AuthMethod::OAuth(tokens.access_token)
                        }
                        _ => AuthMethod::None,
                    }
                } else {
                    AuthMethod::None
                }
            } else {
                AuthMethod::None
            }
        };

        // Qwen OAuth uses portal.qwen.ai/v1, not the DashScope API
        let base_url = if provider == Provider::Qwen && matches!(auth, AuthMethod::OAuth(_)) {
            qwen_oauth::QWEN_PORTAL_BASE.to_string()
        } else {
            base_url
        };

        Ok(Self {
            provider,
            model,
            client: build_llm_client(),
            auth,
            base_url,
            thinking_level: config.thinking_level.clone(),
            response_format: None,
            seed: None,
            fallback_model: config
                .fallback_models
                .as_ref()
                .and_then(|v| v.first())
                .cloned(),
            persistent_retry: false,
            ollama_temperature: config.ollama.temperature,
            ollama_num_predict: config.ollama.num_predict,
            ollama_num_predict_tools: config.ollama.num_predict_tools,
            ollama_keep_alive: config.ollama.keep_alive.clone(),
            ollama_top_p: config.ollama.top_p,
            ollama_top_k: config.ollama.top_k,
            ollama_repeat_penalty: config.ollama.repeat_penalty,
        })
    }

    /// Create with explicit OAuth token
    pub fn with_oauth(provider: Provider, model: String, token: String) -> Result<Self> {
        let base_url = match provider {
            Provider::Anthropic => "https://api.anthropic.com".to_string(),
            Provider::OpenAI => "https://api.openai.com".to_string(),
            Provider::Ollama => ollama::normalize_ollama_url(
                &env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string()),
            ),
            Provider::OpenRouter => "https://openrouter.ai/api".to_string(),
            Provider::Google => "https://generativelanguage.googleapis.com".to_string(),
            Provider::Groq => "https://api.groq.com/openai".to_string(),
            Provider::Mistral => "https://api.mistral.ai".to_string(),
            Provider::Together => "https://api.together.xyz".to_string(),
            Provider::Fireworks => "https://api.fireworks.ai/inference".to_string(),
            Provider::Azure => env::var("AZURE_OPENAI_ENDPOINT")
                .unwrap_or_else(|_| "https://your-resource.openai.azure.com".to_string()),
            Provider::Bedrock => {
                let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                format!("https://bedrock-runtime.{}.amazonaws.com", region)
            }
            Provider::DeepSeek => "https://api.deepseek.com".to_string(),
            Provider::XAI => "https://api.x.ai".to_string(),
            Provider::Cerebras => "https://api.cerebras.ai".to_string(),
            Provider::Moonshot => "https://api.moonshot.ai".to_string(),
            Provider::Zai => "https://open.bigmodel.cn/api/paas/v4".to_string(),
            Provider::Qwen => resolve_qwen_base_url(),
            Provider::Minimax => minimax::MINIMAX_INFERENCE_BASE_GLOBAL.to_string(),
            Provider::XiaomiMimo => "https://api.xiaomimimo.com/v1".to_string(),
            Provider::GoogleGeminiCli => "https://cloudcode-pa.googleapis.com".to_string(),
        };

        Ok(Self {
            provider,
            model,
            client: build_llm_client(),
            auth: AuthMethod::OAuth(token),
            base_url,
            thinking_level: None,
            response_format: None,
            seed: None,
            fallback_model: None, persistent_retry: false,
            ollama_temperature: 0.3,
            ollama_num_predict: 1024,
            ollama_num_predict_tools: 4096,
            ollama_keep_alive: "30m".to_string(),
            ollama_top_p: None,
            ollama_top_k: None,
            ollama_repeat_penalty: None,
        })
    }

    /// Set a fallback model string ("provider/model") to try if primary fails all retries.
    pub fn with_fallback_model(mut self, model: impl Into<String>) -> Self {
        self.fallback_model = Some(model.into());
        self
    }

    /// Set the thinking level for extended thinking (Anthropic only)
    pub fn with_thinking(mut self, level: impl Into<String>) -> Self {
        self.thinking_level = Some(level.into());
        self
    }

    /// Set seed for deterministic outputs (OpenAI-compatible providers)
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set response format for structured outputs
    pub fn set_response_format(&mut self, format: ResponseFormat) -> &mut Self {
        self.response_format = Some(format);
        self
    }

    /// Clear response format (revert to text)
    pub fn clear_response_format(&mut self) -> &mut Self {
        self.response_format = None;
        self
    }

    /// Inject response_format into a JSON request body (OpenAI-compatible format)
    fn inject_response_format(&self, body: &mut serde_json::Value) {
        if let Some(ref fmt) = self.response_format {
            match fmt {
                ResponseFormat::Text => {
                    body["response_format"] = serde_json::json!({"type": "text"});
                }
                ResponseFormat::JsonObject => {
                    body["response_format"] = serde_json::json!({"type": "json_object"});
                }
                ResponseFormat::JsonSchema {
                    name,
                    schema,
                    strict,
                } => {
                    body["response_format"] = serde_json::json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": name,
                            "schema": schema,
                            "strict": strict,
                        }
                    });
                }
            }
        }
    }

    /// Inject seed into a JSON request body (OpenAI-compatible providers only)
    fn inject_seed(&self, body: &mut serde_json::Value) {
        if let Some(seed) = self.seed {
            // Only inject seed for OpenAI-compatible providers
            if self.provider != Provider::Anthropic {
                body["seed"] = serde_json::json!(seed);
            }
        }
    }

    /// Apply OpenAI sampling parameters (`temperature` and/or `reasoning_effort`)
    /// based on model family + tool usage.
    ///
    /// Uses the free-fn predicates [`is_reasoning_model`] and
    /// [`gpt55_rejects_sampling`] to centralize family-membership tests.
    ///
    /// Rules:
    /// - Reasoning models (o1/o3/o4): no `temperature`, set `reasoning_effort=medium`.
    /// - Moonshot/Kimi: never set `temperature` (K2.5 only accepts temperature=1 server-side).
    /// - **GPT-5.5+ with tools**: rejects BOTH `temperature` AND `reasoning_effort` →
    ///   skip both entirely. Without tools, GPT-5.5 still accepts `reasoning_effort`.
    /// - Everything else: `temperature=0.3`.
    fn inject_openai_sampling(&self, body: &mut serde_json::Value, has_tools: bool) {
        let is_reasoning = is_reasoning_model(&self.model);
        let gpt55_with_tools = gpt55_rejects_sampling(&self.model, has_tools);

        if !is_reasoning && !gpt55_with_tools && self.provider != Provider::Moonshot {
            body["temperature"] = serde_json::json!(0.3);
        } else if is_reasoning && !gpt55_with_tools {
            // o1/o3/o4 always supports reasoning_effort; GPT-5.5 only without tools.
            body["reasoning_effort"] = serde_json::json!("medium");
        }
        // GPT-5.5 with tools: send neither (both rejected by the API).
    }

    /// Get the current authentication method
    pub fn auth_method(&self) -> &AuthMethod {
        &self.auth
    }

    /// Check if client has valid credentials
    pub fn has_credentials(&self) -> bool {
        match &self.auth {
            AuthMethod::ApiKey(_) | AuthMethod::OAuth(_) => true,
            AuthMethod::None => self.provider == Provider::Ollama,
        }
    }

    /// Get detailed credential status with helpful suggestions
    pub fn credential_status(&self) -> CredentialStatus {
        match (&self.provider, &self.auth) {
            (Provider::Anthropic, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Anthropic API key configured".to_string())
            }
            (Provider::Anthropic, AuthMethod::OAuth(_)) => {
                CredentialStatus::Valid("Anthropic setup-token configured".to_string())
            }
            (Provider::Anthropic, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Anthropic".to_string(),
                suggestions: vec![
                    "Set ANTHROPIC_API_KEY environment variable".to_string(),
                    "Use /login to paste a setup-token (run `claude setup-token` first)"
                        .to_string(),
                    "Switch to Ollama with /model ollama/llama3.2".to_string(),
                ],
            },
            (Provider::OpenAI, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("OpenAI API key configured".to_string())
            }
            (Provider::OpenAI, AuthMethod::OAuth(_)) => {
                CredentialStatus::Valid("OpenAI Codex OAuth configured".to_string())
            }
            (Provider::OpenAI, AuthMethod::None) => CredentialStatus::Missing {
                provider: "OpenAI".to_string(),
                suggestions: vec![
                    "Set OPENAI_API_KEY environment variable".to_string(),
                    "Get key from https://platform.openai.com/api-keys".to_string(),
                    "Switch to Ollama with /model ollama/llama3.2".to_string(),
                ],
            },
            (Provider::OpenRouter, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("OpenRouter API key configured".to_string())
            }
            (Provider::OpenRouter, AuthMethod::None) => CredentialStatus::Missing {
                provider: "OpenRouter".to_string(),
                suggestions: vec![
                    "Set OPENROUTER_API_KEY environment variable".to_string(),
                    "Get key from https://openrouter.ai/keys".to_string(),
                ],
            },
            (Provider::Ollama, _) => {
                CredentialStatus::Valid("Ollama (no authentication required)".to_string())
            }
            (Provider::Google, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Google API key configured".to_string())
            }
            (Provider::Google, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Google".to_string(),
                suggestions: vec![
                    "Set GOOGLE_API_KEY environment variable".to_string(),
                    "Get key from https://aistudio.google.com/apikey".to_string(),
                ],
            },
            (Provider::Groq, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Groq API key configured".to_string())
            }
            (Provider::Groq, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Groq".to_string(),
                suggestions: vec![
                    "Set GROQ_API_KEY environment variable".to_string(),
                    "Get key from https://console.groq.com/keys".to_string(),
                ],
            },
            (Provider::Mistral, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Mistral API key configured".to_string())
            }
            (Provider::Mistral, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Mistral".to_string(),
                suggestions: vec![
                    "Set MISTRAL_API_KEY environment variable".to_string(),
                    "Get key from https://console.mistral.ai/api-keys".to_string(),
                ],
            },
            (Provider::Together, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Together AI API key configured".to_string())
            }
            (Provider::Together, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Together".to_string(),
                suggestions: vec![
                    "Set TOGETHER_API_KEY environment variable".to_string(),
                    "Get key from https://api.together.xyz/settings/api-keys".to_string(),
                ],
            },
            (Provider::Fireworks, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Fireworks AI API key configured".to_string())
            }
            (Provider::Fireworks, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Fireworks".to_string(),
                suggestions: vec![
                    "Set FIREWORKS_API_KEY environment variable".to_string(),
                    "Get key from https://fireworks.ai/account/api-keys".to_string(),
                ],
            },
            (Provider::Azure, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("Azure OpenAI API key configured".to_string())
            }
            (Provider::Azure, AuthMethod::None) => CredentialStatus::Missing {
                provider: "Azure OpenAI".to_string(),
                suggestions: vec![
                    "Set AZURE_OPENAI_API_KEY environment variable".to_string(),
                    "Set AZURE_OPENAI_ENDPOINT to your resource URL".to_string(),
                    "Set AZURE_OPENAI_DEPLOYMENT to your deployment name".to_string(),
                    "Get keys from https://portal.azure.com".to_string(),
                ],
            },
            (Provider::Bedrock, AuthMethod::ApiKey(_)) => {
                CredentialStatus::Valid("AWS credentials configured".to_string())
            }
            (Provider::Bedrock, AuthMethod::None) => CredentialStatus::Missing {
                provider: "AWS Bedrock".to_string(),
                suggestions: vec![
                    "Set AWS_ACCESS_KEY_ID environment variable".to_string(),
                    "Set AWS_SECRET_ACCESS_KEY environment variable".to_string(),
                    "Set AWS_REGION (default: us-east-1)".to_string(),
                    "Get credentials from https://console.aws.amazon.com/iam".to_string(),
                ],
            },
            _ => CredentialStatus::Valid("Credentials configured".to_string()),
        }
    }

    /// Send a completion request and get a full response.
    ///
    /// Retries up to 10 times with exponential backoff (500ms base) + jitter.
    /// Retries on: 429 (rate limit), 500/502/503 (server errors), overloaded.
    /// Does NOT retry on: 400 (bad request), 401 (unauthorized), 403 (forbidden).
    /// After all retries exhausted, tries the fallback model (if configured) before giving up.
    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        // Sanitize orphaned tool_use blocks before ANY API call.
        // Position-aware + provider-specific: Moonshot strips orphans, others inject synthetic results.
        let sanitized = Self::sanitize_tool_pairs_for_provider(messages, &self.provider);
        let messages = &sanitized;

        let max_retries: u32 = if self.persistent_retry { u32::MAX } else { 10 };
        let mut attempt = 0u32;
        loop {
            let result = match self.provider {
                Provider::Anthropic => self.complete_anthropic(messages, tools, system).await,
                Provider::OpenAI => {
                    // OAuth tokens route through Codex backend (chatgpt.com/backend-api)
                    if matches!(self.auth, AuthMethod::OAuth(_)) {
                        let token = self.codex_bearer_token().await?;
                        codex::complete_codex(&self.client, &self.model, messages, tools, system, &token).await
                    } else {
                        self.complete_openai(messages, tools, system).await
                    }
                }
                Provider::Ollama => self.complete_openai(messages, tools, system).await,
                Provider::OpenRouter => self.complete_openrouter(messages, tools, system).await,
                Provider::Google => self.complete_google(messages, tools, system).await,
                Provider::Groq => self.complete_groq(messages, tools, system).await,
                Provider::Mistral => self.complete_mistral(messages, tools, system).await,
                Provider::Together => self.complete_together(messages, tools, system).await,
                Provider::Fireworks => self.complete_fireworks(messages, tools, system).await,
                Provider::Azure => self.complete_azure(messages, tools, system).await,
                Provider::Bedrock => self.complete_bedrock(messages, tools, system).await,
                // OpenAI-compatible providers
                Provider::DeepSeek => self.complete_openai(messages, tools, system).await,
                Provider::XAI => self.complete_openai(messages, tools, system).await,
                Provider::Cerebras => self.complete_openai(messages, tools, system).await,
                Provider::Moonshot => self.complete_openai(messages, tools, system).await,
                Provider::Zai => self.complete_openai(messages, tools, system).await,
                Provider::Qwen => self.complete_openai(messages, tools, system).await,
                Provider::XiaomiMimo => self.complete_openai(messages, tools, system).await,
                Provider::Minimax => {
                    let token = minimax::ensure_fresh_minimax_token(&self.client, "global").await
                        .or_else(|| if let AuthMethod::OAuth(t) = &self.auth { Some(t.clone()) } else { None })
                        .or_else(|| if let AuthMethod::ApiKey(k) = &self.auth { Some(k.clone()) } else { None })
                        .ok_or_else(|| Error::Llm("MiniMax OAuth token expired — re-authenticate: zeus onboard (Auth step)".to_string()))?;
                    minimax::complete_minimax(&self.client, &self.model, messages, tools, system, &token, None, "global").await
                }
            Provider::GoogleGeminiCli => self.complete_google_gemini_cli(messages, tools, system).await,
            };
            match result {
                Ok(resp) => return Ok(resp),
                Err(ref e) if attempt < max_retries && Self::is_retryable_error(e) => {
                    attempt += 1;
                    // 500ms base * 2^(attempt-1) + jitter, capped at 5 min for persistent mode
                    let base_delay = 500 * 2u64.pow(attempt.min(12) - 1) + rand_jitter_ms();
                    let delay = std::time::Duration::from_millis(base_delay.min(300_000));
                    warn!(
                        "LLM complete failed (attempt {}{}), retrying in {:?}: {}",
                        attempt,
                        if self.persistent_retry { " [persistent]".to_string() } else { format!("/{}", max_retries) },
                        delay, e
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    // Primary model exhausted — try fallback if configured
                    if let Some(ref fallback) = self.fallback_model {
                        warn!(
                            "Primary model {} failed after {} attempts ({}), trying fallback: {}",
                            self.model, attempt, e, fallback
                        );
                        let fallback_client = self.build_fallback_client(fallback);
                        return fallback_client.complete_inner(messages, tools, system).await;
                    }
                    return Err(e);
                }
            }
        }
    }

    /// Inner complete — runs provider dispatch without retry/fallback (used by fallback client).
    async fn complete_inner(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic(messages, tools, system).await,
            Provider::OpenAI => {
                if matches!(self.auth, AuthMethod::OAuth(_)) {
                    let token = self.codex_bearer_token().await?;
                    codex::complete_codex(&self.client, &self.model, messages, tools, system, &token).await
                } else {
                    self.complete_openai(messages, tools, system).await
                }
            }
            Provider::Ollama => self.complete_openai(messages, tools, system).await,
            Provider::OpenRouter => self.complete_openrouter(messages, tools, system).await,
            Provider::Google => self.complete_google(messages, tools, system).await,
            Provider::Groq => self.complete_groq(messages, tools, system).await,
            Provider::Mistral => self.complete_mistral(messages, tools, system).await,
            Provider::Together => self.complete_together(messages, tools, system).await,
            Provider::Fireworks => self.complete_fireworks(messages, tools, system).await,
            Provider::Azure => self.complete_azure(messages, tools, system).await,
            Provider::Bedrock => self.complete_bedrock(messages, tools, system).await,
            Provider::DeepSeek => self.complete_openai(messages, tools, system).await,
            Provider::XAI => self.complete_openai(messages, tools, system).await,
            Provider::Cerebras => self.complete_openai(messages, tools, system).await,
                Provider::Moonshot => self.complete_openai(messages, tools, system).await,
                Provider::Zai => self.complete_openai(messages, tools, system).await,
                Provider::Qwen => self.complete_openai(messages, tools, system).await,
                Provider::XiaomiMimo => self.complete_openai(messages, tools, system).await,
                Provider::Minimax => {
                    let token = minimax::ensure_fresh_minimax_token(&self.client, "global").await
                        .or_else(|| if let AuthMethod::OAuth(t) = &self.auth { Some(t.clone()) } else { None })
                        .or_else(|| if let AuthMethod::ApiKey(k) = &self.auth { Some(k.clone()) } else { None })
                        .ok_or_else(|| Error::Llm("MiniMax OAuth token expired — re-authenticate: zeus onboard (Auth step)".to_string()))?;
                    minimax::complete_minimax(&self.client, &self.model, messages, tools, system, &token, None, "global").await
                }
            Provider::GoogleGeminiCli => self.complete_google_gemini_cli(messages, tools, system).await,
        }
    }

    /// Build a new LlmClient for the fallback model string ("provider/model").
    /// Inherits auth from the current client where possible.
    fn build_fallback_client(&self, fallback_model: &str) -> LlmClient {
        let (provider, model) = router::parse_model_string(fallback_model);
        // Reuse existing auth if the provider matches, otherwise try env var.
        // P2-Qwen: honor the QWEN_API_KEY / DASHSCOPE_API_KEY /
        // MODELSTUDIO_API_KEY compat chain for Qwen.
        let auth = if provider == self.provider {
            self.auth.clone()
        } else if provider == Provider::Qwen {
            match resolve_qwen_api_key() {
                Some(key) => AuthMethod::ApiKey(key),
                None => AuthMethod::None,
            }
        } else {
            match env::var(provider.env_key()) {
                Ok(key) if !key.is_empty() => AuthMethod::ApiKey(key),
                _ => AuthMethod::None,
            }
        };
        let base_url = match provider {
            Provider::Anthropic => "https://api.anthropic.com".to_string(),
            Provider::OpenAI => "https://api.openai.com".to_string(),
            Provider::Ollama => ollama::normalize_ollama_url(
                &env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string()),
            ),
            Provider::OpenRouter => "https://openrouter.ai/api".to_string(),
            Provider::Google => "https://generativelanguage.googleapis.com".to_string(),
            Provider::Groq => "https://api.groq.com/openai".to_string(),
            Provider::Mistral => "https://api.mistral.ai".to_string(),
            Provider::Together => "https://api.together.xyz".to_string(),
            Provider::Fireworks => "https://api.fireworks.ai/inference".to_string(),
            Provider::Azure => env::var("AZURE_OPENAI_ENDPOINT")
                .unwrap_or_else(|_| "https://your-resource.openai.azure.com".to_string()),
            Provider::Bedrock => {
                let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                format!("https://bedrock-runtime.{}.amazonaws.com", region)
            }
            Provider::DeepSeek => "https://api.deepseek.com".to_string(),
            Provider::XAI => "https://api.x.ai".to_string(),
            Provider::Cerebras => "https://api.cerebras.ai".to_string(),
            Provider::Moonshot => "https://api.moonshot.ai".to_string(),
            Provider::Zai => "https://open.bigmodel.cn/api/paas/v4".to_string(),
            Provider::Qwen => resolve_qwen_base_url(),
            Provider::Minimax => minimax::MINIMAX_INFERENCE_BASE_GLOBAL.to_string(),
            Provider::XiaomiMimo => "https://api.xiaomimimo.com/v1".to_string(),
            Provider::GoogleGeminiCli => "https://cloudcode-pa.googleapis.com".to_string(),
        };
        LlmClient {
            provider,
            model,
            client: build_llm_client(),
            auth,
            base_url,
            thinking_level: self.thinking_level.clone(),
            response_format: self.response_format.clone(),
            seed: self.seed,
            fallback_model: None, persistent_retry: false, // fallback doesn't chain further
            ollama_temperature: self.ollama_temperature,
            ollama_num_predict: self.ollama_num_predict,
            ollama_num_predict_tools: self.ollama_num_predict_tools,
            ollama_keep_alive: self.ollama_keep_alive.clone(),
            ollama_top_p: self.ollama_top_p,
            ollama_top_k: self.ollama_top_k,
            ollama_repeat_penalty: self.ollama_repeat_penalty,
        }
    }

    /// Check if an LLM error is retryable.
    ///
    /// Retries on: 429 (rate limit), 500/502/503 (server errors), overloaded.
    /// Does NOT retry on: 400, 401, 403 (client errors — retrying won't help).
    fn is_retryable_error(e: &Error) -> bool {
        let msg = e.to_string();
        // Hard-fail on auth/client errors — retrying won't help
        if msg.contains("401 ") || msg.contains("403 ") || msg.contains("400 ") {
            return false;
        }
        msg.contains("429") || msg.contains("rate_limit")
            || msg.contains("500 ") || msg.contains("502 ")
            || msg.contains("503 ") || msg.contains("overloaded")
    }

    /// Send a streaming completion request with exponential backoff retry.
    ///
    /// Retries the initial HTTP handshake up to 10 times (500ms base + jitter)
    /// on 429/500/502/503. Once the stream starts, no mid-stream retry — the
    /// caller gets the receiver and handles partial content.
    /// After all retries exhausted, tries the fallback model (if configured) before giving up.
    pub async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        // Sanitize orphaned tool_use blocks before API call.
        // Position-aware + provider-specific sanitization.
        let sanitized = Self::sanitize_tool_pairs_for_provider(messages, &self.provider);
        let messages = &sanitized;

        let max_retries: u32 = if self.persistent_retry { u32::MAX } else { 10 };
        let mut attempt = 0u32;
        loop {
            let result = match self.provider {
                Provider::Anthropic => self.stream_anthropic(messages, tools, system).await,
                Provider::OpenAI => {
                    if matches!(self.auth, AuthMethod::OAuth(_)) {
                        let token = self.codex_bearer_token().await?;
                        codex::stream_codex(&self.client, &self.model, messages, tools, system, &token).await
                    } else {
                        self.stream_openai(messages, tools, system).await
                    }
                }
                Provider::Ollama => self.stream_openai(messages, tools, system).await,
                Provider::OpenRouter => self.stream_openrouter(messages, tools, system).await,
                Provider::Google => self.stream_google(messages, tools, system).await,
                Provider::Groq => self.stream_groq(messages, tools, system).await,
                Provider::Mistral => self.stream_mistral(messages, tools, system).await,
                Provider::Together => self.stream_together(messages, tools, system).await,
                Provider::Fireworks => self.stream_fireworks(messages, tools, system).await,
                Provider::Azure => self.stream_azure(messages, tools, system).await,
                Provider::Bedrock => self.stream_bedrock(messages, tools, system).await,
                Provider::DeepSeek => self.stream_openai(messages, tools, system).await,
                Provider::XAI => self.stream_openai(messages, tools, system).await,
                Provider::Cerebras => self.stream_openai(messages, tools, system).await,
                Provider::Moonshot => self.stream_openai(messages, tools, system).await,
                Provider::Zai => self.stream_openai(messages, tools, system).await,
                Provider::Qwen => self.stream_openai(messages, tools, system).await,
                Provider::XiaomiMimo => self.stream_openai(messages, tools, system).await,
                Provider::Minimax => {
                    let token = minimax::ensure_fresh_minimax_token(&self.client, "global").await
                        .or_else(|| if let AuthMethod::OAuth(t) = &self.auth { Some(t.clone()) } else { None })
                        .or_else(|| if let AuthMethod::ApiKey(k) = &self.auth { Some(k.clone()) } else { None })
                        .ok_or_else(|| Error::Llm("MiniMax OAuth token expired — re-authenticate: zeus onboard (Auth step)".to_string()))?;
                    minimax::stream_minimax(&self.client, &self.model, messages, tools, system, &token, None, "global").await
                }
            Provider::GoogleGeminiCli => self.stream_google_gemini_cli(messages, tools, system).await,
            };
            match result {
                Ok(pair) => return Ok(pair),
                Err(ref e) if attempt < max_retries && Self::is_retryable_error(e) => {
                    attempt += 1;
                    let base_delay = 500 * 2u64.pow(attempt.min(12) - 1) + rand_jitter_ms();
                    let delay = std::time::Duration::from_millis(base_delay.min(300_000));
                    warn!(
                        "LLM stream failed (attempt {}{}), retrying in {:?}: {}",
                        attempt,
                        if self.persistent_retry { " [persistent]".to_string() } else { format!("/{}", max_retries) },
                        delay, e
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    // Primary model exhausted — try fallback if configured
                    if let Some(ref fallback) = self.fallback_model {
                        warn!(
                            "Primary model {} stream failed after {} attempts ({}), trying fallback: {}",
                            self.model, attempt, e, fallback
                        );
                        let fallback_client = self.build_fallback_client(fallback);
                        return fallback_client.stream_inner(messages, tools, system).await;
                    }
                    return Err(e);
                }
            }
        }
    }

    /// Inner stream — runs provider dispatch without retry/fallback (used by fallback client).
    async fn stream_inner(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        match self.provider {
            Provider::Anthropic => self.stream_anthropic(messages, tools, system).await,
            Provider::OpenAI => {
                if matches!(self.auth, AuthMethod::OAuth(_)) {
                    let token = self.codex_bearer_token().await?;
                    codex::stream_codex(&self.client, &self.model, messages, tools, system, &token).await
                } else {
                    self.stream_openai(messages, tools, system).await
                }
            }
            Provider::Ollama => self.stream_openai(messages, tools, system).await,
            Provider::OpenRouter => self.stream_openrouter(messages, tools, system).await,
            Provider::Google => self.stream_google(messages, tools, system).await,
            Provider::Groq => self.stream_groq(messages, tools, system).await,
            Provider::Mistral => self.stream_mistral(messages, tools, system).await,
            Provider::Together => self.stream_together(messages, tools, system).await,
            Provider::Fireworks => self.stream_fireworks(messages, tools, system).await,
            Provider::Azure => self.stream_azure(messages, tools, system).await,
            Provider::Bedrock => self.stream_bedrock(messages, tools, system).await,
            Provider::DeepSeek => self.stream_openai(messages, tools, system).await,
            Provider::XAI => self.stream_openai(messages, tools, system).await,
            Provider::Cerebras => self.stream_openai(messages, tools, system).await,
                Provider::Moonshot => self.stream_openai(messages, tools, system).await,
                Provider::Zai => self.stream_openai(messages, tools, system).await,
                Provider::Qwen => self.stream_openai(messages, tools, system).await,
                Provider::XiaomiMimo => self.stream_openai(messages, tools, system).await,
                Provider::Minimax => {
                    let token = minimax::ensure_fresh_minimax_token(&self.client, "global").await
                        .or_else(|| if let AuthMethod::OAuth(t) = &self.auth { Some(t.clone()) } else { None })
                        .or_else(|| if let AuthMethod::ApiKey(k) = &self.auth { Some(k.clone()) } else { None })
                        .ok_or_else(|| Error::Llm("MiniMax OAuth token expired — re-authenticate: zeus onboard (Auth step)".to_string()))?;
                    minimax::stream_minimax(&self.client, &self.model, messages, tools, system, &token, None, "global").await
                }
            Provider::GoogleGeminiCli => self.stream_google_gemini_cli(messages, tools, system).await,
        }
    }

    // ========================================================================
    // Embeddings
    // ========================================================================

    /// Generate embeddings for the given input texts.
    ///
    /// Routes to the appropriate provider's embedding endpoint:
    /// - OpenAI: POST /v1/embeddings (text-embedding-3-small)
    /// - Ollama: POST /api/embeddings (nomic-embed-text)
    /// - Others: falls back to OpenAI-compatible /v1/embeddings
    pub async fn embed(&self, input: &[String], model: Option<&str>) -> Result<EmbeddingResponse> {
        match self.provider {
            Provider::OpenAI => self.embed_openai(input, model).await,
            Provider::Ollama => self.embed_ollama(input, model).await,
            // Providers with OpenAI-compatible embedding endpoints
            Provider::Together | Provider::Fireworks | Provider::Groq | Provider::Mistral
            | Provider::DeepSeek | Provider::XAI | Provider::Cerebras
            | Provider::Moonshot | Provider::Zai | Provider::Qwen => {
                self.embed_openai_compat(input, model).await
            }
            _ => Err(Error::Llm(format!(
                "Embeddings not supported for provider {:?}. Use OpenAI or Ollama.",
                self.provider
            ))),
        }
    }

    /// OpenAI embeddings: POST https://api.openai.com/v1/embeddings
    async fn embed_openai(
        &self,
        input: &[String],
        model: Option<&str>,
    ) -> Result<EmbeddingResponse> {
        let embed_model = model.unwrap_or("text-embedding-3-small");
        let url = format!("{}/v1/embeddings", self.base_url);

        let body = serde_json::json!({
            "model": embed_model,
            "input": input,
        });

        let mut req = self.client.post(&url).json(&body);
        if let AuthMethod::ApiKey(ref key) = self.auth {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| Error::Llm(format!("OpenAI embeddings request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!(
                "OpenAI embeddings error {status}: {body}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("OpenAI embeddings parse error: {e}")))?;

        self.parse_openai_embedding_response(&json, embed_model)
    }

    /// OpenAI-compatible embeddings for Together, Fireworks, Groq, Mistral
    async fn embed_openai_compat(
        &self,
        input: &[String],
        model: Option<&str>,
    ) -> Result<EmbeddingResponse> {
        let default_model = match self.provider {
            Provider::Together => "togethercomputer/m2-bert-80M-8k-retrieval",
            Provider::Fireworks => "nomic-ai/nomic-embed-text-v1.5",
            Provider::Groq => "nomic-embed-text-v1.5",
            Provider::Mistral => "mistral-embed",
            _ => "text-embedding-3-small",
        };
        let embed_model = model.unwrap_or(default_model);
        let url = format!("{}/v1/embeddings", self.base_url);

        let body = serde_json::json!({
            "model": embed_model,
            "input": input,
        });

        let mut req = self.client.post(&url).json(&body);
        if let AuthMethod::ApiKey(ref key) = self.auth {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.map_err(|e| {
            Error::Llm(format!(
                "{:?} embeddings request failed: {e}",
                self.provider
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!(
                "{:?} embeddings error {status}: {body}",
                self.provider
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("{:?} embeddings parse error: {e}", self.provider)))?;

        self.parse_openai_embedding_response(&json, embed_model)
    }

    /// Ollama embeddings: POST {base}/api/embeddings (one at a time)
    async fn embed_ollama(
        &self,
        input: &[String],
        model: Option<&str>,
    ) -> Result<EmbeddingResponse> {
        let embed_model = model.unwrap_or("nomic-embed-text");
        let url = format!("{}/api/embeddings", self.base_url);

        let mut embeddings = Vec::with_capacity(input.len());

        for (i, text) in input.iter().enumerate() {
            let body = serde_json::json!({
                "model": embed_model,
                "prompt": text,
            });

            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Llm(format!("Ollama embeddings request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                return Err(Error::Llm(format!(
                    "Ollama embeddings error {status}: {body_text}"
                )));
            }

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| Error::Llm(format!("Ollama embeddings parse error: {e}")))?;

            let vec = json
                .get("embedding")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>())
                .unwrap_or_default();

            embeddings.push(Embedding {
                index: i,
                embedding: vec,
            });
        }

        Ok(EmbeddingResponse {
            model: embed_model.to_string(),
            data: embeddings,
            usage: EmbeddingUsage {
                prompt_tokens: 0, // Ollama doesn't report tokens
                total_tokens: 0,
            },
        })
    }

    /// Parse an OpenAI-format embedding response JSON
    fn parse_openai_embedding_response(
        &self,
        json: &serde_json::Value,
        model: &str,
    ) -> Result<EmbeddingResponse> {
        let data = json
            .get("data")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Llm("Missing 'data' array in embedding response".to_string()))?;

        let embeddings: Vec<Embedding> = data
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let vec = item
                    .get("embedding")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>())
                    .unwrap_or_default();
                Embedding {
                    index: item
                        .get("index")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(i as u64) as usize,
                    embedding: vec,
                }
            })
            .collect();

        let usage = json.get("usage").cloned().unwrap_or_default();
        let prompt_tokens = usage
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let total_tokens = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(EmbeddingResponse {
            model: json
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(model)
                .to_string(),
            data: embeddings,
            usage: EmbeddingUsage {
                prompt_tokens,
                total_tokens,
            },
        })
    }

    // ========================================================================
    // Anthropic Implementation
    // ========================================================================

    /// Get thinking budget tokens for a given level
    fn thinking_budget(level: &str) -> usize {
        match level {
            "low" => 2048,
            "medium" => 8192,
            "high" => 32768,
            "xhigh" => 65536,
            _ => 8192,
        }
    }

    async fn complete_anthropic(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let anthropic_messages = self.to_anthropic_messages(messages);
        let anthropic_tools = self.to_anthropic_tools(tools);

        let max_tokens = if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            budget + 8192
        } else {
            8192
        };

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": anthropic_messages,
        });

        // Add extended thinking if configured
        if let Some(ref level) = self.thinking_level {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": Self::thinking_budget(level)
            });
        }

        if let Some(sys) = system {
            if matches!(self.auth, AuthMethod::OAuth(_)) {
                // OAuth: system must be array with Claude Code identity as first block
                body["system"] = serde_json::json!([
                    {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."},
                    {"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}
                ]);
            } else {
                // Use content block array for cache_control support
                body["system"] = serde_json::json!([
                    {"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}
                ]);
            }
        } else if matches!(self.auth, AuthMethod::OAuth(_)) {
            body["system"] = serde_json::json!([
                {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude.", "cache_control": {"type": "ephemeral"}}
            ]);
        }

        if !tools.is_empty() {
            // Add cache_control to the last tool definition
            let mut tools_with_cache = anthropic_tools;
            if let Some(arr) = tools_with_cache.as_array_mut()
                && let Some(last) = arr.last_mut()
            {
                last["cache_control"] = serde_json::json!({"type": "ephemeral"});
            }
            body["tools"] = tools_with_cache;
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        debug!(
            "Anthropic request: {}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        // Inject metadata for OAuth (Claude Code fingerprint signal #4)
        if matches!(self.auth, AuthMethod::OAuth(_)) {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut h);
            let session_id = format!("{:016x}", h.finish());
            body["metadata"] = serde_json::json!({
                "user_id": format!("user_zeus_account_zeus_session_{}", &session_id[..8])
            });
        }

        let mut request = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        // Add authentication header
        request = match &self.auth {
            AuthMethod::ApiKey(key) => request.header("x-api-key", key),
            AuthMethod::OAuth(token) => request
                .header("Authorization", format!("Bearer {}", token))
                .header(
                    "anthropic-beta",
                    "oauth-2025-04-20,claude-code-20250219,interleaved-thinking-2025-05-14",
                )
                .header("user-agent", "claude-cli/2.1.2")
                .header("x-app", "cli")
                .header("anthropic-dangerous-direct-browser-access", "true"),
            AuthMethod::None => {
                return Err(Error::Llm(
                    "No API key or OAuth token configured for Anthropic".to_string(),
                ));
            }
        };

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Anthropic API error {}: {}",
                status, text
            )));
        }

        self.parse_anthropic_response(&text)
    }

    async fn stream_anthropic(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let auth = self.auth.clone();

        let anthropic_messages = self.to_anthropic_messages(messages);
        let anthropic_tools = self.to_anthropic_tools(tools);

        let max_tokens = if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            budget + 8192
        } else {
            8192
        };

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": anthropic_messages,
            "stream": true,
        });

        // Add extended thinking if configured
        if let Some(ref level) = self.thinking_level {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": Self::thinking_budget(level)
            });
        }

        if let Some(sys) = system {
            if matches!(self.auth, AuthMethod::OAuth(_)) {
                body["system"] = serde_json::json!([
                    {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."},
                    {"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}
                ]);
            } else {
                body["system"] = serde_json::json!([
                    {"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}
                ]);
            }
        } else if matches!(self.auth, AuthMethod::OAuth(_)) {
            body["system"] = serde_json::json!([
                {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude.", "cache_control": {"type": "ephemeral"}}
            ]);
        }

        if !tools.is_empty() {
            let mut tools_with_cache = anthropic_tools;
            if let Some(arr) = tools_with_cache.as_array_mut()
                && let Some(last) = arr.last_mut()
            {
                last["cache_control"] = serde_json::json!({"type": "ephemeral"});
            }
            body["tools"] = tools_with_cache;
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        // Inject metadata for OAuth (Claude Code fingerprint signal #4)
        if matches!(self.auth, AuthMethod::OAuth(_)) {
            body["metadata"] = serde_json::json!({
                "user_id": "user_zeus_account_zeus_session_stream"
            });
        }

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let use_oauth = matches!(self.auth, AuthMethod::OAuth(_));

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut current_tool: Option<(String, String, String)> = None; // (id, name, args)
            let mut in_tokens: usize = 0;
            let mut out_tokens: usize = 0;
            let mut c_tokens: usize = 0;

            let mut request = client
                .post(format!("{}/v1/messages", base_url))
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");

            // Add authentication header + fingerprint headers for OAuth
            request = match &auth {
                AuthMethod::ApiKey(key) => request.header("x-api-key", key),
                AuthMethod::OAuth(token) => request
                    .header("Authorization", format!("Bearer {}", token))
                    .header(
                        "anthropic-beta",
                        "oauth-2025-04-20,claude-code-20250219,interleaved-thinking-2025-05-14",
                    )
                    .header("user-agent", "claude-cli/2.1.2")
                    .header("x-app", "cli")
                    .header("anthropic-dangerous-direct-browser-access", "true"),
                AuthMethod::None => {
                    debug!("No authentication configured for Anthropic");
                    return LlmResponse {
                        content: "No API key or OAuth token configured for Anthropic. Please set ANTHROPIC_API_KEY or enable OAuth in Settings.".to_string(),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            let response = match request.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    error!("Stream request failed: {}", e);
                    let _ = tx
                        .send(format!("[Error: stream request failed: {}]", e))
                        .await;
                    return LlmResponse {
                        content: format!("Error: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Anthropic API error {}: {}", status, error_body);

                let err_msg = format!("Anthropic API error ({}): {}", status, error_body);

                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();
            let mut sse_buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                sse_buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process only complete SSE lines (newline-terminated).
                // TCP chunks can split mid-line — buffering prevents
                // partial JSON from being silently dropped.
                while let Some(newline_pos) = sse_buffer.find('\n') {
                    let line = sse_buffer[..newline_pos].to_string();
                    sse_buffer = sse_buffer[newline_pos + 1..].to_string();

                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                        // Handle content_block_start for tool_use
                        if event.get("type").and_then(|t| t.as_str()) == Some("content_block_start")
                            && let Some(block) = event.get("content_block")
                            && block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                        {
                            let id = block
                                .get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            // Map Claude Code names back to Zeus names when using OAuth
                            let mapped_name = if use_oauth {
                                LlmClient::zeus_tool_name(&name).to_string()
                            } else {
                                name
                            };
                            current_tool = Some((id, mapped_name, String::new()));
                        }

                        // Handle content_block_delta
                        if event.get("type").and_then(|t| t.as_str()) == Some("content_block_delta")
                            && let Some(delta) = event.get("delta")
                        {
                            // Text delta
                            if let Some(text_delta) = delta.get("text").and_then(|t| t.as_str()) {
                                content.push_str(text_delta);
                                let _ = tx.send(text_delta.to_string()).await;
                            }
                            // Tool input delta
                            if let Some(partial) =
                                delta.get("partial_json").and_then(|p| p.as_str())
                                && let Some((_, _, ref mut args)) = current_tool
                            {
                                args.push_str(partial);
                            }
                        }

                        // Handle content_block_stop - finalize tool call
                        if event.get("type").and_then(|t| t.as_str()) == Some("content_block_stop")
                            && let Some((id, name, args)) = current_tool.take()
                        {
                            let arguments =
                                serde_json::from_str(&zeus_core::strip_json_markdown(&args)).unwrap_or(serde_json::json!({}));
                            tool_calls.push(ToolCall {
                                id,
                                name,
                                arguments,
                            });
                        }

                        // Handle message_start for input token count + cached tokens
                        if event.get("type").and_then(|t| t.as_str()) == Some("message_start")
                            && let Some(msg) = event.get("message")
                            && let Some(usage) = msg.get("usage")
                            && let Some(it) = usage.get("input_tokens").and_then(|v| v.as_u64())
                        {
                            in_tokens = it as usize;
                            if let Some(ct) = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
                                c_tokens = ct as usize;
                            }
                        }

                        // Handle message_delta for stop_reason and output tokens
                        if event.get("type").and_then(|t| t.as_str()) == Some("message_delta") {
                            if let Some(delta) = event.get("delta")
                                && let Some(reason) =
                                    delta.get("stop_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_use" => StopReason::ToolUse,
                                    "max_tokens" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                            if let Some(usage) = event.get("usage")
                                && let Some(ot) =
                                    usage.get("output_tokens").and_then(|v| v.as_u64())
                            {
                                out_tokens = ot as usize;
                            }
                        }
                    }
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens: in_tokens,
                output_tokens: out_tokens,
                cached_tokens: c_tokens,
            }
        });

        Ok((rx, handle))
    }

    /// Check whether the current provider+model combination supports image input.
    /// Returns true if images should be included, false if they should be silently
    /// dropped, and logs a warning if the model explicitly doesn't support vision.
    fn should_include_images(&self) -> bool {
        match capabilities::supports_image_input(&self.provider, &self.model) {
            Ok(true) => true,
            Ok(false) => {
                tracing::debug!(
                    provider = ?self.provider,
                    model = %self.model,
                    "Provider does not support vision — dropping image attachments"
                );
                false
            }
            Err(msg) => {
                tracing::warn!(
                    provider = ?self.provider,
                    model = %self.model,
                    error = %msg,
                    "Model does not support image input — dropping image attachments"
                );
                false
            }
        }
    }

    /// Check if any message in the batch contains image attachments.
    /// Used to auto-route XiaomiMimo vision requests to the vision-capable model variant.
    fn messages_have_images(&self, messages: &[Message]) -> bool {
        messages.iter().any(|m| !m.attachments.is_empty())
    }

    fn to_anthropic_messages(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        // Track tool_use IDs from assistant messages to validate tool_results.
        let mut known_tool_use_ids = std::collections::HashSet::new();
        // Track tool_result IDs globally to prevent duplicates across messages.
        // Anthropic rejects "each tool_use must have a single result" otherwise.
        let mut global_tool_result_ids = std::collections::HashSet::new();

        for msg in messages {
            match msg.role {
                Role::System => continue, // System handled separately
                Role::User => {
                    if msg.attachments.is_empty() || !self.should_include_images() {
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": msg.content
                        }));
                    } else {
                        // Multi-content block: images, documents, text
                        let mut content_blocks = Vec::new();
                        for att in &msg.attachments {
                            if let Some(block) = crate::multimodal::format_anthropic_attachment(att) {
                                content_blocks.push(block);
                            }
                        }
                        if !msg.content.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": msg.content
                            }));
                        }
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": content_blocks
                        }));
                    }
                }
                Role::Assistant => {
                    let mut content = Vec::new();
                    if !msg.content.is_empty() {
                        content.push(serde_json::json!({
                            "type": "text",
                            "text": msg.content
                        }));
                    }
                    let use_oauth_names = matches!(self.auth, AuthMethod::OAuth(_));
                    for tc in &msg.tool_calls {
                        let name = if use_oauth_names {
                            Self::oauth_tool_name(&tc.name).to_string()
                        } else {
                            tc.name.clone()
                        };
                        known_tool_use_ids.insert(tc.id.clone());
                        content.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": name,
                            "input": tc.arguments
                        }));
                    }
                    if !content.is_empty() {
                        result.push(serde_json::json!({
                            "role": "assistant",
                            "content": content
                        }));
                    }
                }
                Role::Tool => {
                    // Dedup tool_results by call_id — Anthropic rejects
                    // duplicate tool_result blocks for the same tool_use_id.
                    // Uses global tracker to catch duplicates across messages too.
                    let content: Vec<_> = msg
                        .tool_results
                        .iter()
                        .filter(|tr| global_tool_result_ids.insert(tr.call_id.clone()))
                        .filter(|tr| known_tool_use_ids.contains(&tr.call_id))
                        .map(|tr| {
                            serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tr.call_id,
                                "content": tr.output,
                                "is_error": !tr.success
                            })
                        })
                        .collect();
                    if !content.is_empty() {
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": content
                        }));
                    }
                }
            }
        }

        result
    }

    /// Map Zeus tool names to Claude Code equivalents for OAuth compatibility
    fn oauth_tool_name(name: &str) -> &str {
        match name {
            "read_file" => "Read",
            "write_file" => "Write",
            "edit_file" => "Edit",
            "shell" => "Bash",
            "list_dir" => "Glob",
            "web_fetch" => "WebFetch",
            "spawn" => "Task",
            "message" => "Bash",
            "link_understanding" => "WebFetch",
            "media_understanding" => "Read",
            "auto_reply" => "Bash",
            "polls" => "Bash",
            "gmail_pubsub" => "Bash",
            other => other,
        }
    }

    /// Map Claude Code tool names back to Zeus equivalents
    fn zeus_tool_name(name: &str) -> &str {
        match name {
            "Read" => "read_file",
            "Write" => "write_file",
            "Edit" => "edit_file",
            "Bash" => "shell",
            "Glob" => "list_dir",
            "WebFetch" => "web_fetch",
            "Task" => "spawn",
            other => other,
        }
    }

    fn to_anthropic_tools(&self, tools: &[ToolSchema]) -> serde_json::Value {
        let use_oauth_names = matches!(self.auth, AuthMethod::OAuth(_));
        let tools: Vec<_> = tools
            .iter()
            .map(|t| {
                let name = if use_oauth_names {
                    Self::oauth_tool_name(&t.name).to_string()
                } else {
                    t.name.clone()
                };
                serde_json::json!({
                    "name": name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();
        // Deduplicate tools by name (multiple Zeus tools may map to same CC name)
        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<_> = tools
            .into_iter()
            .filter(|t| {
                let name = t["name"].as_str().unwrap_or("").to_string();
                seen.insert(name)
            })
            .collect();
        serde_json::Value::Array(deduped)
    }

    fn parse_anthropic_response(&self, text: &str) -> Result<LlmResponse> {
        let response: serde_json::Value = serde_json::from_str(text)?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(blocks) = response.get("content").and_then(|c| c.as_array()) {
            for block in blocks {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            content.push_str(text);
                        }
                    }
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let raw_name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        // Map Claude Code names back to Zeus names when using OAuth
                        let name = if matches!(self.auth, AuthMethod::OAuth(_)) {
                            Self::zeus_tool_name(raw_name).to_string()
                        } else {
                            raw_name.to_string()
                        };
                        let arguments =
                            block.get("input").cloned().unwrap_or(serde_json::json!({}));
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = match response.get("stop_reason").and_then(|r| r.as_str()) {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        // Extract token usage from Anthropic response
        let usage = response.get("usage");
        let input_tokens = usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let output_tokens = usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let cached_tokens = usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(LlmResponse {
            content,
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
            cached_tokens,
        })
    }

    // ========================================================================
    // OpenAI Implementation
    // ========================================================================

    /// Returns the chat completions endpoint URL for this provider.
    /// ZAI (GLM) uses `/v4/chat/completions` — the base_url already contains `/v4`.
    /// All other OpenAI-compatible providers use the standard `/v1/chat/completions`.
    fn completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        // Providers with versioned base URLs: don't double the /v1/
        if base.ends_with("/v1") || base.ends_with("/v4") || base.ends_with("/v3") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    /// OpenAI-specific execution bias prompt (matches OpenClaw's OPENAI_GPT5_EXECUTION_BIAS).
    /// Injected into system prompt for OpenAI models to encourage tool execution over planning.
    const OPENAI_EXECUTION_BIAS: &'static str = "\n\n## Execution Bias\n\nStart the real work in the same turn when the next step is clear.\nDo prerequisite lookup or discovery before dependent actions.\nIf another tool call would likely improve correctness or completeness, keep going instead of stopping at partial progress.\nMulti-part requests stay incomplete until every requested item is handled or clearly marked blocked.\nKeep responses concise. Prefer tool actions over explanations.";

    /// Ollama execution bias — stronger than OpenAI's because local models are
    /// more prone to describing tool usage instead of calling tools. Qwen 3.5
    /// and similar models sometimes hallucinate tool restrictions.
    const OLLAMA_EXECUTION_BIAS: &'static str = "\n\n## Execution Bias — CRITICAL\n\nYou MUST call tools when tasks require action. Do NOT describe what you would do — actually do it by making tool calls.\nDo NOT claim tools are blocked, restricted, or unavailable unless you receive an actual error from calling them.\nDo NOT invent security policies, taint restrictions, or permission issues that don't exist.\nIf a tool call fails, report the actual error — don't fabricate one.\nPrefer tool actions over explanations. Execute first, explain after.\nKeep responses concise.";

    async fn complete_openai(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key_owned: String;
        // For Qwen OAuth: refresh token if near expiry before using it
        let refreshed_qwen_token: Option<String> = if self.provider == Provider::Qwen && matches!(self.auth, AuthMethod::OAuth(_)) {
            qwen_oauth::ensure_fresh_qwen_token(&self.client).await
        } else {
            None
        };
        let api_key = match (&self.auth, &refreshed_qwen_token) {
            (_, Some(fresh)) => { api_key_owned = fresh.clone(); &api_key_owned }
            (AuthMethod::ApiKey(key), _) => key.as_str(),
            (AuthMethod::OAuth(token), _) => token.as_str(),
            (AuthMethod::None, _) if self.provider == Provider::Ollama => {
                api_key_owned = "ollama".to_string();
                &api_key_owned
            }
            _ => return Err(Error::Llm("OPENAI_API_KEY not set".to_string())),
        };

        // Inject execution bias — all providers, including Ollama. Non-Claude models
        // (Qwen, GPT, GLM) sometimes describe tool usage instead of calling tools.
        // This prompt pushes them toward actual execution.
        let enhanced_system = if self.provider == Provider::Ollama {
            system.map(|s| format!("{}{}", s, Self::OLLAMA_EXECUTION_BIAS))
        } else {
            system.map(|s| format!("{}{}", s, Self::OPENAI_EXECUTION_BIAS))
        };
        let mut openai_messages = self.to_openai_messages(messages, enhanced_system.as_deref().or(system));
        let openai_tools = self.to_openai_tools(tools);

        // Smart context trimming: enforce the model's context window limit.
        // Estimate token count (~4 chars per token), trim oldest messages if over 80% capacity.
        // Keeps system prompt (first message) + most recent messages. Drops middle history.
        {
            let caps = capabilities::capabilities(&self.provider);
            let max_tokens = caps.context_window;
            let target = (max_tokens as f64 * 0.8) as usize; // leave 20% for response
            let estimated_tokens: usize = openai_messages.iter()
                .map(|m| m.to_string().len() / 4)
                .sum();
            if estimated_tokens > target && openai_messages.len() > 2 {
                let system_msg = openai_messages.first().cloned();
                let mut trimmed = Vec::new();
                if let Some(sys) = system_msg {
                    trimmed.push(sys);
                }
                // Keep recent messages, drop oldest (skip system at index 0)
                let non_system: Vec<_> = openai_messages.drain(1..).collect();
                let mut running_tokens = trimmed.iter().map(|m| m.to_string().len() / 4).sum::<usize>();
                // Also truncate individual messages with huge content (base64 images, large tool outputs)
                let truncated: Vec<_> = non_system.into_iter().map(|mut m| {
                    if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                        if content.len() > 50_000 {
                            m["content"] = serde_json::json!(format!(
                                "[Content truncated: {} chars → 500 char summary]\n{}",
                                content.len(),
                                &content[..500.min(content.len())]
                            ));
                        }
                    }
                    m
                }).collect();
                // Add messages from newest to oldest until we hit the budget
                let mut to_add: Vec<_> = truncated.into_iter().rev().take_while(|m| {
                    let msg_tokens = m.to_string().len() / 4;
                    running_tokens += msg_tokens;
                    running_tokens < target
                }).collect();
                to_add.reverse();
                let dropped = openai_messages.len().saturating_sub(1).saturating_sub(to_add.len());
                trimmed.extend(to_add);
                if dropped > 0 {
                    info!(
                        "Context trimmed: {} messages dropped, {} → {} estimated tokens (model limit: {})",
                        dropped, estimated_tokens, trimmed.iter().map(|m| m.to_string().len() / 4).sum::<usize>(), max_tokens
                    );
                }
                openai_messages = trimmed;
            }
        }

        let is_reasoning = is_reasoning_model(&self.model);
        let uses_new_api = is_reasoning || self.model.starts_with("gpt-5") || self.model.starts_with("gpt-4o");
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        // GPT-5/4o/reasoning models use max_completion_tokens; older models use max_tokens
        if uses_new_api {
            body["max_completion_tokens"] = serde_json::json!(4096);
        } else {
            body["max_tokens"] = serde_json::json!(4096);
        }
        self.inject_openai_sampling(&mut body, !tools.is_empty());

        // XiaomiMimo vision auto-route: mimo-v2.5-pro doesn't have a vision endpoint,
        // but mimo-v2.5 does. When image attachments are present, override the model name
        // for that single request. Text-only requests keep mimo-v2.5-pro.
        if self.provider == Provider::XiaomiMimo && self.messages_have_images(messages) {
            body["model"] = serde_json::json!("mimo-v2.5");
            tracing::info!("XiaomiMimo vision auto-route: overriding model to mimo-v2.5 for image request");
        }

        // Moonshot/Kimi + XiaomiMimo thinking mode — models with "thinking" in name get it explicitly.
        // K2.5 / MiMo v2.5+ enable thinking server-side by default, but Zeus can't yet capture/replay
        // reasoning_content in session history, which causes 400 errors on multi-turn.
        // Disable thinking for non-thinking models to avoid session corruption.
        //
        // Bug-5: Skip thinking param entirely for XiaomiMimo image requests.
        // MiMo vision endpoint rejects unknown top-level `thinking` param with misleading 404.
        // Text-only requests keep thinking-disabled (preserves Bug-4 fix).
        let skip_thinking = self.provider == Provider::XiaomiMimo && self.messages_have_images(messages);
        if !skip_thinking && (self.provider == Provider::Moonshot || self.provider == Provider::XiaomiMimo) {
            if self.model.contains("thinking") {
                body["thinking"] = serde_json::json!({"type": "enabled"});
            } else {
                body["thinking"] = serde_json::json!({"type": "disabled"});
            }
        }

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
            // GLM/ZAI doesn't support parallel_tool_calls
            if self.provider != Provider::Zai {
                body["parallel_tool_calls"] = serde_json::json!(true);
            }
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let mut req = self
            .client
            .post(self.completions_url())
            .header("content-type", "application/json");
        if !api_key.is_empty() {
            if self.provider == Provider::XiaomiMimo {
                req = req.header("api-key", api_key);
            } else {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
        }
        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!("OpenAI API error {}: {}", status, text)));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_openai(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        // For Qwen OAuth: refresh token if near expiry before using it
        let refreshed_qwen_token: Option<String> = if self.provider == Provider::Qwen && matches!(self.auth, AuthMethod::OAuth(_)) {
            qwen_oauth::ensure_fresh_qwen_token(&self.client).await
        } else {
            None
        };
        let api_key = match (&self.auth, &refreshed_qwen_token) {
            (_, Some(fresh)) => fresh.clone(),
            (AuthMethod::ApiKey(key), _) => key.clone(),
            (AuthMethod::OAuth(token), _) => token.clone(),
            (AuthMethod::None, _) if self.provider == Provider::Ollama => String::new(),
            _ => return Err(Error::Llm("OPENAI_API_KEY not set".to_string())),
        };

        // Inject execution bias — all providers including Ollama
        let enhanced_system = if self.provider == Provider::Ollama {
            system.map(|s| format!("{}{}", s, Self::OLLAMA_EXECUTION_BIAS))
        } else {
            system.map(|s| format!("{}{}", s, Self::OPENAI_EXECUTION_BIAS))
        };
        let mut openai_messages = self.to_openai_messages(messages, enhanced_system.as_deref().or(system));
        let openai_tools = self.to_openai_tools(tools);

        // Smart context trimming (same as complete_openai)
        {
            let caps = capabilities::capabilities(&self.provider);
            let max_tokens = caps.context_window;
            let target = (max_tokens as f64 * 0.8) as usize;
            let estimated_tokens: usize = openai_messages.iter()
                .map(|m| m.to_string().len() / 4)
                .sum();
            if estimated_tokens > target && openai_messages.len() > 2 {
                let system_msg = openai_messages.first().cloned();
                let mut trimmed = Vec::new();
                if let Some(sys) = system_msg {
                    trimmed.push(sys);
                }
                let non_system: Vec<_> = openai_messages.drain(1..).collect();
                let mut running_tokens = trimmed.iter().map(|m| m.to_string().len() / 4).sum::<usize>();
                let truncated: Vec<_> = non_system.into_iter().map(|mut m| {
                    if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                        if content.len() > 50_000 {
                            m["content"] = serde_json::json!(format!(
                                "[Content truncated: {} chars → 500 char summary]\n{}",
                                content.len(), &content[..500.min(content.len())]
                            ));
                        }
                    }
                    m
                }).collect();
                let mut to_add: Vec<_> = truncated.into_iter().rev().take_while(|m| {
                    let msg_tokens = m.to_string().len() / 4;
                    running_tokens += msg_tokens;
                    running_tokens < target
                }).collect();
                to_add.reverse();
                let dropped = openai_messages.len().saturating_sub(1).saturating_sub(to_add.len());
                trimmed.extend(to_add);
                if dropped > 0 {
                    info!(
                        "Context trimmed (stream): {} messages dropped, {} → {} est tokens (limit: {})",
                        dropped, estimated_tokens, trimmed.iter().map(|m| m.to_string().len() / 4).sum::<usize>(), max_tokens
                    );
                }
                openai_messages = trimmed;
            }
        }

        let is_reasoning = is_reasoning_model(&self.model);
        let uses_new_api = is_reasoning || self.model.starts_with("gpt-5") || self.model.starts_with("gpt-4o");
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if uses_new_api {
            body["max_completion_tokens"] = serde_json::json!(4096);
        } else {
            body["max_tokens"] = serde_json::json!(4096);
        }
        self.inject_openai_sampling(&mut body, !tools.is_empty());

        // XiaomiMimo vision auto-route: override model to mimo-v2.5 when image attachments present
        if self.provider == Provider::XiaomiMimo && self.messages_have_images(messages) {
            body["model"] = serde_json::json!("mimo-v2.5");
            tracing::info!("XiaomiMimo vision auto-route: overriding model to mimo-v2.5 for image request (stream)");
        }

        // Moonshot/Kimi + XiaomiMimo thinking mode — disable for non-thinking models
        // to prevent reasoning_content session corruption.
        //
        // Bug-5: Skip thinking param entirely for XiaomiMimo image requests.
        // MiMo vision endpoint rejects unknown top-level `thinking` param with misleading 404.
        let skip_thinking = self.provider == Provider::XiaomiMimo && self.messages_have_images(messages);
        if !skip_thinking && (self.provider == Provider::Moonshot || self.provider == Provider::XiaomiMimo) {
            if self.model.contains("thinking") {
                body["thinking"] = serde_json::json!({"type": "enabled"});
            } else {
                body["thinking"] = serde_json::json!({"type": "disabled"});
            }
        }

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
            if self.provider != Provider::Zai {
                body["parallel_tool_calls"] = serde_json::json!(true);
            }
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let completions_url = self.completions_url();
        let provider = self.provider;

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let mut req = client
                .post(&completions_url)
                .header("content-type", "application/json");
            if !api_key.is_empty() {
                if provider == Provider::XiaomiMimo {
                    req = req.header("api-key", api_key);
                } else {
                    req = req.header("Authorization", format!("Bearer {}", api_key));
                }
            }
            let response = match req
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("OpenAI API error {}: {}", status, error_body);
                let err_msg = format!("OpenAI API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tcs {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(f) = tc.get("function") {
                                            if let Some(name) =
                                                f.get("name").and_then(|n| n.as_str())
                                            {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) =
                                                f.get("arguments").and_then(|a| a.as_str())
                                            {
                                                let current = tool_calls[idx]
                                                    .arguments
                                                    .as_str()
                                                    .unwrap_or("");
                                                let new_args = format!("{}{}", current, args);
                                                tool_calls[idx].arguments =
                                                    serde_json::Value::String(new_args);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                    // Read usage from final streaming chunk (separate parse — usage chunk has no choices)
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(usage) = parsed.get("usage")
                    {
                        input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        output_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    }
                }
            }

            // Parse accumulated tool call arguments and drop any with empty names
            // (can happen if a streaming delta never delivers function.name)
            let tool_calls: Vec<ToolCall> = tool_calls
                .into_iter()
                .filter_map(|mut tc| {
                    if tc.name.is_empty() {
                        warn!("Dropping OpenAI streaming tool call with empty name (id={})", tc.id);
                        return None;
                    }
                    if let serde_json::Value::String(s) = &tc.arguments {
                        tc.arguments = serde_json::from_str(s).unwrap_or(serde_json::json!({}));
                    }
                    Some(tc)
                })
                .collect();

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    /// Sanitize orphaned tool_use blocks in a message list.
    /// Position-aware: checks for matching results only until the next turn
    /// boundary (user/assistant message), not globally. This is critical for
    /// Kimi K2.6 which reuses tool_call_ids (e.g. "shell:0") across turns.
    /// Sanitize orphaned tool_use blocks. Position-aware: checks for matching
    /// results only until the next turn boundary.
    /// For Moonshot/Kimi and MiniMax: STRIPS orphaned tool_calls (these providers reject synthetic results).
    /// For other providers: INJECTS synthetic results (standard approach).
    fn sanitize_tool_pairs_for_provider(messages: &[Message], provider: &Provider) -> Vec<Message> {
        let strip_orphans = *provider == Provider::Moonshot || *provider == Provider::Minimax;

        // Phase 1: Collect all valid tool_call IDs from assistant messages
        let mut all_tool_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in messages {
            if msg.role == Role::Assistant {
                for tc in &msg.tool_calls {
                    if !tc.id.is_empty() {
                        all_tool_call_ids.insert(tc.id.clone());
                    }
                }
            }
        }

        // Phase 2: Process messages — fix orphaned tool_calls AND orphaned tool_results
        let mut result = Vec::with_capacity(messages.len() + 4);
        for (i, msg) in messages.iter().enumerate() {
            // Check: assistant message with tool_calls?
            if msg.role == Role::Assistant && !msg.tool_calls.is_empty() {
                // Look forward ONLY until the next user/assistant message (turn boundary)
                let mut found_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for subsequent in &messages[i + 1..] {
                    if subsequent.role == Role::User || subsequent.role == Role::Assistant {
                        break;
                    }
                    for tr in &subsequent.tool_results {
                        found_ids.insert(tr.call_id.as_str());
                    }
                }
                let orphan_ids: std::collections::HashSet<&str> = msg.tool_calls.iter()
                    .filter(|tc| !found_ids.contains(tc.id.as_str()))
                    .map(|tc| tc.id.as_str())
                    .collect();

                if !orphan_ids.is_empty() {
                    if strip_orphans {
                        // Kimi/MiniMax: STRIP orphaned tool_calls
                        let mut cleaned = msg.clone();
                        cleaned.tool_calls.retain(|tc| !orphan_ids.contains(tc.id.as_str()));
                        result.push(cleaned);
                        tracing::warn!(
                            "Stripped {} orphaned tool_call(s) at message {} (strip-orphans provider)",
                            orphan_ids.len(), i
                        );
                    } else {
                        // Other providers: inject synthetic results
                        result.push(msg.clone());
                        for tc in &msg.tool_calls {
                            if orphan_ids.contains(tc.id.as_str()) {
                                result.push(Message::tool(
                                    &tc.id,
                                    false,
                                    "[session interrupted — tool result unavailable]",
                                ));
                            }
                        }
                        tracing::warn!(
                            "Sanitized {} orphaned tool_use(s) at message {}",
                            orphan_ids.len(), i
                        );
                    }
                } else {
                    result.push(msg.clone());
                }
            } else if msg.role == Role::Tool && !msg.tool_results.is_empty() {
                // Phase 2b: Check for orphaned tool RESULTS — results referencing
                // tool_call_ids that don't exist in any assistant message.
                // This happens when strip-orphans removes tool_calls but leaves the results,
                // or when sessions accumulate stale results from interrupted cooks.
                let mut cleaned = msg.clone();
                let before_len = cleaned.tool_results.len();
                cleaned.tool_results.retain(|tr| {
                    !tr.call_id.is_empty() && all_tool_call_ids.contains(&tr.call_id)
                });
                let removed = before_len - cleaned.tool_results.len();
                if removed > 0 {
                    tracing::warn!(
                        "Stripped {} orphaned tool_result(s) at message {} (no matching tool_call)",
                        removed, i
                    );
                }
                // Only include the message if it still has results (or content)
                if !cleaned.tool_results.is_empty() || !cleaned.content.is_empty() {
                    result.push(cleaned);
                }
            } else {
                result.push(msg.clone());
            }
        }
        result
    }

    fn to_openai_messages(
        &self,
        messages: &[Message],
        system: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();

        // o1/o3/o4 reasoning models use "developer" role instead of "system"
        let system_role = if self.model.starts_with("o1")
            || self.model.starts_with("o3")
            || self.model.starts_with("o4")
        {
            "developer"
        } else {
            "system"
        };

        if let Some(sys) = system {
            result.push(serde_json::json!({
                "role": system_role,
                "content": sys
            }));
        }

        for msg in messages {
            match msg.role {
                Role::System => {
                    result.push(serde_json::json!({
                        "role": system_role,
                        "content": msg.content
                    }));
                }
                Role::User => {
                    if msg.attachments.is_empty() || !self.should_include_images() {
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": msg.content
                        }));
                    } else {
                        // Multi-content block: images + text
                        let mut content_parts = Vec::new();
                        for att in &msg.attachments {
                            if let Some(block) = crate::multimodal::format_openai_attachment(att) {
                                content_parts.push(block);
                            }
                        }
                        if !msg.content.is_empty() {
                            content_parts.push(serde_json::json!({
                                "type": "text",
                                "text": msg.content
                            }));
                        }
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": content_parts
                        }));
                    }
                }
                Role::Assistant => {
                    // Guard: some providers (Kimi/Moonshot, MiniMax) reject empty
                    // assistant content. Use a space placeholder when empty so the
                    // message is valid when only tool_calls are present.
                    // Note: null is rejected by Kimi/Moonshot as "empty".
                    let content_val = if msg.content.is_empty() {
                        serde_json::Value::String(" ".to_string())
                    } else {
                        serde_json::Value::String(msg.content.clone())
                    };
                    let mut m = serde_json::json!({
                        "role": "assistant",
                        "content": content_val
                    });
                    if !msg.tool_calls.is_empty() {
                        let tcs: Vec<_> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments.to_string()
                                    }
                                })
                            })
                            .collect();
                        m["tool_calls"] = serde_json::Value::Array(tcs);
                    }
                    result.push(m);
                }
                Role::Tool => {
                    for tr in &msg.tool_results {
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tr.call_id,
                            "content": tr.output
                        }));
                    }
                }
            }
        }

        result
    }

    fn to_openai_tools(&self, tools: &[ToolSchema]) -> serde_json::Value {
        let tools: Vec<_> = tools
            .iter()
            .filter(|t| !t.name.is_empty()) // Guard: never send empty function.name to OpenAI
            .map(|t| {
                // Defensive: ensure description is non-empty (matches OpenClaw)
                let desc = if t.description.is_empty() { &t.name } else { &t.description };
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": desc,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();
        serde_json::Value::Array(tools)
    }

    fn parse_openai_response(&self, text: &str) -> Result<LlmResponse> {
        let response: serde_json::Value = serde_json::from_str(text)?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        if let Some(choices) = response.get("choices").and_then(|c| c.as_array())
            && let Some(choice) = choices.first()
        {
            if let Some(message) = choice.get("message") {
                if let Some(c) = message.get("content").and_then(|c| c.as_str()) {
                    content = c.to_string();
                }
                if let Some(tcs) = message.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tcs {
                        let id = tc
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        // Skip tool calls with empty names — they cause OpenAI API errors
                        if name.is_empty() {
                            warn!("Dropping OpenAI tool call with empty function name (id={})", id);
                            continue;
                        }
                        let args_str = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        let stripped = zeus_core::strip_json_markdown(args_str);
                        let (arguments, healed) =
                            crate::json_healing::heal_json_or_empty(&stripped);
                        if healed {
                            warn!(
                                "json_healing: repaired malformed tool-call arguments for tool '{}' (id={}, raw_len={})",
                                name, id, args_str.len()
                            );
                        }
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                stop_reason = match reason {
                    "tool_calls" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                };
            }
        }

        // Extract token usage from OpenAI response
        let usage = response.get("usage");
        let input_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let output_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(LlmResponse {
            content,
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
            cached_tokens: 0,
        })
    }

    // ========================================================================
    // Ollama — routed through OpenAI-compat (complete_openai/stream_openai)
    // Native /api/chat path removed in S22 dead code cleanup.
    // ========================================================================

    /// Execution bias for Gemini — same philosophy as OPENAI_EXECUTION_BIAS.
    /// Encourages the model to use tools proactively rather than describing
    /// what it would do.
    const GEMINI_EXECUTION_BIAS: &'static str = "\n\n## Execution Bias\n\nStart the real work in the same turn when the next step is clear.\nDo prerequisite lookup or discovery before dependent actions.\nIf another tool call would likely improve correctness or completeness, keep going instead of stopping at partial progress.\nMulti-part requests stay incomplete until every requested item is handled or clearly marked blocked.\nKeep responses concise. Prefer tool actions over explanations.";

    // complete_ollama, stream_ollama, to_ollama_messages removed (S22).
    // Ollama now uses complete_openai/stream_openai via OpenAI-compat endpoint.

    // ========================================================================
    // OpenRouter Implementation (same as OpenAI)
    // ========================================================================

    async fn complete_openrouter(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("OPENROUTER_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        // #44-b-ii-B: Consult OpenRouterResolver for dynamic model capabilities.
        // If the resolver returns caps with thinking_mode_temperature_lock=Some(true)
        // (e.g. MiMo family `xiaomi/mimo*`), pin temperature=1.0 per spec.
        let resolver = OpenRouterResolver::new(
            self.client.clone(),
            self.base_url.clone(),
            Some(api_key.clone()),
        );
        if let Some(caps) = <OpenRouterResolver as crate::capabilities::ModelCapabilityResolver>::resolve(&resolver, &self.model).await {
            if caps.thinking_mode_temperature_lock == Some(true) {
                body["temperature"] = serde_json::json!(1.0);
            }
        }

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "OpenRouter API error {}: {}",
                status, text
            )));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_openrouter(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("OPENROUTER_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
            "temperature": 0.3,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        // #44-b-ii-B: Consult OpenRouterResolver for dynamic model capabilities.
        // Mirrors complete_openrouter symmetry — pin temperature=1.0 for
        // thinking_mode_temperature_lock=Some(true) models (e.g. MiMo family).
        let resolver = OpenRouterResolver::new(
            self.client.clone(),
            self.base_url.clone(),
            Some(api_key.clone()),
        );
        if let Some(caps) = <OpenRouterResolver as crate::capabilities::ModelCapabilityResolver>::resolve(&resolver, &self.model).await {
            if caps.thinking_mode_temperature_lock == Some(true) {
                body["temperature"] = serde_json::json!(1.0);
            }
        }

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(format!("{}/v1/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("OpenRouter API error {}: {}", status, error_body);
                let err_msg = format!("OpenRouter API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                // Content delta
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                // Tool call deltas (OpenAI-compatible format)
                                if let Some(tc_arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                    for tc in tc_arr {
                                        let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                        // Ensure tool_calls vec is large enough
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(func) = tc.get("function") {
                                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                // Accumulate argument chunks
                                                let existing = tool_calls[idx].arguments.as_str().unwrap_or("").to_string();
                                                let combined = existing + args;
                                                tool_calls[idx].arguments = serde_json::Value::String(combined);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                    // Read usage from streaming chunk
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(usage) = parsed.get("usage")
                    {
                        input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        output_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(usage) = parsed.get("usage")
                    {
                        input_tokens = parsed.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        output_tokens = parsed.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    }
                }
            }

            // Parse accumulated tool call argument strings into JSON
            for tc in &mut tool_calls {
                if let serde_json::Value::String(ref s) = tc.arguments {
                    tc.arguments = serde_json::from_str(s)
                        .unwrap_or_else(|_| serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Google Gemini Implementation
    // ========================================================================

    /// Resolve Google model aliases — redirect old/common names to current equivalents.
    fn resolve_google_model_alias(model: &str) -> &str {
        match model {
            "gemini-pro" => "gemini-1.5-pro",
            "gemini-pro-vision" => "gemini-1.5-pro",
            "gemini-ultra" => "gemini-1.5-pro",
            "gemini-nano" => "gemini-2.0-flash",
            _ => model,
        }
    }

    async fn complete_google(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        // Support both API key and OAuth Bearer auth for Gemini
        let api_key: Option<String> = match &self.auth {
            AuthMethod::ApiKey(key) => Some(key.clone()),
            AuthMethod::OAuth(_) => None, // Will use Bearer auth via google_bearer_token()
            _ => return Err(Error::Llm("GOOGLE_API_KEY or Google OAuth not set".to_string())),
        };

        // Inject execution bias when tools are provided (parity with OpenAI)
        let enhanced_system = if !tools.is_empty() {
            system.map(|s| format!("{}{}", s, Self::GEMINI_EXECUTION_BIAS))
        } else {
            None
        };
        let effective_system = enhanced_system.as_deref().or(system);

        let contents = self.to_gemini_contents(messages);
        let mut body = serde_json::json!({ "contents": contents });

        if let Some(sys) = effective_system {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": sys}]
            });
        }

        if !tools.is_empty() {
            body["tools"] = self.to_gemini_tools(tools);
            // Gemini tool_config — equivalent of OpenAI/Anthropic tool_choice
            body["tool_config"] = serde_json::json!({
                "function_calling_config": {
                    "mode": "AUTO"
                }
            });
        }

        // Generation config — parity with Anthropic/OpenAI
        body["generationConfig"] = serde_json::json!({
            "temperature": 0.3,
            "maxOutputTokens": 4096,
            "topP": 0.95,
        });

        // Thinking/reasoning support for Gemini 2.5+ models — parity with
        // Anthropic's extended thinking (thinking_level + budget_tokens).
        // Gemini uses `thinkingConfig.thinkingBudget` in the generationConfig.
        if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            if let Some(gen_config) = body.get_mut("generationConfig") {
                gen_config["thinkingConfig"] = serde_json::json!({
                    "thinkingBudget": budget
                });
            }
        }

        let url = if let Some(ref key) = api_key {
            format!("{}/v1beta/models/{}:generateContent?key={}", self.base_url, self.model, key)
        } else {
            format!("{}/v1beta/models/{}:generateContent", self.base_url, self.model)
        };

        debug!("Google Gemini complete: model={}", self.model);

        // Retry loop for transient errors (502, 503, 429 rate limit)
        // Same pattern as Ollama — 3 attempts with exponential backoff.
        let max_retries = 2u32;
        let mut last_error = String::new();

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let backoff = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "Gemini retry {}/{} after {} error, waiting {:?}",
                    attempt, max_retries, last_error, backoff
                );
                tokio::time::sleep(backoff).await;
            }

            let mut req = self
                .client
                .post(&url)
                .header("content-type", "application/json");
            // Add Bearer auth for OAuth mode — fail if token unavailable
            if api_key.is_none() {
                let token = self.google_bearer_token().await
                    .map_err(|e| Error::Llm(format!("Gemini OAuth token failed: {}. Use an API key instead.", e)))?;
                req = req.header("Authorization", format!("Bearer {}", token));
            }
            let response = match req
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < max_retries && (e.is_timeout() || e.is_connect()) {
                        continue;
                    }
                    return Err(Error::Llm(last_error));
                }
            };

            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|e| Error::Llm(e.to_string()))?;

            if status.is_success() {
                return self.parse_gemini_response(&text);
            }

            last_error = format!("{}", status.as_u16());
            if attempt < max_retries && is_retryable_status(status.as_u16()) {
                continue;
            }
            return Err(Error::Llm(format!("Gemini API error {}: {}", status, text)));
        }

        Err(Error::Llm(format!("Gemini: all retries exhausted (last error: {})", last_error)))
    }

    async fn stream_google(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key: Option<String> = match &self.auth {
            AuthMethod::ApiKey(key) => Some(key.clone()),
            AuthMethod::OAuth(_) => None,
            _ => return Err(Error::Llm("GOOGLE_API_KEY or Google OAuth not set".to_string())),
        };

        // Inject execution bias when tools are provided (parity with OpenAI)
        let enhanced_system = if !tools.is_empty() {
            system.map(|s| format!("{}{}", s, Self::GEMINI_EXECUTION_BIAS))
        } else {
            None
        };
        let effective_system = enhanced_system.as_deref().or(system);

        let contents = self.to_gemini_contents(messages);
        let mut body = serde_json::json!({ "contents": contents });

        if let Some(sys) = effective_system {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": sys}]
            });
        }

        if !tools.is_empty() {
            body["tools"] = self.to_gemini_tools(tools);
            // Gemini tool_config — equivalent of OpenAI/Anthropic tool_choice
            body["tool_config"] = serde_json::json!({
                "function_calling_config": {
                    "mode": "AUTO"
                }
            });
        }

        // Generation config — parity with Anthropic/OpenAI (same as complete_google)
        body["generationConfig"] = serde_json::json!({
            "temperature": 0.3,
            "maxOutputTokens": 4096,
            "topP": 0.95,
        });

        // Thinking/reasoning for Gemini 2.5+ (same as complete_google)
        if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            if let Some(gen_config) = body.get_mut("generationConfig") {
                gen_config["thinkingConfig"] = serde_json::json!({
                    "thinkingBudget": budget
                });
            }
        }

        let url = if let Some(ref key) = api_key {
            format!("{}/v1beta/models/{}:streamGenerateContent?key={}&alt=sse", self.base_url, self.model, key)
        } else {
            format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", self.base_url, self.model)
        };

        // Get Bearer token for OAuth mode before spawning (can't call &self in spawn)
        let bearer_token = if api_key.is_none() {
            self.google_bearer_token().await.ok()
        } else {
            None
        };

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            // Retry loop for transient errors — same pattern as complete_google
            // and Ollama's stream_ollama (3 attempts, 2^n backoff).
            let max_retries = 2u32;
            let mut response_opt = None;

            for attempt in 0..=max_retries {
                if attempt > 0 {
                    let backoff = std::time::Duration::from_secs(2u64.pow(attempt));
                    tracing::warn!(
                        "Gemini stream retry {}/{} waiting {:?}",
                        attempt, max_retries, backoff
                    );
                    tokio::time::sleep(backoff).await;
                }

                let mut req = client
                    .post(&url)
                    .header("content-type", "application/json");
                if let Some(ref token) = bearer_token {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
                match req
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => {
                        if r.status().is_success() {
                            response_opt = Some(r);
                            break;
                        }
                        let status = r.status();
                        let error_body = r
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error body".to_string());
                        if attempt < max_retries && is_retryable_status(status.as_u16()) {
                            tracing::warn!("Gemini stream got {} (retryable), retrying", status);
                            continue;
                        }
                        error!("Google API error {}: {}", status, error_body);
                        let err_msg = format!("Google API error ({}): {}", status, error_body);
                        let _ = tx.send(format!("[{}]", err_msg)).await;
                        return LlmResponse {
                            content: err_msg,
                            tool_calls: vec![],
                            stop_reason: StopReason::Error,
                            input_tokens: 0,
                            output_tokens: 0,
                            cached_tokens: 0,
                        };
                    }
                    Err(e) => {
                        if attempt < max_retries && (e.is_timeout() || e.is_connect()) {
                            tracing::warn!("Gemini stream request failed (retryable): {}", e);
                            continue;
                        }
                        error!("Gemini stream request failed: {}", e);
                        return LlmResponse {
                            content: format!("Gemini stream request failed: {}", e),
                            tool_calls: vec![],
                            stop_reason: StopReason::Error,
                            input_tokens: 0,
                            output_tokens: 0,
                            cached_tokens: 0,
                        };
                    }
                }
            }

            let response = match response_opt {
                Some(r) => r,
                None => {
                    error!("Gemini stream: all retries exhausted");
                    return LlmResponse {
                        content: "Gemini stream: all retries exhausted".to_string(),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE lines
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(candidates) = event.get("candidates").and_then(|c| c.as_array())
                    {
                        for candidate in candidates {
                            if let Some(parts) = candidate
                                .get("content")
                                .and_then(|c| c.get("parts"))
                                .and_then(|p| p.as_array())
                            {
                                for part in parts {
                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                        content.push_str(text);
                                        let _ = tx.send(text.to_string()).await;
                                    }
                                    if let Some(fc) = part.get("functionCall") {
                                        let name = fc
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let args = fc
                                            .get("args")
                                            .cloned()
                                            .unwrap_or(serde_json::json!({}));
                                        tool_calls.push(ToolCall {
                                            id: format!("call_{}", tool_calls.len()),
                                            name,
                                            arguments: args,
                                        });
                                    }
                                }
                            }
                            if let Some(reason) =
                                candidate.get("finishReason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "MAX_TOKENS" => StopReason::MaxTokens,
                                    "TOOL_USE" => StopReason::ToolUse,
                                    "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => StopReason::Error,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                    // Read usage from Gemini streaming response
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(usage) = parsed.get("usageMetadata")
                    {
                        input_tokens = usage.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        output_tokens = usage.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    }
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    fn to_gemini_contents(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        let mut contents: Vec<serde_json::Value> = Vec::new();

        for msg in messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "model",
                Role::System => continue, // Handled via systemInstruction
                Role::Tool => "user",     // Tool results sent as user messages
            };

            // Handle tool results — look up function name from preceding model message
            if msg.role == Role::Tool && !msg.tool_results.is_empty() {
                // Collect function names from the last model message's functionCall parts
                let mut prev_names: Vec<String> = Vec::new();
                for c in contents.iter().rev() {
                    if let Some(parts_arr) = c.get("parts").and_then(|p| p.as_array()) {
                        for p in parts_arr {
                            if let Some(name) = p.get("functionCall")
                                .and_then(|fc| fc.get("name"))
                                .and_then(|n| n.as_str())
                            {
                                prev_names.push(name.to_string());
                            }
                        }
                        if !prev_names.is_empty() { break; }
                    }
                }
                let mut parts = Vec::new();
                for (i, tr) in msg.tool_results.iter().enumerate() {
                    // Gemini requires functionResponse.name to match functionCall.name
                    let func_name = prev_names.get(i)
                        .cloned()
                        .unwrap_or_else(|| tr.call_id.clone());
                    parts.push(serde_json::json!({
                        "functionResponse": {
                            "name": func_name,
                            "response": {
                                "content": tr.output
                            }
                        }
                    }));
                }
                contents.push(serde_json::json!({ "role": role, "parts": parts }));
                continue;
            }

            // Handle tool calls from assistant
            if msg.role == Role::Assistant && !msg.tool_calls.is_empty() {
                let mut parts = Vec::new();
                if !msg.content.is_empty() {
                    parts.push(serde_json::json!({"text": msg.content}));
                }
                for tc in &msg.tool_calls {
                    parts.push(serde_json::json!({
                        "functionCall": {
                            "name": tc.name,
                            "args": tc.arguments
                        }
                    }));
                }
                contents.push(serde_json::json!({ "role": role, "parts": parts }));
                continue;
            }

            // Handle attachments (images, documents)
            if !msg.attachments.is_empty() && self.should_include_images() {
                let mut parts = Vec::new();
                for att in &msg.attachments {
                    if let Some(block) = crate::multimodal::format_gemini_attachment(att) {
                        parts.push(block);
                    }
                }
                parts.push(serde_json::json!({"text": msg.content}));
                contents.push(serde_json::json!({ "role": role, "parts": parts }));
            } else {
                contents.push(serde_json::json!({
                    "role": role,
                    "parts": [{"text": msg.content}]
                }));
            }
        }

        contents
    }

    fn to_gemini_tools(&self, tools: &[ToolSchema]) -> serde_json::Value {
        let declarations: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();

        serde_json::json!([{
            "functionDeclarations": declarations
        }])
    }

    /// Extract a human-readable message from a Gemini API error body (JSON or plain text).
    fn format_gemini_error(status: u16, body: &str) -> String {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
            // Standard Google API error envelope: {"error": {"message": "...", "status": "..."}}
            if let Some(msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
                let status_str = v.pointer("/error/status").and_then(|s| s.as_str()).unwrap_or("");
                return if status == 429 {
                    format!("Gemini rate limited ({}): {}  Try again in a few seconds.", status_str, msg)
                } else {
                    format!("Gemini error {} ({}): {}", status, status_str, msg)
                };
            }
        }
        // Fallback: raw body (truncated to avoid filling the chat window)
        let truncated = if body.len() > 200 { &body[..200] } else { body };
        format!("Gemini API error {}: {}", status, truncated)
    }

    fn parse_gemini_response(&self, text: &str) -> Result<LlmResponse> {
        let json: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::Llm(format!("Failed to parse Gemini response: {}", e)))?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
            for candidate in candidates {
                if let Some(parts) = candidate
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                            content.push_str(t);
                        }
                        if let Some(fc) = part.get("functionCall") {
                            let name = fc
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                            tool_calls.push(ToolCall {
                                id: format!("call_{}", tool_calls.len()),
                                name,
                                arguments: args,
                            });
                        }
                    }
                }
                if let Some(reason) = candidate.get("finishReason").and_then(|r| r.as_str()) {
                    stop_reason = match reason {
                        "MAX_TOKENS" => StopReason::MaxTokens,
                        "TOOL_USE" => StopReason::ToolUse,
                        "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => StopReason::Error,
                        _ => StopReason::EndTurn,
                    };
                }
            }
        }

        if !tool_calls.is_empty() {
            stop_reason = StopReason::ToolUse;
        }

        // Parse usage from Gemini response
        let input_tokens = json.get("usageMetadata")
            .and_then(|u| u.get("promptTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let output_tokens = json.get("usageMetadata")
            .and_then(|u| u.get("candidatesTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(LlmResponse {
            content,
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
            cached_tokens: 0,
        })
    }


    // ========================================================================
    // Google Gemini CLI (cloudcode-pa.googleapis.com) Implementation
    // ========================================================================

    /// Discover the GCP project ID for Gemini CLI via loadCodeAssist.
    /// Caches after first successful call.
    async fn discover_gemini_cli_project(&self, token: &str) -> Result<String> {
        // Check cache first
        static PROJECT_CACHE: std::sync::LazyLock<std::sync::Mutex<Option<String>>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

        if let Ok(guard) = PROJECT_CACHE.lock() {
            if let Some(ref cached) = *guard {
                return Ok(cached.clone());
            }
        }

        // Call loadCodeAssist to discover project
        let resp = self.client
            .post(format!("{}/v1internal:loadCodeAssist", self.base_url))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "metadata": {
                    "ideType": "ANTIGRAVITY",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI",
                }
            }))
            .send()
            .await
            .map_err(|e| Error::Llm(format!("Gemini CLI project discovery failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!("Gemini CLI loadCodeAssist {} — {}", status, text)));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| Error::Llm(format!("Gemini CLI project parse: {}", e)))?;
        let project = body["cloudaicompanionProject"].as_str()
            .ok_or_else(|| Error::Llm("Gemini CLI: no cloudaicompanionProject in response".into()))?
            .to_string();

        info!("Gemini CLI project discovered: {}", project);
        if let Ok(mut guard) = PROJECT_CACHE.lock() {
            *guard = Some(project.clone());
        }
        Ok(project)
    }

    async fn google_gemini_cli_bearer_token(&self) -> Result<String> {
        match &self.auth {
            AuthMethod::OAuth(token) => {
                if let Ok(Some(cred)) = OAuthManager::get_credential("google-gemini-cli")
                    && Utc::now() + ChronoDuration::minutes(5) >= cred.expires_at
                    && !cred.refresh_token.is_empty()
                    && let Some(tokens) = OAuthManager::refresh_token("google-gemini-cli").await?
                {
                    return Ok(tokens.access_token);
                }
                Ok(token.clone())
            }
            _ => Err(Error::Llm("Google Gemini CLI OAuth not configured".to_string())),
        }
    }

    async fn complete_google_gemini_cli(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let token = self.google_gemini_cli_bearer_token().await?;
        let enhanced_system = if !tools.is_empty() {
            system.map(|s| format!("{}{}", s, Self::GEMINI_EXECUTION_BIAS))
        } else { None };
        let effective_system = enhanced_system.as_deref().or(system);
        let contents = self.to_gemini_contents(messages);
        let mut body = serde_json::json!({ "contents": contents });
        if let Some(sys) = effective_system {
            body["systemInstruction"] = serde_json::json!({ "parts": [{"text": sys}] });
        }
        if !tools.is_empty() {
            body["tools"] = self.to_gemini_tools(tools);
            body["tool_config"] = serde_json::json!({ "function_calling_config": { "mode": "AUTO" } });
        }
        body["generationConfig"] = serde_json::json!({ "temperature": 0.3, "maxOutputTokens": 4096, "topP": 0.95 });
        if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            if let Some(gc) = body.get_mut("generationConfig") {
                gc["thinkingConfig"] = serde_json::json!({ "thinkingBudget": budget });
            }
        }
        // Code Assist API: wrap request in envelope with project + model
        let project_id = self.discover_gemini_cli_project(&token).await?;
        let envelope = serde_json::json!({
            "model": self.model,
            "project": project_id,
            "user_prompt_id": uuid::Uuid::new_v4().to_string(),
            "request": body,
        });
        let url = format!("{}/v1internal:generateContent", self.base_url);
        debug!("Google Gemini CLI complete: model={}, project={}", self.model, project_id);
        let max_retries = 2u32;
        let mut last_error = String::new();
        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            let response = match self.client.post(&url)
                .header("content-type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .json(&envelope).send().await
            {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < max_retries && (e.is_timeout() || e.is_connect()) { continue; }
                    return Err(Error::Llm(last_error));
                }
            };
            let status = response.status();
            let text = response.text().await.map_err(|e| Error::Llm(e.to_string()))?;
            if status.is_success() {
                // Code Assist wraps response in {"response": {...}} envelope — unwrap it
                let unwrapped = match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(v) if v.get("response").is_some() => {
                        serde_json::to_string(&v["response"]).unwrap_or(text.clone())
                    }
                    _ => text,
                };
                return self.parse_gemini_response(&unwrapped);
            }
            last_error = format!("{}", status.as_u16());
            if attempt < max_retries && is_retryable_status(status.as_u16()) { continue; }
            return Err(Error::Llm(Self::format_gemini_error(status.as_u16(), &text)));
        }
        Err(Error::Llm(format!("Gemini CLI: all retries exhausted (last: {})", last_error)))
    }


    async fn stream_google_gemini_cli(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let token = self.google_gemini_cli_bearer_token().await?;
        let enhanced_system = if !tools.is_empty() {
            system.map(|s| format!("{}{}", s, Self::GEMINI_EXECUTION_BIAS))
        } else { None };
        let effective_system = enhanced_system.as_deref().or(system);
        let contents = self.to_gemini_contents(messages);
        let mut body = serde_json::json!({ "contents": contents });
        if let Some(sys) = effective_system {
            body["systemInstruction"] = serde_json::json!({ "parts": [{"text": sys}] });
        }
        if !tools.is_empty() {
            body["tools"] = self.to_gemini_tools(tools);
            body["tool_config"] = serde_json::json!({ "function_calling_config": { "mode": "AUTO" } });
        }
        body["generationConfig"] = serde_json::json!({ "temperature": 0.3, "maxOutputTokens": 4096, "topP": 0.95 });
        if let Some(ref level) = self.thinking_level {
            let budget = Self::thinking_budget(level);
            if let Some(gc) = body.get_mut("generationConfig") {
                gc["thinkingConfig"] = serde_json::json!({ "thinkingBudget": budget });
            }
        }
        // Code Assist API: wrap request in envelope with project + model
        let project_id = self.discover_gemini_cli_project(&token).await?;
        let envelope = serde_json::json!({
            "model": self.model,
            "project": project_id,
            "user_prompt_id": uuid::Uuid::new_v4().to_string(),
            "request": body,
        });
        let url = format!("{}/v1internal:streamGenerateContent?alt=sse", self.base_url);
        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;
            let max_retries = 2u32;
            let mut response_opt = None;
            for attempt in 0..=max_retries {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                }
                match client.post(&url)
                    .header("content-type", "application/json")
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&envelope).send().await
                {
                    Ok(r) if r.status().is_success() => { response_opt = Some(r); break; }
                    Ok(r) => {
                        let st = r.status();
                        let eb = r.text().await.unwrap_or_default();
                        if attempt < max_retries && is_retryable_status(st.as_u16()) { continue; }
                        let msg = Self::format_gemini_error(st.as_u16(), &eb);
                        let _ = tx.send(format!("[{}]", msg)).await;
                        return LlmResponse { content: msg, tool_calls: vec![], stop_reason: StopReason::Error, input_tokens: 0, output_tokens: 0, cached_tokens: 0 };
                    }
                    Err(e) => {
                        if attempt < max_retries && (e.is_timeout() || e.is_connect()) { continue; }
                        return LlmResponse { content: format!("Gemini CLI stream failed: {}", e), tool_calls: vec![], stop_reason: StopReason::Error, input_tokens: 0, output_tokens: 0, cached_tokens: 0 };
                    }
                }
            }
            let response = match response_opt {
                Some(r) => r,
                None => return LlmResponse { content: "Gemini CLI: all retries exhausted".into(), tool_calls: vec![], stop_reason: StopReason::Error, input_tokens: 0, output_tokens: 0, cached_tokens: 0 },
            };
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        let mut last_newline = 0;
                        for (i, c) in buffer.char_indices() {
                            if c == '\n' { last_newline = i + 1; }
                        }
                        let to_process = buffer[..last_newline].to_string();
                        buffer = buffer[last_newline..].to_string();
                        for line in to_process.lines() {
                            let line = line.trim();
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" { break; }
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(cands) = val["candidates"].as_array() {
                                        for c in cands {
                                            if let Some(parts) = c["content"]["parts"].as_array() {
                                                for part in parts {
                                                    if let Some(text) = part["text"].as_str() {
                                                        content.push_str(text);
                                                        let _ = tx.send(text.to_string()).await;
                                                    }
                                                    if let Some(fc) = part.get("functionCall") {
                                                        let name = fc["name"].as_str().unwrap_or("").to_string();
                                                        let args = fc["args"].clone();
                                                        tool_calls.push(ToolCall { id: format!("call_{}", name), name, arguments: args });
                                                        stop_reason = StopReason::ToolUse;
                                                    }
                                                }
                                            }
                                            match c["finishReason"].as_str() {
                                                Some("STOP") => { stop_reason = StopReason::EndTurn; }
                                                Some("MAX_TOKENS") => { stop_reason = StopReason::MaxTokens; }
                                                _ => {}
                                            }
                                        }
                                    }
                                    if let Some(meta) = val.get("usageMetadata") {
                                        input_tokens = meta["promptTokenCount"].as_u64().unwrap_or(0) as usize;
                                        output_tokens = meta["candidatesTokenCount"].as_u64().unwrap_or(0) as usize;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => { error!("Gemini CLI stream chunk error: {}", e); break; }
                }
            }
            LlmResponse { content, tool_calls, stop_reason, input_tokens, output_tokens, cached_tokens: 0 }
        });
        Ok((rx, handle))
    }


    // ========================================================================
    // Groq Implementation (OpenAI-compatible)
    // ========================================================================

    async fn complete_groq(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("GROQ_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        debug!("Groq complete: model={}", self.model);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!("Groq API error {}: {}", status, text)));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_groq(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("GROQ_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(format!("{}/v1/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Groq stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Groq stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Groq API error {}: {}", status, error_body);
                let err_msg = format!("Groq API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                // Tool call deltas
                                if let Some(tc_arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                    for tc in tc_arr {
                                        let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(func) = tc.get("function") {
                                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                let existing = tool_calls[idx].arguments.as_str().unwrap_or("").to_string();
                                                let combined = existing + args;
                                                tool_calls[idx].arguments = serde_json::Value::String(combined);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(usage) = parsed.get("usage")
                    {
                        input_tokens = parsed.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        output_tokens = parsed.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    }
                }
            }

            // Parse accumulated tool call argument strings into JSON
            for tc in &mut tool_calls {
                if let serde_json::Value::String(ref s) = tc.arguments {
                    tc.arguments = serde_json::from_str(s)
                        .unwrap_or_else(|_| serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Mistral Implementation (OpenAI-compatible)
    // ========================================================================

    async fn complete_mistral(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("MISTRAL_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        debug!("Mistral complete: model={}", self.model);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Mistral API error {}: {}",
                status, text
            )));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_mistral(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("MISTRAL_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(format!("{}/v1/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Mistral stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Mistral stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Mistral API error {}: {}", status, error_body);
                let err_msg = format!("Mistral API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tcs {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(f) = tc.get("function") {
                                            if let Some(name) =
                                                f.get("name").and_then(|n| n.as_str())
                                            {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) =
                                                f.get("arguments").and_then(|a| a.as_str())
                                            {
                                                let current = tool_calls[idx]
                                                    .arguments
                                                    .as_str()
                                                    .unwrap_or("");
                                                let new_args = format!("{}{}", current, args);
                                                tool_calls[idx].arguments =
                                                    serde_json::Value::String(new_args);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                }
            }

            // Parse accumulated tool call arguments
            for tc in &mut tool_calls {
                if let serde_json::Value::String(s) = &tc.arguments {
                    tc.arguments = serde_json::from_str(s).unwrap_or(serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Together AI Implementation (OpenAI-compatible)
    // ========================================================================

    async fn complete_together(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("TOGETHER_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        debug!("Together complete: model={}", self.model);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Together API error {}: {}",
                status, text
            )));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_together(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("TOGETHER_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(format!("{}/v1/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Together stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Together stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Together API error {}: {}", status, error_body);
                let err_msg = format!("Together API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tcs {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(f) = tc.get("function") {
                                            if let Some(name) =
                                                f.get("name").and_then(|n| n.as_str())
                                            {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) =
                                                f.get("arguments").and_then(|a| a.as_str())
                                            {
                                                let current = tool_calls[idx]
                                                    .arguments
                                                    .as_str()
                                                    .unwrap_or("");
                                                let new_args = format!("{}{}", current, args);
                                                tool_calls[idx].arguments =
                                                    serde_json::Value::String(new_args);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                }
            }

            // Parse accumulated tool call arguments
            for tc in &mut tool_calls {
                if let serde_json::Value::String(s) = &tc.arguments {
                    tc.arguments = serde_json::from_str(s).unwrap_or(serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Fireworks AI Implementation (OpenAI-compatible)
    // ========================================================================

    async fn complete_fireworks(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("FIREWORKS_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        debug!("Fireworks complete: model={}", self.model);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Fireworks API error {}: {}",
                status, text
            )));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_fireworks(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("FIREWORKS_API_KEY not set".to_string())),
        };

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(format!("{}/v1/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Fireworks stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Fireworks stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Fireworks API error {}: {}", status, error_body);
                let err_msg = format!("Fireworks API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tcs {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(f) = tc.get("function") {
                                            if let Some(name) =
                                                f.get("name").and_then(|n| n.as_str())
                                            {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) =
                                                f.get("arguments").and_then(|a| a.as_str())
                                            {
                                                let current = tool_calls[idx]
                                                    .arguments
                                                    .as_str()
                                                    .unwrap_or("");
                                                let new_args = format!("{}{}", current, args);
                                                tool_calls[idx].arguments =
                                                    serde_json::Value::String(new_args);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                }
            }

            // Parse accumulated tool call arguments
            for tc in &mut tool_calls {
                if let serde_json::Value::String(s) = &tc.arguments {
                    tc.arguments = serde_json::from_str(s).unwrap_or(serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Azure OpenAI Implementation
    // ========================================================================

    async fn complete_azure(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key,
            _ => return Err(Error::Llm("AZURE_OPENAI_API_KEY not set".to_string())),
        };

        let deployment = env::var("AZURE_OPENAI_DEPLOYMENT").unwrap_or_else(|_| self.model.clone());

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version=2024-10-21",
            self.base_url, deployment
        );

        debug!("Azure OpenAI complete: deployment={}", deployment);

        let response = self
            .client
            .post(&url)
            .header("api-key", api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Azure OpenAI API error {}: {}",
                status, text
            )));
        }

        self.parse_openai_response(&text)
    }

    async fn stream_azure(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let api_key = match &self.auth {
            AuthMethod::ApiKey(key) => key.clone(),
            _ => return Err(Error::Llm("AZURE_OPENAI_API_KEY not set".to_string())),
        };

        let deployment = env::var("AZURE_OPENAI_DEPLOYMENT").unwrap_or_else(|_| self.model.clone());

        let openai_messages = self.to_openai_messages(messages, system);
        let openai_tools = self.to_openai_tools(tools);

        let mut body = serde_json::json!({
            "messages": openai_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }
        self.inject_response_format(&mut body);
        self.inject_seed(&mut body);

        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version=2024-10-21",
            self.base_url, deployment
        );

        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            let response = match client
                .post(&url)
                .header("api-key", &api_key)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Azure OpenAI stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Azure OpenAI stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before starting stream
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Azure API error {}: {}", status, error_body);
                let err_msg = format!("Azure API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                    {
                        for choice in choices {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                    content.push_str(c);
                                    let _ = tx.send(c.to_string()).await;
                                }
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tcs {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(ToolCall {
                                                id: String::new(),
                                                name: String::new(),
                                                arguments: serde_json::Value::String(String::new()),
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            tool_calls[idx].id = id.to_string();
                                        }
                                        if let Some(f) = tc.get("function") {
                                            if let Some(name) =
                                                f.get("name").and_then(|n| n.as_str())
                                            {
                                                tool_calls[idx].name = name.to_string();
                                            }
                                            if let Some(args) =
                                                f.get("arguments").and_then(|a| a.as_str())
                                            {
                                                let current = tool_calls[idx]
                                                    .arguments
                                                    .as_str()
                                                    .unwrap_or("");
                                                let new_args = format!("{}{}", current, args);
                                                tool_calls[idx].arguments =
                                                    serde_json::Value::String(new_args);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(reason) =
                                choice.get("finish_reason").and_then(|r| r.as_str())
                            {
                                stop_reason = match reason {
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                }
            }

            // Parse accumulated tool call arguments
            for tc in &mut tool_calls {
                if let serde_json::Value::String(s) = &tc.arguments {
                    tc.arguments = serde_json::from_str(s).unwrap_or(serde_json::json!({}));
                }
            }

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens,
                output_tokens,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // AWS Bedrock Implementation (Converse API + SigV4 signing)
    // ========================================================================

    /// Compute HMAC-SHA256
    fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
        use ring::hmac;
        let k = hmac::Key::new(hmac::HMAC_SHA256, key);
        hmac::sign(&k, msg).as_ref().to_vec()
    }

    /// Sign a request with AWS Signature V4
    #[allow(clippy::too_many_arguments)]
    fn sign_aws_v4(
        method: &str,
        uri: &str,
        host: &str,
        body: &[u8],
        access_key: &str,
        secret_key: &str,
        region: &str,
        service: &str,
    ) -> (String, String, String) {
        use sha2::{Digest, Sha256};

        let now = chrono::Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Hash payload
        let payload_hash = hex::encode(Sha256::digest(body));

        // Canonical headers (must be sorted)
        let canonical_headers = format!(
            "content-type:application/json\nhost:{}\nx-amz-date:{}\n",
            host, amz_date
        );
        let signed_headers = "content-type;host;x-amz-date";

        // Canonical request
        let canonical_request = format!(
            "{}\n{}\n\n{}\n{}\n{}",
            method, uri, canonical_headers, signed_headers, payload_hash
        );

        let canonical_request_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));

        // String to sign
        let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, credential_scope, canonical_request_hash
        );

        // Derive signing key
        let k_date = Self::hmac_sha256(
            format!("AWS4{}", secret_key).as_bytes(),
            date_stamp.as_bytes(),
        );
        let k_region = Self::hmac_sha256(&k_date, region.as_bytes());
        let k_service = Self::hmac_sha256(&k_region, service.as_bytes());
        let k_signing = Self::hmac_sha256(&k_service, b"aws4_request");

        let signature = hex::encode(Self::hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            access_key, credential_scope, signed_headers, signature
        );

        (authorization, amz_date, payload_hash)
    }

    /// Convert messages to Bedrock Converse API format
    fn to_bedrock_messages(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    _ => "user",
                };

                // Handle tool results
                if !m.tool_results.is_empty() {
                    let content: Vec<serde_json::Value> = m
                        .tool_results
                        .iter()
                        .map(|tr| {
                            serde_json::json!({
                                "toolResult": {
                                    "toolUseId": tr.call_id,
                                    "content": [{"text": tr.output}],
                                    "status": if tr.success { "success" } else { "error" }
                                }
                            })
                        })
                        .collect();
                    return serde_json::json!({ "role": role, "content": content });
                }

                // Handle tool calls from assistant
                if !m.tool_calls.is_empty() {
                    let mut content: Vec<serde_json::Value> = Vec::new();
                    if !m.content.is_empty() {
                        content.push(serde_json::json!({"text": m.content}));
                    }
                    for tc in &m.tool_calls {
                        content.push(serde_json::json!({
                            "toolUse": {
                                "toolUseId": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            }
                        }));
                    }
                    return serde_json::json!({ "role": role, "content": content });
                }

                serde_json::json!({
                    "role": role,
                    "content": [{"text": m.content}]
                })
            })
            .collect()
    }

    /// Convert tools to Bedrock Converse API format
    fn to_bedrock_tools(&self, tools: &[ToolSchema]) -> serde_json::Value {
        let tool_specs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "toolSpec": {
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": {
                            "json": t.parameters
                        }
                    }
                })
            })
            .collect();
        serde_json::json!({"tools": tool_specs})
    }

    /// Parse Bedrock Converse API response
    fn parse_bedrock_response(&self, text: &str) -> Result<LlmResponse> {
        let json: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::Llm(format!("JSON parse error: {}", e)))?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(output) = json.get("output").and_then(|o| o.get("message"))
            && let Some(parts) = output.get("content").and_then(|c| c.as_array())
        {
            for part in parts {
                if let Some(text_val) = part.get("text").and_then(|t| t.as_str()) {
                    content.push_str(text_val);
                }
                if let Some(tool_use) = part.get("toolUse") {
                    let id = tool_use
                        .get("toolUseId")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = tool_use
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = tool_use
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
            }
        }

        let stop_reason = match json.get("stopReason").and_then(|r| r.as_str()) {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        // Extract token usage from Bedrock Converse API response
        let usage = json.get("usage");
        let input_tokens = usage
            .and_then(|u| u.get("inputTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let output_tokens = usage
            .and_then(|u| u.get("outputTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(LlmResponse {
            content,
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
            cached_tokens: 0,
        })
    }

    async fn complete_bedrock(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let access_key = env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| Error::Llm("AWS_ACCESS_KEY_ID not set".to_string()))?;
        let secret_key = env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| Error::Llm("AWS_SECRET_ACCESS_KEY not set".to_string()))?;
        let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let bedrock_messages = self.to_bedrock_messages(messages);

        let mut body = serde_json::json!({
            "messages": bedrock_messages,
            "inferenceConfig": {
                "maxTokens": 8192
            }
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!([{"text": sys}]);
        }

        if !tools.is_empty() {
            body["toolConfig"] = self.to_bedrock_tools(tools);
        }

        let uri = format!("/model/{}/converse", self.model);
        let url = format!("{}{}", self.base_url, uri);
        let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::Llm(e.to_string()))?;

        let host = format!("bedrock-runtime.{}.amazonaws.com", region);
        let (authorization, amz_date, payload_hash) = Self::sign_aws_v4(
            "POST",
            &uri,
            &host,
            &body_bytes,
            &access_key,
            &secret_key,
            &region,
            "bedrock",
        );

        debug!("Bedrock complete: model={}", self.model);

        let response = self
            .client
            .post(&url)
            .header("Authorization", &authorization)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", &payload_hash)
            .header("content-type", "application/json")
            .header("host", &host)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        if !status.is_success() {
            return Err(Error::Llm(format!(
                "Bedrock API error {}: {}",
                status, text
            )));
        }

        self.parse_bedrock_response(&text)
    }

    async fn stream_bedrock(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        // Bedrock ConverseStream uses AWS event stream binary encoding.
        // For simplicity, use the synchronous Converse API and send the
        // complete response through the channel as a single chunk.
        let access_key = env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| Error::Llm("AWS_ACCESS_KEY_ID not set".to_string()))?;
        let secret_key = env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| Error::Llm("AWS_SECRET_ACCESS_KEY not set".to_string()))?;
        let region = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let bedrock_messages = self.to_bedrock_messages(messages);

        let mut body = serde_json::json!({
            "messages": bedrock_messages,
            "inferenceConfig": {
                "maxTokens": 8192
            }
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!([{"text": sys}]);
        }

        if !tools.is_empty() {
            body["toolConfig"] = self.to_bedrock_tools(tools);
        }

        let model = self.model.clone();
        let base_url = self.base_url.clone();
        let (tx, rx) = mpsc::channel(100);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let uri = format!("/model/{}/converse", model);
            let url = format!("{}{}", base_url, uri);
            let body_bytes = match serde_json::to_vec(&body) {
                Ok(b) => b,
                Err(e) => {
                    error!("Bedrock body serialization failed: {}", e);
                    return LlmResponse {
                        content: format!("Bedrock body serialization failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            let host = format!("bedrock-runtime.{}.amazonaws.com", region);
            let (authorization, amz_date, payload_hash) = LlmClient::sign_aws_v4(
                "POST",
                &uri,
                &host,
                &body_bytes,
                &access_key,
                &secret_key,
                &region,
                "bedrock",
            );

            let response = match client
                .post(&url)
                .header("Authorization", &authorization)
                .header("x-amz-date", &amz_date)
                .header("x-amz-content-sha256", &payload_hash)
                .header("content-type", "application/json")
                .header("host", &host)
                .body(body_bytes)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("Bedrock stream request failed: {}", e);
                    return LlmResponse {
                        content: format!("Bedrock stream request failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Check HTTP status before reading response
            if !response.status().is_success() {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read error body".to_string());
                error!("Bedrock API error {}: {}", status, error_body);
                let err_msg = format!("Bedrock API error ({}): {}", status, error_body);
                let _ = tx.send(format!("[{}]", err_msg)).await;
                return LlmResponse {
                    content: err_msg,
                    tool_calls: vec![],
                    stop_reason: StopReason::Error,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                };
            }

            let text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    error!("Bedrock response read failed: {}", e);
                    return LlmResponse {
                        content: format!("Bedrock response read failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            // Parse the response
            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(j) => j,
                Err(e) => {
                    error!("Bedrock JSON parse failed: {}", e);
                    return LlmResponse {
                        content: format!("Bedrock JSON parse failed: {}", e),
                        tool_calls: vec![],
                        stop_reason: StopReason::Error,
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                    };
                }
            };

            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            if let Some(output) = json.get("output").and_then(|o| o.get("message"))
                && let Some(parts) = output.get("content").and_then(|c| c.as_array())
            {
                for part in parts {
                    if let Some(text_val) = part.get("text").and_then(|t| t.as_str()) {
                        content.push_str(text_val);
                        let _ = tx.send(text_val.to_string()).await;
                    }
                    if let Some(tool_use) = part.get("toolUse") {
                        let id = tool_use
                            .get("toolUseId")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = tool_use
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let input = tool_use
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments: input,
                        });
                    }
                }
            }

            let stop_reason = match json.get("stopReason").and_then(|r| r.as_str()) {
                Some("tool_use") => StopReason::ToolUse,
                Some("max_tokens") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };

            LlmResponse {
                content,
                tool_calls,
                stop_reason,
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
            }
        });

        Ok((rx, handle))
    }

    // ========================================================================
    // Token Counting
    // ========================================================================

    /// Estimate token count for a message array.
    ///
    /// Uses a 4-characters-per-token heuristic as baseline.
    /// Tool calls count their serialized JSON arguments.
    /// Tool results count their output text.
    pub fn count_tokens(messages: &[Message]) -> usize {
        let mut total_chars: usize = 0;
        for msg in messages {
            total_chars += msg.content.len();
            // Count tool call arguments as serialized JSON
            for tc in &msg.tool_calls {
                total_chars += tc.name.len();
                total_chars += tc.arguments.to_string().len();
            }
            // Count tool result outputs
            for tr in &msg.tool_results {
                total_chars += tr.output.len();
            }
        }
        // 4 chars per token heuristic, minimum 1 token per non-empty input
        let estimated = total_chars / 4;
        if total_chars > 0 && estimated == 0 {
            1
        } else {
            estimated
        }
    }

    /// Estimate token count for a single string
    pub fn count_tokens_str(text: &str) -> usize {
        let estimated = text.len() / 4;
        if !text.is_empty() && estimated == 0 {
            1
        } else {
            estimated
        }
    }
}

// ── OpenRouterResolver ────────────────────────────────────────────────────
//
// Dynamic model-capability resolver for OpenRouter. Fetches the catalog
// from `https://openrouter.ai/api/v1/models`, matches the requested model
// string, and returns a `DynamicModelCapabilities` populated from the
// catalog entry. Notably populates `thinking_mode_temperature_lock` for
// MiMo-family models (`xiaomi/mimo*`) which require `temperature=1.0`
// when reasoning is active.
//
// #44-b Step 2 (split A): impl + /v1/models fetcher, no consumer wiring
// yet. Wired into `complete_openrouter` in Step 2 split B.
//
// Cache layer is intentionally NOT included in this SHA — deferred to
// split B per coord (R-2) scope-lock. Each `resolve()` call currently
// performs a fresh HTTP fetch; consumers should wrap this in a cache
// before production use.
pub struct OpenRouterResolver {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl OpenRouterResolver {
    pub fn new(client: reqwest::Client, base_url: String, api_key: Option<String>) -> Self {
        Self {
            client,
            base_url,
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl crate::capabilities::ModelCapabilityResolver for OpenRouterResolver {
    async fn resolve(
        &self,
        model: &str,
    ) -> Option<crate::capabilities::DynamicModelCapabilities> {
        let url = format!("{}/v1/models", self.base_url.trim_end_matches('/'));
        let mut req = self.client.get(&url);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        let entries = body.get("data")?.as_array()?;

        for entry in entries {
            let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id != model {
                continue;
            }
            return map_openrouter_entry(model, entry);
        }
        None
    }
}

/// Pure helper: map a single OpenRouter `/v1/models` entry → DynamicModelCapabilities.
///
/// Extracted from `OpenRouterResolver::resolve()` for unit-testability (catch #44-b-ii-B').
/// Synthetic `serde_json::Value` entries can be passed directly without HTTP infra.
///
/// Caller is responsible for matching `entry["id"] == model` before invoking; this helper
/// does not re-check the slug match (it trusts the caller's `for entry in entries` filter).
pub(crate) fn map_openrouter_entry(
    model: &str,
    entry: &serde_json::Value,
) -> Option<crate::capabilities::DynamicModelCapabilities> {
    let arch = entry.get("architecture");
    let modalities = arch
        .and_then(|a| a.get("input_modalities"))
        .and_then(|m| m.as_array());
    let supports_vision = modalities
        .map(|m| m.iter().any(|v| v.as_str() == Some("image")))
        .unwrap_or(false);
    let supported_params = entry
        .get("supported_parameters")
        .and_then(|p| p.as_array());
    let supports_tools = supported_params
        .map(|p| p.iter().any(|v| v.as_str() == Some("tools")))
        .unwrap_or(false);
    let context_length = entry
        .get("context_length")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let family = arch
        .and_then(|a| a.get("modality"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // MiMo and other thinking-mode models that pin temperature=1.0
    // OpenRouter slug pattern: `xiaomi/mimo*` (e.g. `xiaomi/mimo-vl-7b-rl`)
    let thinking_mode_temperature_lock = if model.starts_with("xiaomi/mimo") {
        Some(true)
    } else {
        None
    };
    Some(crate::capabilities::DynamicModelCapabilities {
        supports_vision,
        supports_tools,
        supports_embeddings: false,
        supports_system_prompt: true,
        context_length,
        family,
        thinking_mode_temperature_lock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::Attachment;

    #[test]
    fn test_llm_client_creation() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string());
        assert!(client.is_ok());
    }

    #[test]
    fn test_thinking_budget_levels() {
        assert_eq!(LlmClient::thinking_budget("low"), 2048);
        assert_eq!(LlmClient::thinking_budget("medium"), 8192);
        assert_eq!(LlmClient::thinking_budget("high"), 32768);
        assert_eq!(LlmClient::thinking_budget("xhigh"), 65536);
        assert_eq!(LlmClient::thinking_budget("unknown"), 8192);
    }

    #[test]
    fn test_with_thinking_builder() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize")
            .with_thinking("high");
        assert_eq!(client.thinking_level, Some("high".to_string()));
    }

    #[test]
    fn test_anthropic_multimodal_message() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize");
        let msg = Message::user_with_attachments(
            "What is in this image?",
            vec![Attachment {
                mime_type: "image/jpeg".to_string(),
                data: vec![0xFF, 0xD8, 0xFF],
                filename: None,
                source_url: None,
            }],
        );
        let result = client.to_anthropic_messages(&[msg]);
        assert_eq!(result.len(), 1);
        let content = result[0]["content"].as_array().expect("should be an array");
        assert_eq!(content.len(), 2); // image + text
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "What is in this image?");
    }

    #[test]
    fn test_openai_multimodal_message() {
        let client =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");
        let msg = Message::user_with_attachments(
            "Describe this",
            vec![Attachment {
                mime_type: "image/png".to_string(),
                data: vec![0x89, 0x50, 0x4E, 0x47],
                filename: None,
                source_url: None,
            }],
        );
        let result = client.to_openai_messages(&[msg], None);
        assert_eq!(result.len(), 1);
        let content = result[0]["content"].as_array().expect("should be an array");
        assert_eq!(content.len(), 2); // image_url + text
        assert_eq!(content[0]["type"], "image_url");
        let url = content[0]["image_url"]["url"]
            .as_str()
            .expect("should be a string");
        assert!(url.starts_with("data:image/png;base64,"));
        assert_eq!(content[1]["type"], "text");
    }

    #[test]
    fn test_no_attachments_plain_text() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize");
        let msg = Message::user("Hello");
        let result = client.to_anthropic_messages(&[msg]);
        assert_eq!(result.len(), 1);
        // Plain text, not array content
        assert_eq!(result[0]["content"], "Hello");
    }

    // ========================================================================
    // Google Gemini tests
    // ========================================================================

    #[test]
    fn test_google_client_creation() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string());
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert_eq!(client.base_url, "https://generativelanguage.googleapis.com");
    }

    #[test]
    fn test_google_credential_status_missing() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Google")
            );
        }
    }

    #[test]
    fn test_gemini_messages_basic() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let messages = vec![
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("How are you?"),
        ];
        let contents = client.to_gemini_contents(&messages);
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "Hi there!");
        assert_eq!(contents[2]["role"], "user");
    }

    #[test]
    fn test_gemini_messages_skip_system() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let messages = vec![Message::system("You are helpful"), Message::user("Hello")];
        let contents = client.to_gemini_contents(&messages);
        // System messages are skipped (handled via systemInstruction)
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn test_gemini_tools_format() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let tools = vec![ToolSchema {
            name: "get_weather".to_string(),
            description: "Get current weather".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                }
            }),
        }];
        let result = client.to_gemini_tools(&tools);
        let declarations = result[0]["functionDeclarations"]
            .as_array()
            .expect("should be an array");
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0]["name"], "get_weather");
        assert_eq!(declarations[0]["description"], "Get current weather");
        assert!(declarations[0]["parameters"].is_object());
    }

    #[test]
    fn test_gemini_parse_response_text() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let response = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello! How can I help you?"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        }"#;
        let result = client
            .parse_gemini_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello! How can I help you?");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_gemini_parse_response_tool_call() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let response = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "San Francisco"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        }"#;
        let result = client
            .parse_gemini_response(response)
            .expect("should parse successfully");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(result.tool_calls[0].arguments["location"], "San Francisco");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn test_gemini_parse_response_max_tokens() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let response = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "truncated"}],
                    "role": "model"
                },
                "finishReason": "MAX_TOKENS"
            }]
        }"#;
        let result = client
            .parse_gemini_response(response)
            .expect("should parse successfully");
        assert_eq!(result.stop_reason, StopReason::MaxTokens);
    }

    #[test]
    fn test_gemini_multimodal_message() {
        let client = LlmClient::new(Provider::Google, "gemini-2.0-flash".to_string())
            .expect("should serialize");
        let msg = Message::user_with_attachments(
            "What is this?",
            vec![Attachment {
                mime_type: "image/jpeg".to_string(),
                data: vec![0xFF, 0xD8, 0xFF],
                filename: None,
                source_url: None,
            }],
        );
        let contents = client.to_gemini_contents(&[msg]);
        assert_eq!(contents.len(), 1);
        let parts = contents[0]["parts"].as_array().expect("should be an array");
        assert_eq!(parts.len(), 2); // inlineData + text
        assert!(parts[0]["inlineData"].is_object());
        assert_eq!(parts[0]["inlineData"]["mimeType"], "image/jpeg");
        assert_eq!(parts[1]["text"], "What is this?");
    }

    // ========================================================================
    // Groq tests
    // ========================================================================

    #[test]
    fn test_groq_client_creation() {
        let client = LlmClient::new(Provider::Groq, "llama-3.3-70b-versatile".to_string());
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert_eq!(client.base_url, "https://api.groq.com/openai");
    }

    #[test]
    fn test_groq_credential_status_missing() {
        let client = LlmClient::new(Provider::Groq, "llama-3.3-70b-versatile".to_string())
            .expect("should serialize");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Groq")
            );
        }
    }

    #[test]
    fn test_groq_reuses_openai_format() {
        // Groq uses OpenAI-compatible format, so messages should format identically
        let groq = LlmClient::new(Provider::Groq, "llama-3.3-70b-versatile".to_string())
            .expect("should serialize");
        let openai =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi")];
        let groq_msgs = groq.to_openai_messages(&messages, Some("System prompt"));
        let openai_msgs = openai.to_openai_messages(&messages, Some("System prompt"));

        assert_eq!(groq_msgs.len(), openai_msgs.len());
        // Both should have system + user + assistant = 3
        assert_eq!(groq_msgs.len(), 3);
        assert_eq!(groq_msgs[0]["role"], "system");
        assert_eq!(groq_msgs[1]["role"], "user");
        assert_eq!(groq_msgs[2]["role"], "assistant");
    }

    // ========================================================================
    // Provider enum tests
    // ========================================================================

    #[test]
    fn test_google_env_key() {
        assert_eq!(Provider::Google.env_key(), "GOOGLE_API_KEY");
    }

    #[test]
    fn test_groq_env_key() {
        assert_eq!(Provider::Groq.env_key(), "GROQ_API_KEY");
    }

    // ========================================================================
    // Sprint 2 P2-Qwen: endpoint resolution + env var chain + catalog filter.
    //
    // These tests mutate process env vars, so they're serialized behind a
    // module-local mutex to avoid cross-test bleed when run with multiple
    // test threads. `unsafe { set_var }` is required by Rust 2024 edition.
    // ========================================================================

    static QWEN_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Clear all Qwen-related env vars so each test starts from a known state.
    fn clear_qwen_env() {
        unsafe {
            std::env::remove_var("QWEN_BASE_URL");
            std::env::remove_var("QWEN_REGION");
            std::env::remove_var("QWEN_PLAN");
            std::env::remove_var("QWEN_API_KEY");
            std::env::remove_var("DASHSCOPE_API_KEY");
            std::env::remove_var("MODELSTUDIO_API_KEY");
        }
    }

    #[test]
    fn test_qwen_base_url_default_is_standard_global() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        assert_eq!(
            resolve_qwen_base_url(),
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
        );
    }

    #[test]
    fn test_qwen_base_url_standard_cn() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_REGION", "cn");
        }
        assert_eq!(
            resolve_qwen_base_url(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_base_url_coding_global() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_PLAN", "coding");
        }
        assert_eq!(
            resolve_qwen_base_url(),
            "https://coding-intl.dashscope.aliyuncs.com/v1"
        );
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_base_url_coding_cn() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_PLAN", "coding");
            std::env::set_var("QWEN_REGION", "cn");
        }
        assert_eq!(
            resolve_qwen_base_url(),
            "https://coding.dashscope.aliyuncs.com/v1"
        );
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_base_url_explicit_override_wins() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        // An explicit base URL must override structured region+plan.
        unsafe {
            std::env::set_var("QWEN_BASE_URL", "https://proxy.example.com/qwen/v1");
            std::env::set_var("QWEN_REGION", "cn");
            std::env::set_var("QWEN_PLAN", "coding");
        }
        assert_eq!(resolve_qwen_base_url(), "https://proxy.example.com/qwen/v1");
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_base_url_trailing_slash_stripped() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_BASE_URL", "https://proxy.example.com/v1/");
        }
        assert_eq!(resolve_qwen_base_url(), "https://proxy.example.com/v1");
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_base_url_region_case_insensitive() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_REGION", "CN");
            std::env::set_var("QWEN_PLAN", "STANDARD");
        }
        assert_eq!(
            resolve_qwen_base_url(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_api_key_chain_primary_wins() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_API_KEY", "qk-primary");
            std::env::set_var("DASHSCOPE_API_KEY", "qk-dashscope");
            std::env::set_var("MODELSTUDIO_API_KEY", "qk-modelstudio");
        }
        assert_eq!(resolve_qwen_api_key().as_deref(), Some("qk-primary"));
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_api_key_chain_dashscope_fallback() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("DASHSCOPE_API_KEY", "qk-dashscope");
        }
        assert_eq!(resolve_qwen_api_key().as_deref(), Some("qk-dashscope"));
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_api_key_chain_modelstudio_fallback() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("MODELSTUDIO_API_KEY", "qk-ms");
        }
        assert_eq!(resolve_qwen_api_key().as_deref(), Some("qk-ms"));
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_api_key_chain_all_unset() {
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        assert_eq!(resolve_qwen_api_key(), None);
    }

    #[test]
    fn test_qwen_api_key_chain_empty_string_skipped() {
        // Empty env var should not be treated as a valid key.
        let _g = QWEN_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear_qwen_env();
        unsafe {
            std::env::set_var("QWEN_API_KEY", "");
            std::env::set_var("DASHSCOPE_API_KEY", "   ");
            std::env::set_var("MODELSTUDIO_API_KEY", "qk-real");
        }
        assert_eq!(resolve_qwen_api_key().as_deref(), Some("qk-real"));
        clear_qwen_env();
    }

    #[test]
    fn test_qwen_catalog_has_expected_models() {
        let cat = qwen_bundled_catalog();
        let ids: Vec<_> = cat.iter().map(|m| m.id).collect();
        assert!(ids.contains(&"qwen3.5-plus"));
        assert!(ids.contains(&"qwen3.6-plus"));
        assert!(ids.contains(&"qwen3-max-2026-01-23"));
        assert!(ids.contains(&"qwen3-coder-plus"));
        // Cross-brand models hosted via Alibaba
        assert!(ids.contains(&"MiniMax-M2.5"));
        assert!(ids.contains(&"glm-5"));
        assert!(ids.contains(&"kimi-k2.5"));
    }

    #[test]
    fn test_qwen_catalog_filter_standard_endpoint_keeps_all() {
        let filtered =
            qwen_filtered_catalog("https://dashscope-intl.aliyuncs.com/compatible-mode/v1");
        let ids: Vec<_> = filtered.iter().map(|m| m.id).collect();
        // qwen3.6-plus is StandardOnly — must be present on Standard
        assert!(ids.contains(&"qwen3.6-plus"));
        // Full catalog otherwise
        assert_eq!(filtered.len(), qwen_bundled_catalog().len());
    }

    #[test]
    fn test_qwen_catalog_filter_coding_endpoint_drops_standard_only() {
        let filtered = qwen_filtered_catalog("https://coding-intl.dashscope.aliyuncs.com/v1");
        let ids: Vec<_> = filtered.iter().map(|m| m.id).collect();
        // qwen3.6-plus is Standard-only — must be dropped on Coding Plan
        assert!(!ids.contains(&"qwen3.6-plus"));
        // But Any-scope models should still be present
        assert!(ids.contains(&"qwen3.5-plus"));
        assert!(ids.contains(&"qwen3-coder-plus"));
    }

    #[test]
    fn test_qwen_catalog_filter_coding_cn_endpoint_also_drops_standard_only() {
        let filtered = qwen_filtered_catalog("https://coding.dashscope.aliyuncs.com/v1");
        let ids: Vec<_> = filtered.iter().map(|m| m.id).collect();
        assert!(!ids.contains(&"qwen3.6-plus"));
    }

    #[test]
    fn test_qwen_catalog_filter_unknown_endpoint_keeps_all() {
        // User-supplied proxy URL — we can't know what it serves, so keep
        // the full catalog rather than mis-filter.
        let filtered = qwen_filtered_catalog("https://proxy.example.com/qwen/v1");
        assert_eq!(filtered.len(), qwen_bundled_catalog().len());
    }

    // ========================================================================
    // Mistral tests
    // ========================================================================

    #[test]
    fn test_mistral_client_creation() {
        let client = LlmClient::new(Provider::Mistral, "mistral-large-latest".to_string());
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert_eq!(client.base_url, "https://api.mistral.ai");
    }

    #[test]
    fn test_mistral_credential_status_missing() {
        let client = LlmClient::new(Provider::Mistral, "mistral-large-latest".to_string())
            .expect("should serialize");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Mistral")
            );
        }
    }

    #[test]
    fn test_mistral_reuses_openai_format() {
        // Mistral uses OpenAI-compatible format, so messages should format identically
        let mistral = LlmClient::new(Provider::Mistral, "mistral-large-latest".to_string())
            .expect("should serialize");
        let openai =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi")];
        let mistral_msgs = mistral.to_openai_messages(&messages, Some("System prompt"));
        let openai_msgs = openai.to_openai_messages(&messages, Some("System prompt"));

        assert_eq!(mistral_msgs.len(), openai_msgs.len());
        // Both should have system + user + assistant = 3
        assert_eq!(mistral_msgs.len(), 3);
        assert_eq!(mistral_msgs[0]["role"], "system");
        assert_eq!(mistral_msgs[1]["role"], "user");
        assert_eq!(mistral_msgs[2]["role"], "assistant");
    }

    #[test]
    fn test_mistral_env_key() {
        assert_eq!(Provider::Mistral.env_key(), "MISTRAL_API_KEY");
    }

    #[test]
    fn test_mistral_parse_openai_response() {
        let client = LlmClient::new(Provider::Mistral, "mistral-large-latest".to_string())
            .expect("should serialize");
        let response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello from Mistral!"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello from Mistral!");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_mistral_parse_tool_call_response() {
        let client = LlmClient::new(Provider::Mistral, "mistral-large-latest".to_string())
            .expect("should serialize");
        let response = r#"{
            "id": "chatcmpl-456",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\": \"/tmp/test.txt\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_abc123");
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.tool_calls[0].arguments["path"], "/tmp/test.txt");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
    }

    // ========================================================================
    // Together AI tests
    // ========================================================================

    #[test]
    fn test_together_client_creation() {
        let client = LlmClient::new(
            Provider::Together,
            "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
        );
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert_eq!(client.base_url, "https://api.together.xyz");
    }

    #[test]
    fn test_together_credential_status_missing() {
        let client = LlmClient::new(
            Provider::Together,
            "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
        )
        .expect("operation should succeed");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Together")
            );
        }
    }

    #[test]
    fn test_together_reuses_openai_format() {
        let together = LlmClient::new(
            Provider::Together,
            "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
        )
        .expect("operation should succeed");
        let openai =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi")];
        let together_msgs = together.to_openai_messages(&messages, Some("System prompt"));
        let openai_msgs = openai.to_openai_messages(&messages, Some("System prompt"));

        assert_eq!(together_msgs.len(), openai_msgs.len());
        assert_eq!(together_msgs.len(), 3);
    }

    #[test]
    fn test_together_env_key() {
        assert_eq!(Provider::Together.env_key(), "TOGETHER_API_KEY");
    }

    #[test]
    fn test_together_parse_response() {
        let client = LlmClient::new(
            Provider::Together,
            "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
        )
        .expect("operation should succeed");
        let response = r#"{
            "id": "chatcmpl-tog-123",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello from Together!"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello from Together!");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    // ========================================================================
    // Fireworks AI tests
    // ========================================================================

    #[test]
    fn test_fireworks_client_creation() {
        let client = LlmClient::new(
            Provider::Fireworks,
            "accounts/fireworks/models/llama-v3p1-405b-instruct".to_string(),
        );
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert_eq!(client.base_url, "https://api.fireworks.ai/inference");
    }

    #[test]
    fn test_fireworks_credential_status_missing() {
        let client = LlmClient::new(
            Provider::Fireworks,
            "accounts/fireworks/models/llama-v3p1-405b-instruct".to_string(),
        )
        .expect("operation should succeed");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Fireworks")
            );
        }
    }

    #[test]
    fn test_fireworks_reuses_openai_format() {
        let fireworks = LlmClient::new(
            Provider::Fireworks,
            "accounts/fireworks/models/llama-v3p1-405b-instruct".to_string(),
        )
        .expect("operation should succeed");
        let openai =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi")];
        let fw_msgs = fireworks.to_openai_messages(&messages, Some("System prompt"));
        let openai_msgs = openai.to_openai_messages(&messages, Some("System prompt"));

        assert_eq!(fw_msgs.len(), openai_msgs.len());
        assert_eq!(fw_msgs.len(), 3);
    }

    #[test]
    fn test_fireworks_env_key() {
        assert_eq!(Provider::Fireworks.env_key(), "FIREWORKS_API_KEY");
    }

    #[test]
    fn test_fireworks_parse_response() {
        let client = LlmClient::new(
            Provider::Fireworks,
            "accounts/fireworks/models/llama-v3p1-405b-instruct".to_string(),
        )
        .expect("operation should succeed");
        let response = r#"{
            "id": "chatcmpl-fw-789",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello from Fireworks!"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello from Fireworks!");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    // ========================================================================
    // Azure OpenAI tests
    // ========================================================================

    #[test]
    fn test_azure_client_creation() {
        let client = LlmClient::new(Provider::Azure, "gpt-4o".to_string());
        assert!(client.is_ok());
    }

    #[test]
    fn test_azure_credential_status_missing() {
        let client =
            LlmClient::new(Provider::Azure, "gpt-4o".to_string()).expect("should serialize");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "Azure OpenAI")
            );
        }
    }

    #[test]
    fn test_azure_reuses_openai_format() {
        let azure =
            LlmClient::new(Provider::Azure, "gpt-4o".to_string()).expect("should serialize");
        let openai =
            LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("should serialize");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi")];
        let azure_msgs = azure.to_openai_messages(&messages, Some("System prompt"));
        let openai_msgs = openai.to_openai_messages(&messages, Some("System prompt"));

        assert_eq!(azure_msgs.len(), openai_msgs.len());
        assert_eq!(azure_msgs.len(), 3);
        assert_eq!(azure_msgs[0]["role"], "system");
        assert_eq!(azure_msgs[1]["role"], "user");
        assert_eq!(azure_msgs[2]["role"], "assistant");
    }

    #[test]
    fn test_azure_env_key() {
        assert_eq!(Provider::Azure.env_key(), "AZURE_OPENAI_API_KEY");
    }

    #[test]
    fn test_azure_parse_response() {
        let client =
            LlmClient::new(Provider::Azure, "gpt-4o".to_string()).expect("should serialize");
        let response = r#"{
            "id": "chatcmpl-azure-123",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello from Azure OpenAI!"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello from Azure OpenAI!");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_azure_parse_tool_call_response() {
        let client =
            LlmClient::new(Provider::Azure, "gpt-4o".to_string()).expect("should serialize");
        let response = r#"{
            "id": "chatcmpl-azure-456",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_azure_abc",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\": \"/tmp/test.txt\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;
        let result = client
            .parse_openai_response(response)
            .expect("should parse successfully");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_azure_abc");
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.tool_calls[0].arguments["path"], "/tmp/test.txt");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
    }

    // ========================================================================
    // AWS Bedrock tests
    // ========================================================================

    #[test]
    fn test_bedrock_client_creation() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn test_bedrock_credential_status_missing() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        if matches!(client.auth, AuthMethod::None) {
            let status = client.credential_status();
            assert!(
                matches!(status, CredentialStatus::Missing { provider, .. } if provider == "AWS Bedrock")
            );
        }
    }

    #[test]
    fn test_bedrock_env_key() {
        assert_eq!(Provider::Bedrock.env_key(), "AWS_ACCESS_KEY_ID");
    }

    #[test]
    fn test_bedrock_messages_format() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let messages = vec![
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("How are you?"),
        ];
        let bedrock_msgs = client.to_bedrock_messages(&messages);
        assert_eq!(bedrock_msgs.len(), 3);
        assert_eq!(bedrock_msgs[0]["role"], "user");
        assert_eq!(bedrock_msgs[0]["content"][0]["text"], "Hello");
        assert_eq!(bedrock_msgs[1]["role"], "assistant");
        assert_eq!(bedrock_msgs[1]["content"][0]["text"], "Hi there!");
        assert_eq!(bedrock_msgs[2]["role"], "user");
    }

    #[test]
    fn test_bedrock_messages_skip_system() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let messages = vec![Message::system("You are helpful"), Message::user("Hello")];
        let bedrock_msgs = client.to_bedrock_messages(&messages);
        // System messages are skipped (handled via system field)
        assert_eq!(bedrock_msgs.len(), 1);
        assert_eq!(bedrock_msgs[0]["role"], "user");
    }

    #[test]
    fn test_bedrock_tools_format() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let tools = vec![ToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        }];
        let result = client.to_bedrock_tools(&tools);
        let tool_list = result["tools"].as_array().expect("should be an array");
        assert_eq!(tool_list.len(), 1);
        assert_eq!(tool_list[0]["toolSpec"]["name"], "read_file");
        assert_eq!(tool_list[0]["toolSpec"]["description"], "Read a file");
        assert!(tool_list[0]["toolSpec"]["inputSchema"]["json"].is_object());
    }

    #[test]
    fn test_bedrock_parse_response_text() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let response = r#"{
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{"text": "Hello from Bedrock!"}]
                }
            },
            "stopReason": "end_turn",
            "usage": {"inputTokens": 10, "outputTokens": 5}
        }"#;
        let result = client
            .parse_bedrock_response(response)
            .expect("should parse successfully");
        assert_eq!(result.content, "Hello from Bedrock!");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_bedrock_parse_response_tool_call() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let response = r#"{
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{
                        "toolUse": {
                            "toolUseId": "tool_abc",
                            "name": "read_file",
                            "input": {"path": "/tmp/test.txt"}
                        }
                    }]
                }
            },
            "stopReason": "tool_use"
        }"#;
        let result = client
            .parse_bedrock_response(response)
            .expect("should parse successfully");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "tool_abc");
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.tool_calls[0].arguments["path"], "/tmp/test.txt");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn test_bedrock_parse_response_max_tokens() {
        let client = LlmClient::new(
            Provider::Bedrock,
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        )
        .expect("operation should succeed");
        let response = r#"{
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{"text": "truncated"}]
                }
            },
            "stopReason": "max_tokens"
        }"#;
        let result = client
            .parse_bedrock_response(response)
            .expect("should parse successfully");
        assert_eq!(result.stop_reason, StopReason::MaxTokens);
    }

    #[test]
    fn test_aws_sigv4_signing() {
        // Test that the signing function produces a valid Authorization header format
        let (auth, amz_date, payload_hash) = LlmClient::sign_aws_v4(
            "POST",
            "/model/test-model/converse",
            "bedrock-runtime.us-east-1.amazonaws.com",
            b"{}",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "bedrock",
        );
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
        assert!(auth.contains("/us-east-1/bedrock/aws4_request"));
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-date"));
        assert!(auth.contains("Signature="));
        assert!(!amz_date.is_empty());
        assert!(!payload_hash.is_empty());
    }

    // ========================================================================
    // Prompt caching tests
    // ========================================================================

    #[test]
    fn test_anthropic_system_cache_control() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("client");
        let messages = vec![Message::user("Hello")];
        let _tools: Vec<ToolSchema> = vec![];
        // Build body the same way complete_anthropic does
        let anthropic_messages = client.to_anthropic_messages(&messages);
        let mut body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 8192,
            "messages": anthropic_messages,
        });
        // Simulate system prompt with cache_control
        let sys = "You are a helpful assistant.";
        body["system"] = serde_json::json!([
            {"type": "text", "text": sys, "cache_control": {"type": "ephemeral"}}
        ]);
        let system_blocks = body["system"].as_array().unwrap();
        assert_eq!(system_blocks.len(), 1);
        assert_eq!(system_blocks[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_anthropic_tool_cache_control() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("client");
        let tools = vec![
            ToolSchema::new("read_file", "Read a file").with_param(
                "path",
                "string",
                "File path",
                true,
            ),
            ToolSchema::new("write_file", "Write a file")
                .with_param("path", "string", "File path", true)
                .with_param("content", "string", "Content", true),
        ];
        let mut anthropic_tools = client.to_anthropic_tools(&tools);
        // Add cache_control to last tool (as the real code does)
        if let Some(arr) = anthropic_tools.as_array_mut() {
            if let Some(last) = arr.last_mut() {
                last["cache_control"] = serde_json::json!({"type": "ephemeral"});
            }
        }
        let arr = anthropic_tools.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // First tool should NOT have cache_control
        assert!(arr[0].get("cache_control").is_none());
        // Last tool should have cache_control
        assert_eq!(arr[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_parse_anthropic_cached_tokens() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("client");
        let response_json = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 25,
                "cache_read_input_tokens": 80,
                "cache_creation_input_tokens": 20
            }
        });
        let result = client
            .parse_anthropic_response(&serde_json::to_string(&response_json).unwrap())
            .unwrap();
        assert_eq!(result.content, "Hello!");
        assert_eq!(result.input_tokens, 100);
        assert_eq!(result.output_tokens, 25);
        assert_eq!(result.cached_tokens, 80);
    }

    #[test]
    fn test_parse_anthropic_no_cache_tokens() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("client");
        let response_json = serde_json::json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hi"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 50,
                "output_tokens": 10
            }
        });
        let result = client
            .parse_anthropic_response(&serde_json::to_string(&response_json).unwrap())
            .unwrap();
        assert_eq!(result.cached_tokens, 0);
        assert_eq!(result.input_tokens, 50);
        assert_eq!(result.output_tokens, 10);
    }

    #[test]
    fn test_parse_openai_usage_tokens() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("client");
        let response_json = serde_json::json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 42,
                "completion_tokens": 8,
                "total_tokens": 50
            }
        });
        let result = client
            .parse_openai_response(&serde_json::to_string(&response_json).unwrap())
            .unwrap();
        assert_eq!(result.input_tokens, 42);
        assert_eq!(result.output_tokens, 8);
        assert_eq!(result.cached_tokens, 0);
    }

    // ========================================================================
    // Seed tests
    // ========================================================================

    #[test]
    fn test_with_seed_builder() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string())
            .expect("client")
            .with_seed(42);
        assert_eq!(client.seed, Some(42));
    }

    #[test]
    fn test_inject_seed_openai() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string())
            .expect("client")
            .with_seed(12345);
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_seed(&mut body);
        assert_eq!(body["seed"], 12345);
    }

    #[test]
    fn test_inject_seed_anthropic_skipped() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("client")
            .with_seed(42);
        let mut body = serde_json::json!({"model": "claude-sonnet-4-20250514"});
        client.inject_seed(&mut body);
        assert!(body.get("seed").is_none());
    }

    #[test]
    fn test_inject_seed_none() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).expect("client");
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_seed(&mut body);
        assert!(body.get("seed").is_none());
    }

    // ========================================================================
    // Token counting tests
    // ========================================================================

    #[test]
    fn test_count_tokens_empty() {
        let messages: Vec<Message> = vec![];
        assert_eq!(LlmClient::count_tokens(&messages), 0);
    }

    #[test]
    fn test_count_tokens_simple() {
        // "Hello world" = 11 chars -> 11/4 = 2 tokens
        let messages = vec![Message::user("Hello world")];
        assert_eq!(LlmClient::count_tokens(&messages), 2);
    }

    #[test]
    fn test_count_tokens_short_string() {
        // "Hi" = 2 chars -> 2/4 = 0, but min 1
        let messages = vec![Message::user("Hi")];
        assert_eq!(LlmClient::count_tokens(&messages), 1);
    }

    #[test]
    fn test_count_tokens_multiple_messages() {
        let messages = vec![
            Message::user("Hello world!"),         // 12 chars
            Message::assistant("How can I help?"), // 15 chars
        ];
        // 27 / 4 = 6
        assert_eq!(LlmClient::count_tokens(&messages), 6);
    }

    #[test]
    fn test_count_tokens_with_tool_calls() {
        let mut msg = Message::assistant("Calling tool");
        msg.tool_calls.push(ToolCall {
            id: "tc1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        });
        let messages = vec![msg];
        // "Calling tool" = 12 + "read_file" = 9 + json args ~25 chars
        let count = LlmClient::count_tokens(&messages);
        assert!(count > 5); // should be substantial
    }

    #[test]
    fn test_count_tokens_str() {
        assert_eq!(LlmClient::count_tokens_str(""), 0);
        assert_eq!(LlmClient::count_tokens_str("Hi"), 1); // min 1
        assert_eq!(LlmClient::count_tokens_str("Hello world"), 2);
        // 100 chars -> 25 tokens
        let text = "a".repeat(100);
        assert_eq!(LlmClient::count_tokens_str(&text), 25);
    }

    #[test]
    fn test_llm_response_cached_tokens_field() {
        let response = LlmResponse {
            content: "cached response".to_string(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            input_tokens: 100,
            output_tokens: 25,
            cached_tokens: 80,
        };
        assert_eq!(response.cached_tokens, 80);
        assert_eq!(response.input_tokens, 100);
    }

    // ============================================================================
    // #44-b-ii-B' — map_openrouter_entry pure-helper tests
    //
    // Verifies the entry-to-caps mapping extracted from OpenRouterResolver::resolve()
    // for unit-testability. Synthetic serde_json entries cover:
    //   - xiaomi/mimo* → thinking_mode_temperature_lock=Some(true)  (positive case)
    //   - anthropic/claude-* → flag=None                            (negative case)
    //   - openai/gpt-* → flag=None                                  (negative case)
    //   - malformed/empty entry → still returns Some(...) with defaults
    //     (caller's `id == model` filter is the entry-presence gate)
    // ============================================================================

    #[test]
    fn map_openrouter_entry_xiaomi_mimo_sets_temperature_lock() {
        let entry = serde_json::json!({
            "id": "xiaomi/mimo-vl-7b-rl",
            "architecture": {
                "input_modalities": ["text", "image"],
                "modality": "multimodal",
            },
            "supported_parameters": ["tools", "temperature"],
            "context_length": 32768u64,
        });
        let caps = map_openrouter_entry("xiaomi/mimo-vl-7b-rl", &entry)
            .expect("MiMo entry should map to Some(caps)");
        assert_eq!(
            caps.thinking_mode_temperature_lock,
            Some(true),
            "xiaomi/mimo* must set thinking_mode_temperature_lock=Some(true)"
        );
        assert!(caps.supports_vision, "image modality should yield supports_vision");
        assert!(caps.supports_tools, "tools param should yield supports_tools");
        assert_eq!(caps.context_length, Some(32768));
        assert_eq!(caps.family, "multimodal");
    }

    #[test]
    fn map_openrouter_entry_anthropic_claude_no_temperature_lock() {
        let entry = serde_json::json!({
            "id": "anthropic/claude-sonnet-4",
            "architecture": {
                "input_modalities": ["text", "image"],
                "modality": "text+vision",
            },
            "supported_parameters": ["tools"],
            "context_length": 200000u64,
        });
        let caps = map_openrouter_entry("anthropic/claude-sonnet-4", &entry)
            .expect("claude entry should map to Some(caps)");
        assert_eq!(
            caps.thinking_mode_temperature_lock, None,
            "anthropic/claude-* must NOT set thinking_mode_temperature_lock"
        );
    }

    #[test]
    fn map_openrouter_entry_openai_gpt_no_temperature_lock() {
        let entry = serde_json::json!({
            "id": "openai/gpt-4o",
            "architecture": {
                "input_modalities": ["text"],
                "modality": "text",
            },
            "supported_parameters": ["tools"],
            "context_length": 128000u64,
        });
        let caps = map_openrouter_entry("openai/gpt-4o", &entry)
            .expect("gpt entry should map to Some(caps)");
        assert_eq!(
            caps.thinking_mode_temperature_lock, None,
            "openai/gpt-* must NOT set thinking_mode_temperature_lock"
        );
        assert!(!caps.supports_vision, "text-only modality → no supports_vision");
    }

    #[test]
    fn map_openrouter_entry_malformed_entry_returns_defaults() {
        // Empty entry — no architecture, no supported_parameters, no context_length.
        // Helper should still return Some(caps) with all-defaults; caller's
        // `id == model` match is the upstream entry-presence gate.
        let entry = serde_json::json!({});
        let caps = map_openrouter_entry("openai/gpt-3.5-turbo", &entry)
            .expect("malformed entry should still map to Some(caps) with defaults");
        assert!(!caps.supports_vision);
        assert!(!caps.supports_tools);
        assert_eq!(caps.context_length, None);
        assert_eq!(caps.family, "");
        assert_eq!(caps.thinking_mode_temperature_lock, None);
    }
}

// ============================================================================
// Additional comprehensive tests
// ============================================================================

#[cfg(test)]
mod additional_tests {
    use super::*;

    // ── AuthMethod ─────────────────────────────────────────────────────────

    #[test]
    fn test_auth_method_api_key() {
        let auth = AuthMethod::ApiKey("sk-test-123".to_string());
        match &auth {
            AuthMethod::ApiKey(key) => assert_eq!(key, "sk-test-123"),
            _ => panic!("Expected ApiKey"),
        }
    }

    #[test]
    fn test_auth_method_oauth() {
        let auth = AuthMethod::OAuth("oauth-token-xyz".to_string());
        match &auth {
            AuthMethod::OAuth(token) => assert_eq!(token, "oauth-token-xyz"),
            _ => panic!("Expected OAuth"),
        }
    }

    #[test]
    fn test_auth_method_none() {
        let auth = AuthMethod::None;
        assert!(matches!(auth, AuthMethod::None));
    }

    // ── CredentialStatus ───────────────────────────────────────────────────

    #[test]
    fn test_credential_status_valid() {
        let status = CredentialStatus::Valid("Anthropic API key configured".to_string());
        match &status {
            CredentialStatus::Valid(msg) => assert!(msg.contains("Anthropic")),
            _ => panic!("Expected Valid"),
        }
    }

    #[test]
    fn test_credential_status_missing() {
        let status = CredentialStatus::Missing {
            provider: "Anthropic".to_string(),
            suggestions: vec![
                "Set ANTHROPIC_API_KEY".to_string(),
                "Use /login for OAuth".to_string(),
            ],
        };
        match &status {
            CredentialStatus::Missing {
                provider,
                suggestions,
            } => {
                assert_eq!(provider, "Anthropic");
                assert_eq!(suggestions.len(), 2);
            }
            _ => panic!("Expected Missing"),
        }
    }

    // ── StopReason ─────────────────────────────────────────────────────────

    #[test]
    fn test_stop_reason_variants() {
        assert_eq!(StopReason::EndTurn, StopReason::EndTurn);
        assert_eq!(StopReason::ToolUse, StopReason::ToolUse);
        assert_eq!(StopReason::MaxTokens, StopReason::MaxTokens);
        assert_eq!(StopReason::Error, StopReason::Error);
        assert_ne!(StopReason::EndTurn, StopReason::Error);
    }

    #[test]
    fn test_stop_reason_debug() {
        let reason = StopReason::EndTurn;
        let debug = format!("{:?}", reason);
        assert_eq!(debug, "EndTurn");
    }

    // ── LlmResponse ───────────────────────────────────────────────────────

    #[test]
    fn test_llm_response_default() {
        let response = LlmResponse {
            content: "Hello!".to_string(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
        };
        assert_eq!(response.content, "Hello!");
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.input_tokens, 10);
        assert_eq!(response.output_tokens, 5);
    }

    #[test]
    fn test_llm_response_with_tool_calls() {
        let tool_call = ToolCall {
            id: "tc1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "test.rs"}),
        };
        let response = LlmResponse {
            content: String::new(),
            tool_calls: vec![tool_call],
            stop_reason: StopReason::ToolUse,
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 0,
        };
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
    }

    // ── LlmClient ─────────────────────────────────────────────────────────

    #[test]
    fn test_client_all_providers() {
        let providers = [
            (Provider::Anthropic, "claude-sonnet-4-20250514"),
            (Provider::OpenAI, "gpt-4"),
            (Provider::Google, "gemini-pro"),
            (Provider::Groq, "llama3-70b"),
            (Provider::Mistral, "mistral-large"),
            (Provider::Ollama, "llama3"),
            (Provider::Together, "meta-llama/Meta-Llama-3-70B"),
            (Provider::Fireworks, "accounts/fireworks/models/llama3"),
        ];
        for (provider, model) in &providers {
            let client = LlmClient::new(provider.clone(), model.to_string());
            assert!(client.is_ok(), "Failed for {:?}", provider);
        }
    }

    #[test]
    fn test_client_has_credentials_none() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize");
        // Without env vars set, credentials depend on env
        let _ = client.has_credentials(); // Should not panic
    }

    #[test]
    fn test_client_credential_status() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize");
        let status = client.credential_status();
        // Should return a valid status (either Valid or Missing)
        match status {
            CredentialStatus::Valid(_) => {}
            CredentialStatus::Missing {
                provider,
                suggestions,
            } => {
                assert_eq!(provider, "Anthropic");
                assert!(!suggestions.is_empty());
            }
        }
    }

    #[test]
    fn test_client_auth_method() {
        let client =
            LlmClient::new(Provider::Ollama, "llama3".to_string()).expect("should serialize");
        // Ollama uses None auth by default
        assert!(matches!(client.auth_method(), AuthMethod::None));
    }

    #[test]
    fn test_client_with_oauth() {
        let client = LlmClient::with_oauth(
            Provider::Anthropic,
            "claude-sonnet-4-20250514".to_string(),
            "test-oauth-token".to_string(),
        );
        assert!(client.is_ok());
        let client = client.expect("operation should succeed");
        assert!(matches!(client.auth_method(), AuthMethod::OAuth(_)));
        assert!(client.has_credentials());
    }

    #[test]
    fn test_thinking_budget_all_levels() {
        assert_eq!(LlmClient::thinking_budget("low"), 2048);
        assert_eq!(LlmClient::thinking_budget("medium"), 8192);
        assert_eq!(LlmClient::thinking_budget("high"), 32768);
        assert_eq!(LlmClient::thinking_budget("xhigh"), 65536);
        // Unknown defaults to medium
        assert_eq!(LlmClient::thinking_budget("invalid"), 8192);
        assert_eq!(LlmClient::thinking_budget(""), 8192);
    }

    #[test]
    fn test_with_thinking_chaining() {
        let client = LlmClient::new(Provider::Anthropic, "claude-sonnet-4-20250514".to_string())
            .expect("should serialize")
            .with_thinking("high");
        // Should not panic, thinking level is set internally
        let _ = client.auth_method();
    }

    // ── Provider ───────────────────────────────────────────────────────────

    #[test]
    fn test_provider_debug() {
        let providers = [
            Provider::Anthropic,
            Provider::OpenAI,
            Provider::Google,
            Provider::Groq,
            Provider::Mistral,
            Provider::Ollama,
            Provider::Together,
            Provider::Fireworks,
            Provider::Azure,
            Provider::Bedrock,
        ];
        for p in &providers {
            let debug = format!("{:?}", p);
            assert!(!debug.is_empty());
        }
    }

    #[test]
    fn test_provider_clone() {
        let p1 = Provider::Anthropic;
        let p2 = p1.clone();
        assert_eq!(format!("{:?}", p1), format!("{:?}", p2));
    }

    // ── Edge cases ─────────────────────────────────────────────────────────

    #[test]
    fn test_empty_model_string() {
        let client = LlmClient::new(Provider::Anthropic, String::new());
        // Empty model is rejected at construction time
        assert!(client.is_err());
    }

    #[test]
    fn test_llm_response_zero_tokens() {
        let response = LlmResponse {
            content: String::new(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::Error,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
        };
        assert!(response.content.is_empty());
        assert_eq!(response.input_tokens, 0);
    }

    // ========================================================================
    // Structured output / response_format tests
    // ========================================================================

    #[test]
    fn test_response_format_text() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        client.set_response_format(ResponseFormat::Text);
        assert_eq!(client.response_format, Some(ResponseFormat::Text));
    }

    #[test]
    fn test_response_format_json_object() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        client.set_response_format(ResponseFormat::JsonObject);
        assert_eq!(client.response_format, Some(ResponseFormat::JsonObject));
    }

    #[test]
    fn test_response_format_json_schema() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name", "age"]
        });
        client.set_response_format(ResponseFormat::JsonSchema {
            name: "person".to_string(),
            schema: schema.clone(),
            strict: true,
        });
        match &client.response_format {
            Some(ResponseFormat::JsonSchema {
                name,
                schema: s,
                strict,
            }) => {
                assert_eq!(name, "person");
                assert_eq!(s, &schema);
                assert!(strict);
            }
            _ => panic!("Expected JsonSchema"),
        }
    }

    #[test]
    fn test_clear_response_format() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        client.set_response_format(ResponseFormat::JsonObject);
        assert!(client.response_format.is_some());
        client.clear_response_format();
        assert!(client.response_format.is_none());
    }

    #[test]
    fn test_inject_response_format_text() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        client.set_response_format(ResponseFormat::Text);
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_response_format(&mut body);
        assert_eq!(body["response_format"]["type"], "text");
    }

    #[test]
    fn test_inject_response_format_json_object() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        client.set_response_format(ResponseFormat::JsonObject);
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_response_format(&mut body);
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_inject_response_format_json_schema() {
        let mut client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "number"}},
            "required": ["x"]
        });
        client.set_response_format(ResponseFormat::JsonSchema {
            name: "point".to_string(),
            schema: schema.clone(),
            strict: false,
        });
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_response_format(&mut body);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "point");
        assert_eq!(body["response_format"]["json_schema"]["strict"], false);
    }

    #[test]
    fn test_inject_response_format_none_leaves_body() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let mut body = serde_json::json!({"model": "gpt-4o"});
        client.inject_response_format(&mut body);
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn test_response_format_default_none() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        assert!(client.response_format.is_none());
    }

    #[test]
    fn test_to_openai_tools_produces_valid_function_calling_format() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let tools = vec![ToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"]
            }),
        }];

        let openai_tools = client.to_openai_tools(&tools);
        let arr = openai_tools.as_array().expect("tools should serialize to an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "read_file");
        assert_eq!(arr[0]["function"]["description"], "Read a file");
        assert_eq!(arr[0]["function"]["parameters"]["type"], "object");
        assert_eq!(arr[0]["function"]["parameters"]["required"], serde_json::json!(["path"]));
    }

    #[test]
    fn test_to_openai_tools_filters_empty_tool_names() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let tools = vec![
            ToolSchema {
                name: "".to_string(),
                description: "Should be dropped".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
            ToolSchema {
                name: "write_file".to_string(),
                description: "Write a file".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ];

        let openai_tools = client.to_openai_tools(&tools);
        let arr = openai_tools.as_array().expect("tools should serialize to an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["function"]["name"], "write_file");
    }

    #[test]
    fn test_openai_request_body_includes_tool_choice_auto_when_tools_present() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let messages = client.to_openai_messages(&[], Some("system prompt"));
        let tools = vec![ToolSchema {
            name: "list_dir".to_string(),
            description: "List a directory".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let openai_tools = client.to_openai_tools(&tools);

        let mut body = serde_json::json!({
            "model": client.model,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = openai_tools;
            body["tool_choice"] = serde_json::json!("auto");
        }

        assert!(body.get("tools").is_some());
        assert_eq!(body["tool_choice"], "auto");
    }

    // ─── inject_openai_sampling: GPT-5.5 + reasoning_effort + temperature ───────

    #[test]
    fn test_sampling_gpt4o_no_tools_sets_temperature() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, false);
        assert_eq!(body["temperature"], 0.3);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_sampling_gpt4o_with_tools_sets_temperature() {
        let client = LlmClient::new(Provider::OpenAI, "gpt-4o".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, true);
        assert_eq!(body["temperature"], 0.3);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_sampling_o3_sets_reasoning_effort_not_temperature() {
        let client = LlmClient::new(Provider::OpenAI, "o3-mini".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, true);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn test_sampling_o1_sets_reasoning_effort_not_temperature() {
        let client = LlmClient::new(Provider::OpenAI, "o1-preview".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, false);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn test_sampling_gpt55_without_tools_sets_reasoning_effort_only() {
        // GPT-5.5 without tools: still a reasoning model, accepts reasoning_effort
        let client = LlmClient::new(Provider::OpenAI, "gpt-5.5".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, false);
        assert!(body.get("temperature").is_none(), "GPT-5.5 must not set temperature");
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn test_sampling_gpt55_with_tools_skips_both() {
        // T6: GPT-5.5+ rejects BOTH temperature AND reasoning_effort when tools are present.
        let client = LlmClient::new(Provider::OpenAI, "gpt-5.5".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, true);
        assert!(body.get("temperature").is_none(), "GPT-5.5+tools must not set temperature");
        assert!(
            body.get("reasoning_effort").is_none(),
            "GPT-5.5+tools must not set reasoning_effort"
        );
    }

    #[test]
    fn test_sampling_gpt55_turbo_with_tools_skips_both() {
        // GPT-5.5+ family — model variants with the gpt-5.5 prefix all behave the same.
        let client = LlmClient::new(Provider::OpenAI, "gpt-5.5-turbo".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, true);
        assert!(body.get("temperature").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_sampling_moonshot_skips_temperature() {
        // Moonshot/Kimi K2.5 only accepts temperature=1 server-side — Zeus skips it.
        let client = LlmClient::new(Provider::Moonshot, "kimi-k2.5".to_string()).unwrap();
        let mut body = serde_json::json!({});
        client.inject_openai_sampling(&mut body, false);
        assert!(body.get("temperature").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    // ── #44-b-ii-C: family-predicate helpers ───────────────────────────────

    #[test]
    fn test_is_reasoning_model_positive_family() {
        // o1/o3/o4/gpt-5.5 family — all positive
        assert!(is_reasoning_model("o1"));
        assert!(is_reasoning_model("o1-mini"));
        assert!(is_reasoning_model("o3"));
        assert!(is_reasoning_model("o3-pro"));
        assert!(is_reasoning_model("o4"));
        assert!(is_reasoning_model("o4-mini"));
        assert!(is_reasoning_model("gpt-5.5"));
        assert!(is_reasoning_model("gpt-5.5-turbo"));
    }

    #[test]
    fn test_is_reasoning_model_negative_family() {
        // Non-reasoning OpenAI + other providers — all negative
        assert!(!is_reasoning_model("gpt-5"));
        assert!(!is_reasoning_model("gpt-5-mini"));
        assert!(!is_reasoning_model("gpt-4o"));
        assert!(!is_reasoning_model("gpt-4-turbo"));
        assert!(!is_reasoning_model("gpt-3.5-turbo"));
        assert!(!is_reasoning_model("claude-opus-4"));
        assert!(!is_reasoning_model("kimi-k2.5"));
        assert!(!is_reasoning_model(""));
    }

    #[test]
    fn test_gpt55_rejects_sampling_only_with_tools() {
        // gpt-5.5 + tools = reject both temperature AND reasoning_effort
        assert!(gpt55_rejects_sampling("gpt-5.5", true));
        assert!(gpt55_rejects_sampling("gpt-5.5-turbo", true));
        // gpt-5.5 WITHOUT tools = still accepts reasoning_effort
        assert!(!gpt55_rejects_sampling("gpt-5.5", false));
        assert!(!gpt55_rejects_sampling("gpt-5.5-turbo", false));
    }

    #[test]
    fn test_gpt55_rejects_sampling_negative_for_other_models() {
        // Non-gpt-5.5 models are never gated by this carve-out
        assert!(!gpt55_rejects_sampling("o1", true));
        assert!(!gpt55_rejects_sampling("o3-pro", true));
        assert!(!gpt55_rejects_sampling("gpt-5", true));
        assert!(!gpt55_rejects_sampling("gpt-4o", true));
        assert!(!gpt55_rejects_sampling("claude-opus-4", true));
        assert!(!gpt55_rejects_sampling("", true));
    }
}
