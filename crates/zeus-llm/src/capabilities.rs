//! Provider Capability Registry — centralized provider-specific quirks and capabilities.
//!
//! Replaces scattered `if provider == X` checks throughout lib.rs with a single
//! lookup table. Adding a new provider means adding one entry here instead of
//! touching 10+ code paths.

use zeus_core::Provider;

/// API compatibility format — determines which message format and endpoint to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    /// OpenAI Chat Completions API format
    OpenAI,
    /// Anthropic Messages API format
    Anthropic,
    /// Google Gemini API (custom envelope)
    GoogleGemini,
}

/// Authentication method supported by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    ApiKey,
    OAuth,
    DeviceCode,
    BrowserOAuth,
    None,
}

/// Centralized capabilities and quirks for an LLM provider.
#[derive(Debug, Clone)]
pub struct ProviderCapabilities {
    /// API format (OpenAI, Anthropic, Gemini)
    pub api_format: ApiFormat,
    /// Supported authentication methods (ordered by preference)
    pub auth_methods: &'static [AuthType],

    /// Model supports tool calling
    pub supports_tools: bool,
    /// Model supports vision (image inputs)
    pub supports_vision: bool,
    /// Model supports extended thinking / reasoning
    pub supports_thinking: bool,
    /// Model supports streaming responses
    pub supports_streaming: bool,
    /// Model supports parallel tool calls in a single response
    pub supports_parallel_tools: bool,

    // ── Cooking behavior ─────────────────────────────────────
    /// Inject tool-call audit summaries between cooking iterations.
    /// Only Anthropic handles mid-conversation system messages well.
    pub supports_audit_logging: bool,
    /// Allow mid-loop message interruption during cooking.
    /// Models that can't resume gracefully should have this disabled.
    pub supports_mid_loop_interrupt: bool,
    /// Minimum cooking iterations for bot-sender messages (default 3).
    /// Some models need more turns to complete tool calls.
    pub bot_sender_min_iterations: usize,

    // ── Quirks ──────────────────────────────────────────────
    /// Skip temperature parameter (Moonshot only accepts 1.0)
    pub skip_temperature: bool,
    /// Skip /v1 prefix in completions path (ZAI uses /v4 directly)
    pub skip_v1_prefix: bool,
    /// Skip parallel_tool_calls parameter (not all providers support it)
    pub skip_parallel_tool_calls: bool,
    /// Default context window size
    pub context_window: usize,
    /// Maximum output (completion) tokens to request. Most providers cap at a
    /// conservative 4096; high-output models (e.g. Sakana Fugu Ultra, 128k)
    /// override this so long responses aren't needlessly truncated.
    pub max_output_tokens: usize,
}

