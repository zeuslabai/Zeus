//! Cost estimation for session token usage
//!
//! Maps model names to per-token pricing and calculates session costs.
//! Pricing data is approximate as of early 2026.

use serde::{Deserialize, Serialize};

/// Per-model pricing entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Model name pattern (matched as substring)
    pub pattern: String,
    /// Cost per 1,000 input tokens (USD)
    pub input_cost_per_1k: f64,
    /// Cost per 1,000 output tokens (USD)
    pub output_cost_per_1k: f64,
    /// Display name for the model
    pub display_name: String,
}

/// Estimated cost for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCost {
    /// Model used for the estimate
    pub model: String,
    /// Input token count
    pub input_tokens: usize,
    /// Output token count
    pub output_tokens: usize,
    /// Cost of input tokens (USD)
    pub input_cost: f64,
    /// Cost of output tokens (USD)
    pub output_cost: f64,
    /// Total cost (USD)
    pub total_cost: f64,
}

/// Cost estimator with configurable pricing table.
pub struct CostEstimator {
    pricing: Vec<ModelPricing>,
}

impl CostEstimator {
    /// Create a new estimator with the given pricing table.
    pub fn new(pricing: Vec<ModelPricing>) -> Self {
        Self { pricing }
    }

    /// Create an estimator with the default pricing table.
    pub fn with_defaults() -> Self {
        Self::new(default_pricing_table())
    }

    /// Look up pricing for a model string.
    ///
    /// Matches by substring against the model patterns, returning the first match.
    pub fn find_pricing(&self, model: &str) -> Option<&ModelPricing> {
        let lower = model.to_lowercase();
        self.pricing.iter().find(|p| lower.contains(&p.pattern))
    }

    /// Estimate cost for given token counts and model.
    pub fn estimate(&self, input_tokens: usize, output_tokens: usize, model: &str) -> SessionCost {
        let (input_rate, output_rate) = self
            .find_pricing(model)
            .map(|p| (p.input_cost_per_1k, p.output_cost_per_1k))
            .unwrap_or((0.003, 0.015)); // Default to Sonnet-class pricing

        let input_cost = (input_tokens as f64 / 1000.0) * input_rate;
        let output_cost = (output_tokens as f64 / 1000.0) * output_rate;

        SessionCost {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            input_cost,
            output_cost,
            total_cost: input_cost + output_cost,
        }
    }

    /// Estimate cost from a `SessionTokenUsage`.
    pub fn estimate_from_usage(
        &self,
        usage: &super::token_counter::SessionTokenUsage,
        model: &str,
    ) -> SessionCost {
        self.estimate(usage.input_tokens, usage.output_tokens, model)
    }

