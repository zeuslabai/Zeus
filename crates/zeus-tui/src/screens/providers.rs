//! Canonical LLM provider registry — the ONE shared source of truth.
//!
//! Consumed by the Provider (03), Model (05), and Fallback (06) onboarding
//! screens so the provider list is defined exactly once. Previously each screen
//! carried its own duplicated `PROVIDERS` / `FALLBACK_CANDIDATES` array which
//! drifted out of sync (stale flagships, divergent membership). This module
//! collapses them into a single `PROVIDERS` const.
//!
//! Membership = our 12 supported providers (Grok/xAI in, Groq/Mistral/Together/
//! Fireworks/DeepSeek/Azure out).

use ratatui::style::Color;

use crate::theme;

/// A single provider's canonical metadata. Superset of every field any consumer
/// screen needs — Provider screen reads all of it; Fallback reads a subset
/// (id/name/glyph/color/flagship); Model keys its catalog off `id`.
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub glyph: &'static str,
    pub color: Color,
    pub sub: &'static str,
    pub flagship: &'static str,
    pub price: &'static str,
    pub key_fmt: &'static str,
    /// `★ FEATURED` badge on the Provider screen.
    pub featured: bool,
}

/// The canonical 16. Order = display order on the Provider 3-col grid.
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "anthropic",
        name: "Anthropic",
        glyph: "ANT",
        color: theme::FIRE_ORANGE,
        sub: "Deep reasoning, code, long context",
        flagship: "claude-opus-4-8",
        price: "$15/$75 per Mtok",
        key_fmt: "sk-ant-...",
        featured: true,
    },
    ProviderInfo {
        id: "openai",
        name: "OpenAI",
        glyph: "OAI",
        color: theme::GREEN,
        sub: "Broad multimodal capability",
        flagship: "gpt-4o",
        price: "$2.50/$10 per Mtok",
        key_fmt: "sk-...",
        featured: false,
    },
    ProviderInfo {
        id: "google",
        name: "Google",
        glyph: "GCP",
        color: theme::BLUE,
        sub: "Speed, multimodal workflows",
        flagship: "gemini-2.5-pro",
        price: "$1.25/$10 per Mtok",
        key_fmt: "AIza...",
        featured: false,
    },
    ProviderInfo {
        id: "ollama",
        name: "Ollama",
        glyph: "OLM",
        color: theme::CYAN,
        sub: "Local models, zero cost, private",
        flagship: "Local model server",
        price: "Free / self-hosted",
        key_fmt: "(none)",
        featured: false,
    },
    ProviderInfo {
        id: "gemini-cli",
        name: "Gemini CLI",
        glyph: "GCL",
        color: theme::BLUE,
        sub: "OAuth via google-gemini/gemini-cli",
        flagship: "gemini-2.5-pro",
        price: "OAuth (no key)",
        key_fmt: "OAuth",
        featured: false,
    },
    ProviderInfo {
        id: "kimi",
        name: "Kimi",
        glyph: "KMI",
        color: theme::PURPLE,
        sub: "Moonshot — agentic, long context",
        flagship: "kimi-k2.7-code",
        price: "$0.60/$2.50 per Mtok",
        key_fmt: "sk-...",
        featured: false,
    },
    ProviderInfo {
        id: "kimi-code",
        name: "Kimi Code",
        glyph: "K3",
        color: theme::PURPLE,
        sub: "Kimi subscription — K3 + coding",
        flagship: "k3",
        price: "Subscription",
        key_fmt: "sk-...",
        featured: false,
    },
    ProviderInfo {
        id: "glm",
        name: "GLM",
        glyph: "GLM",
        color: theme::TEAL,
        sub: "Zhipu — strong code + reasoning",
        flagship: "glm-5.2",
        price: "$0.60/$2.20 per Mtok",
        key_fmt: "...",
        featured: false,
    },
    ProviderInfo {
        id: "glm-coding",
        name: "GLM Coding",
        glyph: "GLC",
        color: theme::TEAL,
        sub: "GLM Coding subscription — flat-rate",
        flagship: "glm-5.2",
        price: "Subscription",
        key_fmt: "...",
        featured: false,
    },
    ProviderInfo {
        id: "qwen",
        name: "Qwen",
        glyph: "QWN",
        color: theme::PURPLE,
        sub: "Alibaba — multilingual, agentic",
        flagship: "qwen3-max",
        price: "$0.40/$1.20 per Mtok",
        key_fmt: "sk-...",
        featured: false,
    },
    ProviderInfo {
        id: "minimax",
        name: "MiniMax",
        glyph: "MNX",
        color: theme::AMBER,
        sub: "High-throughput, multilingual, agentic",
        flagship: "MiniMax-M3",
        price: "$0.20/$0.80 per Mtok",
        // MiniMax: keys start with `sk-api-` prefix.
        key_fmt: "sk-api-...",
        featured: false,
    },
    ProviderInfo {
        id: "minimax-coding",
        name: "MiniMax Coding",
        glyph: "MNC",
        color: theme::AMBER,
        sub: "MiniMax subscription — Token Plan",
        flagship: "MiniMax-M3",
        price: "Subscription",
        key_fmt: "sk-api-...",
        featured: false,
    },
    ProviderInfo {
        id: "mimo",
        name: "MiMo",
        glyph: "MMO",
        color: theme::YELLOW,
        sub: "Xiaomi — agentic, long context",
        flagship: "mimo-v2.5-pro",
        price: "$0.30/$1.20 per Mtok",
        key_fmt: "...",
        featured: false,
    },
    ProviderInfo {
        id: "openrouter",
        name: "OpenRouter",
        glyph: "ORT",
        color: theme::GREEN,
        sub: "100+ models, cost optimization",
        flagship: "auto",
        price: "Varies by model",
        key_fmt: "sk-or-...",
        featured: false,
    },
    ProviderInfo {
        id: "xai",
        name: "xAI",
        glyph: "XAI",
        color: theme::WHITE,
        sub: "Grok models, real-time data",
        flagship: "grok-4",
        price: "$3/$15 per Mtok",
        key_fmt: "xai-...",
        featured: false,
    },
    ProviderInfo {
        id: "sakana",
        name: "Sakana Fugu",
        glyph: "FUG",
        color: theme::CYAN,
        sub: "Sakana AI — efficient, low-cost",
        flagship: "fugu-ultra",
        price: "Low-cost",
        // Real Sakana keys start with `fish_` (live-verified, #268). The #240
        // validator (app.rs prefix check) gates advance on this format, so an
        // `sk-...` hint would hard-block onboarding for valid Sakana keys.
        key_fmt: "fish_...",
        featured: false,
    },
];

/// Id of the provider at `idx` (clamped).
pub fn id_at(idx: usize) -> &'static str {
    PROVIDERS[idx.min(PROVIDERS.len() - 1)].id
}

/// Display fields (name, accent color, key format) for the provider at `idx`
/// (clamped) — drives the Auth screen for whichever provider was selected.
pub fn display(idx: usize) -> (&'static str, Color, &'static str) {
    let p = &PROVIDERS[idx.min(PROVIDERS.len() - 1)];
    (p.name, p.color, p.key_fmt)
}

/// Lookup by id.
pub fn by_id(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}