/// Look up capabilities for a provider. Returns sensible defaults for unknown providers.
pub fn capabilities(provider: &Provider) -> ProviderCapabilities {
    match provider {
        Provider::Anthropic => ProviderCapabilities {
            api_format: ApiFormat::Anthropic,
            auth_methods: &[AuthType::ApiKey, AuthType::OAuth],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: true,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: true,  // Claude handles mid-conversation system messages well
            supports_mid_loop_interrupt: true,
            bot_sender_min_iterations: 3,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 200_000,
            max_output_tokens: 4096,
        },
        Provider::OpenAI => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey, AuthType::OAuth],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: true, // o-series models
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Google => ProviderCapabilities {
            api_format: ApiFormat::OpenAI, // uses OpenAI-compat
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 1_000_000,
            max_output_tokens: 4096,
        },
        Provider::GoogleGeminiCli => ProviderCapabilities {
            api_format: ApiFormat::GoogleGemini,
            auth_methods: &[AuthType::BrowserOAuth],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 1_000_000,
            max_output_tokens: 4096,
        },
        Provider::Ollama => ProviderCapabilities {
            api_format: ApiFormat::OpenAI, // routed through OpenAI-compat
            auth_methods: &[AuthType::None],
            supports_tools: true, // model-dependent, augmented at runtime
            supports_vision: true, // model-dependent
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: false,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: true, // most Ollama models don't support it
            context_window: 32_768, // overridden per-model via /api/show
            max_output_tokens: 4096,
        },
        Provider::Moonshot => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: true, // Kimi K2.5 only accepts temperature=1
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 262_144,
            max_output_tokens: 4096,
        },
        Provider::Zai => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: false,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: true, // GLM uses /v4 path directly
            skip_parallel_tool_calls: true,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Qwen => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey, AuthType::DeviceCode],
            supports_tools: true,
            supports_vision: true, // model-dependent
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 1_000_000,
            max_output_tokens: 4096,
        },
        Provider::Minimax => ProviderCapabilities {
            api_format: ApiFormat::Anthropic, // uses Anthropic Messages API
            auth_methods: &[AuthType::ApiKey, AuthType::DeviceCode],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 1_000_000,
            max_output_tokens: 4096,
        },
        Provider::OpenRouter => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: true,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Groq => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 131_072,
            max_output_tokens: 4096,
        },
        Provider::Mistral => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Together => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Fireworks => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Azure => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: true,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        Provider::Bedrock => ProviderCapabilities {
            api_format: ApiFormat::Anthropic, // Bedrock uses Anthropic format for Claude models
            auth_methods: &[AuthType::ApiKey], // AWS IAM
            supports_tools: true,
            supports_vision: true,
            supports_thinking: true,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 200_000,
            max_output_tokens: 4096,
        },
        // Providers that route through OpenAI-compat
        Provider::XAI | Provider::Cerebras | Provider::DeepSeek | Provider::XiaomiMimo => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: false,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 128_000,
            max_output_tokens: 4096,
        },
        // Sakana Fugu Ultra — a multi-agent-system-as-a-model (Thinker/Worker/
        // Verifier pool orchestrated server-side via TRINITY + Conductor).
        // OpenAI-compatible surface, but NOT a generic 128k chat model:
        //   - 1,000,000-token context (was mis-modeled as 128k → drove the
        //     history-trimmer into the wall → intermittent `400 prompt too long`)
        //   - 128k max output (via max_output_tokens; other providers cap at 4096)
        //   - reasoning/orchestration model: rejects `temperature` like other
        //     reasoning models → skip_temperature avoids `400 invalid arguments`
        // See docs/Fugu-Research.md for the full root-cause analysis.
        Provider::Sakana => ProviderCapabilities {
            api_format: ApiFormat::OpenAI,
            auth_methods: &[AuthType::ApiKey],
            supports_tools: true,
            // Vision CONFIRMED supported — Sakana's published API surface for
            // fugu-ultra advertises modality `text+image->text` (input
            // modalities: text, image). Verified 2026-06-27 against the
            // first-party model spec. See docs/Fugu-Research.md.
            supports_vision: true,
            supports_thinking: true,
            supports_streaming: true,
            supports_parallel_tools: true,
            supports_audit_logging: false,
            supports_mid_loop_interrupt: false,
            bot_sender_min_iterations: 5,
            skip_temperature: true,
            skip_v1_prefix: false,
            skip_parallel_tool_calls: false,
            context_window: 1_000_000,
            max_output_tokens: 128_000,
        },
    }
}

/// Two-tier vision capability check: provider level first, then model level.
///
/// Returns `true` only if **both** the provider declares vision support **and**
/// the specific model (if known in the catalog) also supports it.
///
/// Returns a `Result<bool, String>`:
/// - `Ok(true)` — provider + model both support vision (or model unknown
///   on a catalog-less provider like Anthropic/OpenAI/Google)
/// - `Ok(false)` — provider doesn't support vision (no error, just skip images)
/// - `Err(msg)` — provider supports vision but this specific model does not
///   (surface this as a clean error to the agent instead of letting the
///   upstream API's raw rejection leak through)
pub fn supports_image_input(provider: &Provider, model: &str) -> Result<bool, String> {
    let provider_caps = capabilities(provider);
    if !provider_caps.supports_vision {
        return Ok(false);
    }

    // Check the bundled model catalog for model-level vision support.
    // The catalog covers Zai (GLM), Qwen, Moonshot, and Minimax models
    // served via Alibaba's DashScope — these are the providers where
    // vision support varies per model on a vision-capable provider.
    if let Some(model_card) = crate::qwen_bundled_catalog().iter().find(|m| m.id == model) {
        if model_card.supports_vision {
            return Ok(true);
        } else {
            return Err(format!(
                "Model {} does not support image input — use a vision-capable variant or switch providers",
                model
            ));
        }
    }

    // GLM model family (Zai provider) does not support vision.
    // The catalog lists "glm-5" and "glm-4.7" but variants like "glm-5.1"
    // won't match — catch them by prefix.
    if provider == &Provider::Zai && model.starts_with("glm") {
        return Err(format!(
            "Model {} does not support image input — GLM models are text-only. Use a vision-capable variant or switch providers",
            model
        ));
    }

    // For providers with their own model catalogs (Ollama) or providers
    // where all models support vision (Anthropic, OpenAI, Google, etc.),
    // trust the provider-level flag when the model isn't in our catalog.
    Ok(true)
}

