//! Per-request cost calculation for LLM calls.
//!
//! Provides a pricing table keyed by (provider, model) and utilities to
//! compute the actual dollar cost of a call based on token usage.
//!
//! Pricing is expressed in **USD per million tokens** (the standard unit
//! used by major providers). Input, output, and cached-input rates are
//! tracked separately since providers charge different rates for each.
//!
//! # Example
//!
//! ```
//! use zeus_llm::cost::{CostBreakdown, calculate_cost};
//!
//! let cost = calculate_cost("anthropic", "claude-opus-4-7", 1_000, 500, 0);
//! assert!(cost.is_some());
//! let c = cost.unwrap();
//! assert!(c.total_usd > 0.0);
//! ```

use serde::{Deserialize, Serialize};

/// Pricing entry: USD per 1M tokens for each token category.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    /// USD per 1M input (prompt) tokens.
    pub input_per_mtok: f64,
    /// USD per 1M output (completion) tokens.
    pub output_per_mtok: f64,
    /// USD per 1M cached input tokens (Anthropic prompt cache read rate).
    /// Falls back to `input_per_mtok` if the provider doesn't discount cache hits.
    pub cached_input_per_mtok: f64,
}

impl ModelPricing {
    pub const fn new(input: f64, output: f64, cached: f64) -> Self {
        Self {
            input_per_mtok: input,
            output_per_mtok: output,
            cached_input_per_mtok: cached,
        }
    }

    /// Convenience: provider doesn't discount cached tokens — cached rate = input rate.
    pub const fn flat(input: f64, output: f64) -> Self {
        Self {
            input_per_mtok: input,
            output_per_mtok: output,
            cached_input_per_mtok: input,
        }
    }
}

/// Itemized cost breakdown for a single LLM request.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub input_usd: f64,
    pub output_usd: f64,
    pub cached_input_usd: f64,
    pub total_usd: f64,
}

impl CostBreakdown {
    pub fn zero() -> Self {
        Self {
            input_usd: 0.0,
            output_usd: 0.0,
            cached_input_usd: 0.0,
            total_usd: 0.0,
        }
    }
}

/// Look up pricing for a (provider, model) pair.
///
/// Matching is case-insensitive and tolerant of common variants
/// (e.g. `claude-opus-4-7`, `claude-opus-4.7`, `anthropic/claude-opus-4-7`).
/// Returns `None` for unknown models — callers should treat that as
/// "cost unavailable", not an error.
pub fn lookup_pricing(provider: &str, model: &str) -> Option<ModelPricing> {
    let p = provider.to_ascii_lowercase();
    let m = normalize_model(model);

    match p.as_str() {
        "anthropic" | "claude" => anthropic_pricing(&m),
        "openai" => openai_pricing(&m),
        "openrouter" => openrouter_pricing(&m),
        "ollama" | "local" => Some(ModelPricing::flat(0.0, 0.0)),
        _ => None,
    }
}

/// Calculate the dollar cost of a single LLM call.
///
/// Returns `None` if the (provider, model) pair is unknown.
/// Token counts are in whole tokens — conversion to per-1M rates is internal.
///
/// Note: `input_tokens` is assumed to be the *total* prompt tokens including
/// any cached portion. We subtract `cached_tokens` to avoid double-counting.
pub fn calculate_cost(
    provider: &str,
    model: &str,
    input_tokens: usize,
    output_tokens: usize,
    cached_tokens: usize,
) -> Option<CostBreakdown> {
    let pricing = lookup_pricing(provider, model)?;
    Some(calculate_with_pricing(
        &pricing,
        input_tokens,
        output_tokens,
        cached_tokens,
    ))
}