    /// Get the full pricing table.
    pub fn pricing(&self) -> &[ModelPricing] {
        &self.pricing
    }
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Default pricing table covering major providers (approximate, early 2026).
pub fn default_pricing_table() -> Vec<ModelPricing> {
    vec![
        // Anthropic
        ModelPricing {
            pattern: "claude-opus".into(),
            input_cost_per_1k: 0.015,
            output_cost_per_1k: 0.075,
            display_name: "Claude Opus".into(),
        },
        ModelPricing {
            pattern: "claude-sonnet".into(),
            input_cost_per_1k: 0.003,
            output_cost_per_1k: 0.015,
            display_name: "Claude Sonnet".into(),
        },
        ModelPricing {
            pattern: "claude-haiku".into(),
            input_cost_per_1k: 0.00025,
            output_cost_per_1k: 0.00125,
            display_name: "Claude Haiku".into(),
        },
        // OpenAI
        ModelPricing {
            pattern: "gpt-4o-mini".into(),
            input_cost_per_1k: 0.00015,
            output_cost_per_1k: 0.0006,
            display_name: "GPT-4o Mini".into(),
        },
        ModelPricing {
            pattern: "gpt-4o".into(),
            input_cost_per_1k: 0.0025,
            output_cost_per_1k: 0.01,
            display_name: "GPT-4o".into(),
        },
        ModelPricing {
            pattern: "gpt-4".into(),
            input_cost_per_1k: 0.03,
            output_cost_per_1k: 0.06,
            display_name: "GPT-4".into(),
        },
        ModelPricing {
            pattern: "o1-mini".into(),
            input_cost_per_1k: 0.003,
            output_cost_per_1k: 0.012,
            display_name: "o1-mini".into(),
        },
        ModelPricing {
            pattern: "o1".into(),
            input_cost_per_1k: 0.015,
            output_cost_per_1k: 0.06,
            display_name: "o1".into(),
        },
        // Google
        ModelPricing {
            pattern: "gemini-2.0-flash".into(),
            input_cost_per_1k: 0.0001,
            output_cost_per_1k: 0.0004,
            display_name: "Gemini 2.0 Flash".into(),
        },
        ModelPricing {
            pattern: "gemini-1.5-pro".into(),
            input_cost_per_1k: 0.00125,
            output_cost_per_1k: 0.005,
            display_name: "Gemini 1.5 Pro".into(),
        },
        ModelPricing {
            pattern: "gemini-flash".into(),
            input_cost_per_1k: 0.000075,
            output_cost_per_1k: 0.0003,
            display_name: "Gemini Flash".into(),
        },
        // Groq (hosted)
        ModelPricing {
            pattern: "groq/".into(),
            input_cost_per_1k: 0.00059,
            output_cost_per_1k: 0.00079,
            display_name: "Groq".into(),
        },
        // Mistral
        ModelPricing {
            pattern: "mistral-large".into(),
            input_cost_per_1k: 0.002,
            output_cost_per_1k: 0.006,
            display_name: "Mistral Large".into(),
        },
        ModelPricing {
            pattern: "mistral-small".into(),
            input_cost_per_1k: 0.0002,
            output_cost_per_1k: 0.0006,
            display_name: "Mistral Small".into(),
        },
        // Together / Fireworks (Llama hosted)
        ModelPricing {
            pattern: "llama-3.3-70b".into(),
            input_cost_per_1k: 0.00059,
            output_cost_per_1k: 0.00079,
            display_name: "Llama 3.3 70B".into(),
        },
        ModelPricing {
            pattern: "llama-3".into(),
            input_cost_per_1k: 0.0002,
            output_cost_per_1k: 0.0002,
            display_name: "Llama 3".into(),
        },
        // Ollama (local, free)
        ModelPricing {
            pattern: "ollama/".into(),
            input_cost_per_1k: 0.0,
            output_cost_per_1k: 0.0,
            display_name: "Ollama (local)".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_estimator_default_pricing() {
        let estimator = CostEstimator::with_defaults();
        assert!(!estimator.pricing().is_empty());
        assert!(estimator.pricing().len() >= 15);
    }

    #[test]
    fn test_find_pricing_anthropic() {
        let estimator = CostEstimator::with_defaults();

        let sonnet = estimator.find_pricing("anthropic/claude-sonnet-4-20250514");
        assert!(sonnet.is_some());
        assert_eq!(sonnet.unwrap().display_name, "Claude Sonnet");

        let opus = estimator.find_pricing("anthropic/claude-opus-4");
        assert!(opus.is_some());
        assert_eq!(opus.unwrap().display_name, "Claude Opus");

        let haiku = estimator.find_pricing("anthropic/claude-haiku-3.5");
        assert!(haiku.is_some());
        assert_eq!(haiku.unwrap().display_name, "Claude Haiku");
    }

    #[test]
    fn test_find_pricing_openai() {
        let estimator = CostEstimator::with_defaults();

        let gpt4o = estimator.find_pricing("openai/gpt-4o");
        assert!(gpt4o.is_some());
        assert_eq!(gpt4o.unwrap().display_name, "GPT-4o");

        let mini = estimator.find_pricing("openai/gpt-4o-mini");
        assert!(mini.is_some());
        assert_eq!(mini.unwrap().display_name, "GPT-4o Mini");
    }

    #[test]
    fn test_find_pricing_ollama_free() {
        let estimator = CostEstimator::with_defaults();
        let ollama = estimator.find_pricing("ollama/llama3.2");
        assert!(ollama.is_some());
        assert_eq!(ollama.unwrap().input_cost_per_1k, 0.0);
        assert_eq!(ollama.unwrap().output_cost_per_1k, 0.0);
    }

    #[test]
    fn test_find_pricing_unknown_model() {
        let estimator = CostEstimator::with_defaults();
        assert!(estimator.find_pricing("unknown/model-xyz").is_none());
    }

    #[test]
    fn test_estimate_cost_sonnet() {
        let estimator = CostEstimator::with_defaults();
        let cost = estimator.estimate(1000, 500, "anthropic/claude-sonnet-4-20250514");

        // 1000 input * 0.003/1k + 500 output * 0.015/1k
        let expected_input = 1.0 * 0.003;
        let expected_output = 0.5 * 0.015;
        assert!((cost.input_cost - expected_input).abs() < 1e-9);
        assert!((cost.output_cost - expected_output).abs() < 1e-9);
        assert!((cost.total_cost - (expected_input + expected_output)).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_cost_ollama_free() {
        let estimator = CostEstimator::with_defaults();
        let cost = estimator.estimate(10000, 5000, "ollama/llama3.2");
        assert_eq!(cost.total_cost, 0.0);
    }

    #[test]
    fn test_estimate_cost_unknown_defaults_to_sonnet() {
        let estimator = CostEstimator::with_defaults();
        let cost = estimator.estimate(1000, 1000, "totally-unknown-model");
        // Default pricing: 0.003/1k input, 0.015/1k output
        assert!((cost.input_cost - 0.003).abs() < 1e-9);
        assert!((cost.output_cost - 0.015).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_from_usage() {
        use crate::token_counter::SessionTokenUsage;

        let usage = SessionTokenUsage {
            input_tokens: 2000,
            output_tokens: 1000,
            total_tokens: 3000,
            message_count: 4,
            by_role: vec![],
        };

        let estimator = CostEstimator::with_defaults();
        let cost = estimator.estimate_from_usage(&usage, "openai/gpt-4o");

        // 2000 * 0.0025/1k + 1000 * 0.01/1k = 0.005 + 0.01 = 0.015
        assert!((cost.total_cost - 0.015).abs() < 1e-9);
        assert_eq!(cost.input_tokens, 2000);
        assert_eq!(cost.output_tokens, 1000);
    }

    #[test]
    fn test_session_cost_serialization() {
        let cost = SessionCost {
            model: "anthropic/claude-sonnet".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            input_cost: 0.003,
            output_cost: 0.0075,
            total_cost: 0.0105,
        };

        let json = serde_json::to_string(&cost).expect("should serialize");
        let parsed: SessionCost = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.model, "anthropic/claude-sonnet");
        assert_eq!(parsed.input_tokens, 1000);
        assert!((parsed.total_cost - 0.0105).abs() < 1e-9);
    }

    #[test]
    fn test_model_pricing_order_matters() {
        // gpt-4o-mini should match before gpt-4o
        let estimator = CostEstimator::with_defaults();
        let mini = estimator.find_pricing("openai/gpt-4o-mini");
        assert!(mini.is_some());
        assert_eq!(mini.unwrap().display_name, "GPT-4o Mini");
    }

    #[test]
    fn test_groq_pricing() {
        let estimator = CostEstimator::with_defaults();
        let groq = estimator.find_pricing("groq/llama-3.3-70b-versatile");
        assert!(groq.is_some());
    }
}