// ── Dynamic Per-Model Capability Resolver (#44-a) ──────────────────────
//
// `ModelCapabilityResolver` generalizes the per-model dynamic capability
// detection pattern (originally Ollama-only via `/api/show`) into a
// trait-based API. Providers that expose a model-catalog endpoint
// (Ollama `/api/show`, OpenRouter `/api/v1/models`, etc.) implement this
// trait to surface per-model capabilities at runtime.
//
// Static fallback: providers without dynamic catalogs return `None` and
// callers fall back to the static `ProviderCapabilities` table above.
//
// Refactor invariant (#44-a, zero behavior change): Ollama's existing
// `get_cached_model_capabilities` is wrapped in an adapter (`OllamaResolver`
// in `ollama.rs`) that delegates to the free function. #44-b will wire
// OpenRouter + replace the GPT-5.5 string heuristic + add MiMo entry.

/// Per-model capabilities resolved at runtime (typically from a provider's
/// model-catalog endpoint).
///
/// Distinct from `ProviderCapabilities` above:
/// - `ProviderCapabilities` = static, provider-level (e.g. "Anthropic supports tools")
/// - `DynamicModelCapabilities` = runtime, model-level (e.g. "qwen2.5:32b on
///   this Ollama instance supports tools and has 32k context")
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModelCapabilities {
    /// Model accepts image inputs (multimodal / vision)
    pub supports_vision: bool,
    /// Model supports tool/function calling
    pub supports_tools: bool,
    /// Model produces vector embeddings (not chat)
    pub supports_embeddings: bool,
    /// Model accepts a system prompt role
    pub supports_system_prompt: bool,
    /// Native context window length, if discoverable
    pub context_length: Option<usize>,
    /// Model family string (e.g. "llama3.2", "gemma4", "nomic-embed")
    pub family: String,
    /// Thinking-mode forces `temperature=1.0` (e.g. MiMo, o-series).
    /// When `Some(true)`, callers should pin temperature to 1.0 regardless
    /// of user-supplied value. `None` means "not applicable / unknown".
    pub thinking_mode_temperature_lock: Option<bool>,
}

impl Default for DynamicModelCapabilities {
    fn default() -> Self {
        Self {
            supports_vision: false,
            supports_tools: false,
            supports_embeddings: false,
            supports_system_prompt: true,
            context_length: None,
            family: String::new(),
            thinking_mode_temperature_lock: None,
        }
    }
}

/// Trait for providers that resolve per-model capabilities dynamically.
///
/// Implementations are expected to be cache-backed (avoid re-querying the
/// provider's catalog endpoint on every call). The Ollama implementation
/// (`OllamaResolver` in `ollama.rs`) wraps the process-global
/// `CAPABILITIES_CACHE`.
#[async_trait::async_trait]
pub trait ModelCapabilityResolver: Send + Sync {
    /// Resolve capabilities for `model` on this provider.
    ///
    /// Returns `None` if:
    /// - The model is unknown to the provider's catalog, OR
    /// - The catalog query failed (network/parse error)
    ///
    /// Callers should fall back to the static `ProviderCapabilities` table
    /// when this returns `None`.
    async fn resolve(&self, model: &str) -> Option<DynamicModelCapabilities>;
}

// ── Static-authoritative resolvers (#44 / #44-doc) ─────────────────────
//
// For Anthropic, OpenAI, Google, and Groq the static `ProviderCapabilities`
// table is AUTHORITATIVE — `resolve()` returns `None` by design, instructing
// callers to use the static table. This is NOT a gap or an unfinished stub.
//
// Dynamic per-model capability resolution is an OpenRouter-specific
// optimization: OpenRouter brokers hundreds of third-party models whose
// per-model capabilities vary and shift, and it exposes a queryable
// `/models` catalog that makes runtime resolution worthwhile. The four
// providers below expose no equivalently useful queryable per-model
// capability endpoint, and their model rosters are small and stable enough
// that the static table covers them correctly. Fleshing out `/models`
// fetchers for them would add network calls and parsing surface with no
// behavior benefit, so we deliberately return `None`.
//
// The dynamic-catalog cases are `OllamaResolver` (in `ollama.rs`) and
// `OpenRouterResolver` (in `lib.rs`, added in #44-b Step 2).