/// Calculate cost when you already have a pricing struct in hand.
pub fn calculate_with_pricing(
    pricing: &ModelPricing,
    input_tokens: usize,
    output_tokens: usize,
    cached_tokens: usize,
) -> CostBreakdown {
    // Fresh input = total input minus the cached portion.
    let fresh_input = input_tokens.saturating_sub(cached_tokens);

    let per_mtok = 1_000_000.0;
    let input_usd = (fresh_input as f64 / per_mtok) * pricing.input_per_mtok;
    let output_usd = (output_tokens as f64 / per_mtok) * pricing.output_per_mtok;
    let cached_input_usd =
        (cached_tokens as f64 / per_mtok) * pricing.cached_input_per_mtok;

    CostBreakdown {
        input_usd,
        output_usd,
        cached_input_usd,
        total_usd: input_usd + output_usd + cached_input_usd,
    }
}

/// Normalize a model identifier: lowercase, strip vendor prefix, unify separators.
fn normalize_model(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    // Strip "anthropic/", "openai/", "openrouter/" prefixes.
    let stripped = lower
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(&lower);
    // Treat '.' and '_' as '-' for fuzzy matching.
    stripped.replace(['.', '_'], "-")
}

// ── Provider pricing tables ────────────────────────────────────────────────
// All prices in USD per 1M tokens. Last updated 2025-04 from public pricing
// pages. Keep this table small and well-labeled; add entries as new models
// are adopted.

fn anthropic_pricing(model: &str) -> Option<ModelPricing> {
    // Cached input rate for Anthropic = 10% of input rate (prompt cache reads).
    Some(match model {
        // Claude 4 family
        m if m.contains("opus-4-7") || m.contains("opus-4.7") => {
            ModelPricing::new(15.00, 75.00, 1.50)
        }
        m if m.contains("opus-4") => ModelPricing::new(15.00, 75.00, 1.50),
        m if m.contains("sonnet-4-7") || m.contains("sonnet-4.7") => {
            ModelPricing::new(3.00, 15.00, 0.30)
        }
        m if m.contains("sonnet-4") => ModelPricing::new(3.00, 15.00, 0.30),
        m if m.contains("haiku-4") => ModelPricing::new(1.00, 5.00, 0.10),
        // Claude 3.5 family
        m if m.contains("3-5-sonnet") || m.contains("3.5-sonnet") => {
            ModelPricing::new(3.00, 15.00, 0.30)
        }
        m if m.contains("3-5-haiku") || m.contains("3.5-haiku") => {
            ModelPricing::new(0.80, 4.00, 0.08)
        }
        // Claude 3 family (legacy)
        m if m.contains("3-opus") => ModelPricing::new(15.00, 75.00, 1.50),
        m if m.contains("3-sonnet") => ModelPricing::new(3.00, 15.00, 0.30),
        m if m.contains("3-haiku") => ModelPricing::new(0.25, 1.25, 0.03),
        _ => return None,
    })
}

fn openai_pricing(model: &str) -> Option<ModelPricing> {
    Some(match model {
        // GPT-4o family
        m if m.contains("gpt-4o-mini") => ModelPricing::new(0.15, 0.60, 0.075),
        m if m.contains("gpt-4o") => ModelPricing::new(2.50, 10.00, 1.25),
        // o-series
        m if m.contains("o1-mini") => ModelPricing::new(3.00, 12.00, 1.50),
        m if m.contains("o1") => ModelPricing::new(15.00, 60.00, 7.50),
        m if m.contains("o3-mini") => ModelPricing::new(1.10, 4.40, 0.55),
        m if m.contains("o3") => ModelPricing::new(2.00, 8.00, 0.50),
        // GPT-4 Turbo / GPT-4
        m if m.contains("gpt-4-turbo") => ModelPricing::flat(10.00, 30.00),
        m if m.contains("gpt-4") => ModelPricing::flat(30.00, 60.00),
        // GPT-3.5
        m if m.contains("gpt-3-5") || m.contains("gpt-3.5") => {
            ModelPricing::flat(0.50, 1.50)
        }
        _ => return None,
    })
}

