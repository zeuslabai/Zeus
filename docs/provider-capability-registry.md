# Provider Capability Registry — Design Doc

**Status:** Approved for next sprint
**Author:** Zeus112
**Date:** 2026-04-14

## Problem

Provider-specific quirks are scattered across `crates/zeus-llm/src/lib.rs` as inline `if provider == X` checks. Adding a new provider means touching 10+ code paths. No single source of truth for what a provider supports.

### Current scattered quirks (examples)

- **Moonshot:** skip temperature (only accepts 1.0), disable thinking mode
- **ZAI (GLM):** skip `/v1` path prefix, skip `parallel_tool_calls`
- **Ollama:** skip auth, OpenAI-compat endpoint, local context window query
- **MiniMax:** Anthropic Messages API (not OpenAI), device code OAuth
- **Qwen:** portal vs DashScope endpoint based on auth type

## Proposed Schema

```rust
// Location: crates/zeus-llm/src/capabilities.rs

pub struct ProviderCapabilities {
    // API compatibility
    pub api_format: ApiFormat,        // OpenAI | Anthropic | Custom
    pub auth_methods: Vec<AuthType>,  // ApiKey, OAuth, DeviceCode, None
    pub base_url: String,
    pub completions_path: String,     // "/v1/chat/completions" or "/v1/messages"

    // Model capabilities
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_thinking: bool,      // extended thinking / reasoning
    pub supports_streaming: bool,
    pub supports_parallel_tools: bool,

    // Quirks
    pub skip_temperature: bool,       // Moonshot
    pub fixed_temperature: Option<f64>,
    pub max_tools: Option<usize>,     // tool count cap
    pub context_window: usize,        // default context size
    pub skip_v1_prefix: bool,         // ZAI
}

pub enum ApiFormat { OpenAI, Anthropic, GoogleGemini, Custom }
pub enum AuthType { ApiKey, OAuth, DeviceCode, BrowserOAuth, None }
```

## Provider Matrix

| Provider | API Format | Auth | Tools | Vision | Thinking | Parallel | Quirks |
|----------|-----------|------|-------|--------|----------|----------|--------|
| Anthropic | Anthropic | ApiKey/OAuth | Y | Y | Y | Y | thinking_level |
| OpenAI | OpenAI | ApiKey/OAuth | Y | Y | Y (o-series) | Y | reasoning models |
| Google | OpenAI-compat | ApiKey | Y | Y | N | Y | -- |
| Gemini CLI | Custom | BrowserOAuth | Y | Y | N | Y | CAGenerateContent envelope |
| Ollama | OpenAI-compat | None | Y* | Y* | N | N | *model-dependent, runtime query |
| Moonshot | OpenAI | ApiKey | Y | Y | N | Y | temp=1 only, no thinking |
| ZAI | OpenAI | ApiKey | Y | N | N | N | skip /v1 prefix |
| Qwen | OpenAI | ApiKey/DeviceCode | Y | Y* | N | Y | portal vs DashScope |
| MiniMax | Anthropic | ApiKey/DeviceCode | Y | N | N | Y | separate inference URL |
| OpenRouter | OpenAI | ApiKey | Y | Y | Y | Y | inherits from underlying provider |
| Together | OpenAI | ApiKey | Y | Y | N | Y | -- |
| Fireworks | OpenAI | ApiKey | Y | Y | N | Y | -- |
| Azure | OpenAI | ApiKey | Y | Y | Y | Y | custom endpoint + deployment |
| Bedrock | Custom | AWS IAM | Y | Y | Y | Y | SigV4 signing, region-based |
| Groq | OpenAI | ApiKey | Y | N | N | Y | fast inference |
| Mistral | OpenAI | ApiKey | Y | Y | N | Y | -- |

## Design Decisions

### Location
`crates/zeus-llm/src/capabilities.rs` — LLM-specific, `zeus-core` shouldn't know about provider quirks.

### Static + Dynamic Overlay
Static defaults hardcoded per provider (the matrix above). For Ollama, augmented at runtime via `/api/show` (existing `ModelCapabilities`). Dynamic fetch for cloud providers is unnecessary — capabilities don't change per-session.

### OpenRouter Inheritance
OpenRouter model strings are `provider/model` — extract the provider prefix and look up that provider's capabilities. If unknown, default to OpenAI-compat with all features enabled (safe superset).

## Integration Points

Where the registry replaces inline checks:

1. **`completions_url()`** — base URL + path construction (currently hardcoded per provider)
2. **`complete_openai()` / `stream_openai()`** — temperature skip, parallel_tools skip
3. **`complete()` / `stream()`** — provider dispatch routing (the big match block)
4. **Smart tool loading** — tool count cap per provider
5. **Auth resolution** — which auth methods to try per provider
6. **Intent classifier** — adjust complexity threshold per provider capability

## Implementation Plan

1. Add `ProviderCapabilities` struct + `ApiFormat`/`AuthType` enums to `zeus-llm`
2. Add `fn capabilities(provider: Provider) -> ProviderCapabilities` static lookup
3. Replace inline `if provider == X` checks with capability lookups (~150 lines)
4. For Ollama: merge with existing `ModelCapabilities` (runtime augments static)
5. Add tests: one per provider verifying correct capability set

## Estimated Scope

- ~200 lines for the registry (struct + lookup function + tests)
- ~150 lines of inline check replacements across `lib.rs`
- One sprint