pub struct AnthropicResolver;

#[async_trait::async_trait]
impl ModelCapabilityResolver for AnthropicResolver {
    /// Static capabilities are authoritative for Anthropic; dynamic per-model
    /// resolution is an OpenRouter-specific optimization (provider exposes no
    /// queryable per-model capability endpoint). Returning `None` = use static
    /// table. Not a gap.
    async fn resolve(&self, _model: &str) -> Option<DynamicModelCapabilities> {
        None
    }
}

pub struct OpenAIResolver;

#[async_trait::async_trait]
impl ModelCapabilityResolver for OpenAIResolver {
    /// Static capabilities are authoritative for OpenAI; dynamic per-model
    /// resolution is an OpenRouter-specific optimization (provider exposes no
    /// queryable per-model capability endpoint). Returning `None` = use static
    /// table. Not a gap.
    async fn resolve(&self, _model: &str) -> Option<DynamicModelCapabilities> {
        None
    }
}

pub struct GoogleResolver;

#[async_trait::async_trait]
impl ModelCapabilityResolver for GoogleResolver {
    /// Static capabilities are authoritative for Google; dynamic per-model
    /// resolution is an OpenRouter-specific optimization (provider exposes no
    /// queryable per-model capability endpoint). Returning `None` = use static
    /// table. Not a gap.
    async fn resolve(&self, _model: &str) -> Option<DynamicModelCapabilities> {
        None
    }
}

pub struct GroqResolver;