fn openrouter_pricing(model: &str) -> Option<ModelPricing> {
    // OpenRouter passes through upstream pricing. For routed models we look
    // up the underlying provider's price. Callers passing `openrouter` with
    // a model like `anthropic/claude-opus-4-7` should hit this path.
    if let Some(p) = anthropic_pricing(model) {
        return Some(p);
    }
    if let Some(p) = openai_pricing(model) {
        return Some(p);
    }
    // Known OpenRouter-native / other-provider models.
    Some(match model {
        m if m.contains("deepseek-v3") => ModelPricing::flat(0.27, 1.10),
        m if m.contains("deepseek-r1") => ModelPricing::flat(0.55, 2.19),
        m if m.contains("llama-3-3-70b") || m.contains("llama-3.3-70b") => {
            ModelPricing::flat(0.12, 0.30)
        }
        m if m.contains("qwen-2-5-72b") || m.contains("qwen-2.5-72b") => {
            ModelPricing::flat(0.35, 0.40)
        }
        m if m.contains("mistral-large") => ModelPricing::flat(2.00, 6.00),
        m if m.contains("gemini-2-0-flash") || m.contains("gemini-2.0-flash") => {
            ModelPricing::flat(0.10, 0.40)
        }
        m if m.contains("gemini-1-5-pro") || m.contains("gemini-1.5-pro") => {
            ModelPricing::flat(1.25, 5.00)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_opus_lookup() {
        let p = lookup_pricing("anthropic", "claude-opus-4-7").unwrap();
        assert_eq!(p.input_per_mtok, 15.00);
        assert_eq!(p.output_per_mtok, 75.00);
        assert_eq!(p.cached_input_per_mtok, 1.50);
    }

    #[test]
    fn model_normalization_handles_dots_and_prefix() {
        // openrouter path stripping prefix + dot→dash normalization
        let p = lookup_pricing("openrouter", "anthropic/claude-opus-4.7").unwrap();
        assert_eq!(p.input_per_mtok, 15.00);
    }

    #[test]
    fn case_insensitive_provider() {
        let p = lookup_pricing("Anthropic", "claude-3-5-sonnet").unwrap();
        assert_eq!(p.output_per_mtok, 15.00);
    }

    #[test]
    fn calculate_cost_basic() {
        // 1M input + 1M output of opus-4-7 = $15 + $75 = $90
        let c = calculate_cost("anthropic", "claude-opus-4-7", 1_000_000, 1_000_000, 0)
            .unwrap();
        assert!((c.input_usd - 15.0).abs() < 1e-9);
        assert!((c.output_usd - 75.0).abs() < 1e-9);
        assert!((c.total_usd - 90.0).abs() < 1e-9);
    }

    #[test]
    fn cached_tokens_discounted() {
        // 1M input total, 500k cached, 0 output:
        // 500k fresh @ $15/M = $7.50
        // 500k cached @ $1.50/M = $0.75
        // total = $8.25
        let c = calculate_cost("anthropic", "claude-opus-4-7", 1_000_000, 0, 500_000)
            .unwrap();
        assert!((c.input_usd - 7.50).abs() < 1e-9);
        assert!((c.cached_input_usd - 0.75).abs() < 1e-9);
        assert!((c.total_usd - 8.25).abs() < 1e-9);
    }

    #[test]
    fn ollama_is_free() {
        let c = calculate_cost("ollama", "llama3", 1_000_000, 1_000_000, 0).unwrap();
        assert_eq!(c.total_usd, 0.0);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(calculate_cost("anthropic", "claude-banana-99", 100, 100, 0).is_none());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(lookup_pricing("mystery-corp", "some-model").is_none());
    }

    #[test]
    fn openai_gpt4o_mini() {
        let c =
            calculate_cost("openai", "gpt-4o-mini", 1_000_000, 1_000_000, 0).unwrap();
        assert!((c.input_usd - 0.15).abs() < 1e-9);
        assert!((c.output_usd - 0.60).abs() < 1e-9);
    }

    #[test]
    fn openrouter_deepseek() {
        let c = calculate_cost(
            "openrouter",
            "deepseek/deepseek-v3",
            1_000_000,
            1_000_000,
            0,
        )
        .unwrap();
        assert!((c.total_usd - (0.27 + 1.10)).abs() < 1e-9);
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let c = calculate_cost("anthropic", "claude-opus-4-7", 0, 0, 0).unwrap();
        assert_eq!(c.total_usd, 0.0);
    }
}