#[async_trait::async_trait]
impl ModelCapabilityResolver for GroqResolver {
    /// Static capabilities are authoritative for Groq; dynamic per-model
    /// resolution is an OpenRouter-specific optimization (provider exposes no
    /// queryable per-model capability endpoint). Returning `None` = use static
    /// table. Not a gap.
    async fn resolve(&self, _model: &str) -> Option<DynamicModelCapabilities> {
        None
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_capabilities() {
        let caps = capabilities(&Provider::Anthropic);
        assert_eq!(caps.api_format, ApiFormat::Anthropic);
        assert!(caps.supports_tools);
        assert!(caps.supports_vision);
        assert!(caps.supports_thinking);
        assert!(!caps.skip_temperature);
    }

    #[test]
    fn test_moonshot_skip_temperature() {
        let caps = capabilities(&Provider::Moonshot);
        assert!(caps.skip_temperature);
        assert_eq!(caps.api_format, ApiFormat::OpenAI);
    }

    #[test]
    fn test_sakana_fugu_capabilities() {
        // Fugu Ultra must be its own arm — NOT the generic 128k OpenAI default.
        // The 1M context window is what stops the `400 prompt too long` (the
        // history-trimmer calibrates off context_window), and skip_temperature
        // stops the `400 invalid arguments`.
        let caps = capabilities(&Provider::Sakana);
        assert_eq!(caps.context_window, 1_000_000, "Fugu has a 1M context window");
        assert_eq!(caps.max_output_tokens, 128_000, "Fugu supports 128k output");
        assert!(caps.skip_temperature, "Fugu rejects the temperature param");
        assert!(caps.supports_thinking, "Fugu is an orchestration/reasoning model");
        assert!(caps.supports_vision, "Fugu vision confirmed (text+image->text)");
        assert_eq!(caps.api_format, ApiFormat::OpenAI);
    }

    #[test]
    fn test_default_providers_keep_4096_output() {
        // The per-provider output cap must not regress others to 128k —
        // only Sakana overrides the conservative default.
        assert_eq!(capabilities(&Provider::OpenAI).max_output_tokens, 4096);
        assert_eq!(capabilities(&Provider::Anthropic).max_output_tokens, 4096);
        assert_eq!(capabilities(&Provider::XAI).max_output_tokens, 4096);
    }

    #[test]
    fn test_zai_skip_v1_prefix() {
        let caps = capabilities(&Provider::Zai);
        assert!(caps.skip_v1_prefix);
        assert!(caps.skip_parallel_tool_calls);
    }

    #[test]
    fn test_ollama_no_auth() {
        let caps = capabilities(&Provider::Ollama);
        assert_eq!(caps.auth_methods, &[AuthType::None]);
        assert!(caps.skip_parallel_tool_calls);
    }

    #[test]
    fn test_minimax_anthropic_format() {
        let caps = capabilities(&Provider::Minimax);
        assert_eq!(caps.api_format, ApiFormat::Anthropic);
    }

    #[test]
    fn test_all_providers_have_capabilities() {
        // Verify every provider variant returns without panic
        let providers = [
            Provider::Anthropic, Provider::OpenAI, Provider::Google,
            Provider::GoogleGeminiCli, Provider::Ollama, Provider::Moonshot,
            Provider::Zai, Provider::Qwen, Provider::Minimax,
            Provider::OpenRouter, Provider::Groq, Provider::Mistral,
            Provider::Together, Provider::Fireworks, Provider::Azure,
            Provider::Bedrock, Provider::XAI, Provider::Cerebras,
            Provider::DeepSeek,
        ];
        for p in &providers {
            let caps = capabilities(p);
            assert!(caps.supports_streaming, "All providers should support streaming");
        }
    }

    // ── Two-tier vision tests ──────────────────────────────────────

    #[test]
    fn test_vision_provider_no_vision_model() {
        // Zai provider supports vision, but glm-5 does not
        let result = supports_image_input(&Provider::Zai, "glm-5");
        assert!(result.is_err(), "glm-5 should be rejected for image input");
        assert!(
            result.unwrap_err().contains("does not support image input"),
            "error should mention image input"
        );
    }

    #[test]
    fn test_vision_provider_vision_model() {
        // Zai provider supports vision, and qwen3.5-plus also supports vision
        let result = supports_image_input(&Provider::Zai, "qwen3.5-plus");
        assert!(result.is_ok(), "qwen3.5-plus should be accepted for image input");
        assert!(result.unwrap(), "qwen3.5-plus should return true");
    }

    #[test]
    fn test_no_vision_provider() {
        // Currently all providers declare supports_vision: true at the provider level,
        // so the Ok(false) path is unreachable in practice. If a provider is added
        // with supports_vision: false, this test should be updated to test it.
        // For now, verify that a provider with vision=true returns Ok(true) for
        // an unknown model not in any catalog.
        let result = supports_image_input(&Provider::OpenAI, "gpt-5-turbo");
        assert!(result.is_ok(), "OpenAI unknown model should return Ok");
        assert!(result.unwrap(), "OpenAI supports vision for all models");
    }

    #[test]
    fn test_unknown_model_on_catalog_provider() {
        // Model not in catalog on a catalog-based provider (Zai) —
        // non-GLM models trust provider-level flag
        let result = supports_image_input(&Provider::Zai, "some-future-model");
        assert!(result.is_ok(), "unknown non-GLM model on Zai should defer to provider flag");
        assert!(result.unwrap(), "Zai provider supports vision, so unknown non-GLM model should be allowed");
    }

    #[test]
    fn test_glm_variant_rejected() {
        // glm-5.1 is not in the catalog but should be caught by prefix match
        let result = supports_image_input(&Provider::Zai, "glm-5.1");
        assert!(result.is_err(), "glm-5.1 should be rejected for image input");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("does not support image input"),
            "error should mention image input, got: {err_msg}"
        );
        assert!(
            err_msg.contains("text-only"),
            "error should mention GLM is text-only, got: {err_msg}"
        );
    }

    #[test]
    fn test_anthropic_vision() {
        // Anthropic is a vision-capable provider with no catalog entries
        // (not in qwen_bundled_catalog), so trust provider-level flag
        let result = supports_image_input(&Provider::Anthropic, "claude-sonnet-4");
        assert!(result.is_ok(), "Anthropic models not in catalog should trust provider flag");
        assert!(result.unwrap(), "Anthropic supports vision");
    }

    #[test]
    fn test_ollama_deferred() {
        // Ollama has its own per-model detection; unknown models are allowed
        let result = supports_image_input(&Provider::Ollama, "llava:13b");
        assert!(result.is_ok(), "Ollama should defer to provider-level flag");
        assert!(result.unwrap(), "Ollama should return true for unknown models");
    }
}
