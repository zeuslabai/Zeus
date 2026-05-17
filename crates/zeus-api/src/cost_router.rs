//! Cost-based auto-routing module
//!
//! Routes requests to cheaper LLM providers when appropriate based on task
//! complexity. Tracks per-model spending against a configurable monthly budget.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Types
// ============================================================================

/// Per-model pricing and capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCost {
    /// Model identifier (e.g. "anthropic/claude-sonnet-4-20250514")
    pub model: String,
    /// Cost per 1 000 input tokens (USD)
    pub cost_per_1k_input: f64,
    /// Cost per 1 000 output tokens (USD)
    pub cost_per_1k_output: f64,
    /// Maximum context window the model supports
    pub max_tokens: u32,
    /// Estimated first-token latency in milliseconds
    pub latency_ms_estimate: u32,
    /// Highest task tier this model is suitable for
    pub max_tier: TaskTier,
}

/// Complexity classification for an incoming request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTier {
    /// Trivial look-ups, greetings, formatting — cheapest model
    Simple = 0,
    /// Summarisation, Q&A, light code review — mid-range model
    Standard = 1,
    /// Multi-step reasoning, code generation, analysis — premium model
    Complex = 2,
    /// Research-grade tasks, long planning, architecture — best available
    Expert = 3,
}

/// Accumulated cost snapshot returned by the budget endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    /// Total spend in the current period (USD)
    pub total_cost: f64,
    /// Remaining budget (USD). `None` when no budget is set.
    pub budget_remaining: Option<f64>,
    /// Per-model spend, sorted descending by cost
    pub top_models: Vec<(String, f64)>,
    /// ISO-8601 timestamp of the current period start
    pub period_start: String,
}

// ============================================================================
// CostRouter
// ============================================================================

/// Selects the cheapest capable model for a given task tier and tracks spend
/// against a monthly budget.
#[derive(Debug, Clone)]
pub struct CostRouter {
    /// Available models with pricing
    providers: Vec<ProviderCost>,
    /// Optional monthly budget cap (USD). `None` means unlimited.
    monthly_budget: Option<f64>,
    /// Running cost total for the current period
    running_cost: f64,
    /// Per-model cost accumulator
    model_costs: HashMap<String, f64>,
    /// Per-model token counts: model -> (input_tokens, output_tokens)
    model_tokens: HashMap<String, (u64, u64)>,
    /// Per-model request counts
    model_requests: HashMap<String, u64>,
    /// ISO-8601 start of current billing period
    period_start: String,
}

impl CostRouter {
    // -- constructors --------------------------------------------------------

    /// Create a router with explicit provider list and budget.
    pub fn new(providers: Vec<ProviderCost>, monthly_budget: Option<f64>) -> Self {
        Self {
            providers,
            monthly_budget,
            running_cost: 0.0,
            model_costs: HashMap::new(),
            model_tokens: HashMap::new(),
            model_requests: HashMap::new(),
            period_start: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create a router pre-populated with the default cost table.
    pub fn with_defaults(monthly_budget: Option<f64>) -> Self {
        Self::new(default_cost_table(), monthly_budget)
    }

    // -- routing -------------------------------------------------------------

    /// Return the cheapest model that can handle `tier`.
    ///
    /// Models are filtered to those whose `max_tier >= tier`, then sorted by
    /// a blended cost metric (average of input + output cost per 1k tokens).
    /// Returns `None` only when the provider list is empty or no model covers
    /// the requested tier.
    pub fn recommend(&self, tier: TaskTier) -> Option<&ProviderCost> {
        let mut candidates: Vec<&ProviderCost> = self
            .providers
            .iter()
            .filter(|p| p.max_tier >= tier)
            .collect();

        candidates.sort_by(|a, b| {
            let cost_a = a.cost_per_1k_input + a.cost_per_1k_output;
            let cost_b = b.cost_per_1k_input + b.cost_per_1k_output;
            cost_a
                .partial_cmp(&cost_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.first().copied()
    }

    // -- budget tracking -----------------------------------------------------

    /// Record token usage for a model and update the running cost total.
    ///
    /// Returns the cost of this particular usage.  If the model is unknown the
    /// cost is recorded as zero.
    pub fn record_usage(&mut self, input_tokens: u64, output_tokens: u64, model: &str) -> f64 {
        let cost = self
            .providers
            .iter()
            .find(|p| p.model == model)
            .map(|p| {
                (input_tokens as f64 / 1000.0) * p.cost_per_1k_input
                    + (output_tokens as f64 / 1000.0) * p.cost_per_1k_output
            })
            .unwrap_or(0.0);

        self.running_cost += cost;
        *self.model_costs.entry(model.to_string()).or_insert(0.0) += cost;

        let tokens = self.model_tokens.entry(model.to_string()).or_insert((0, 0));
        tokens.0 += input_tokens;
        tokens.1 += output_tokens;

        *self.model_requests.entry(model.to_string()).or_insert(0) += 1;

        cost
    }

    /// `true` when spend is still within the configured monthly budget (or
    /// when no budget has been set).
    pub fn check_budget(&self) -> bool {
        match self.monthly_budget {
            Some(limit) => self.running_cost <= limit,
            None => true,
        }
    }

    /// Build a cost summary snapshot.
    pub fn summary(&self) -> CostSummary {
        let mut top: Vec<(String, f64)> = self.model_costs.clone().into_iter().collect();
        top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        CostSummary {
            total_cost: self.running_cost,
            budget_remaining: self
                .monthly_budget
                .map(|b| (b - self.running_cost).max(0.0)),
            top_models: top,
            period_start: self.period_start.clone(),
        }
    }

    // -- accessors -----------------------------------------------------------

    /// Reset the running totals (e.g. at the start of a new billing period).
    pub fn reset_period(&mut self) {
        self.running_cost = 0.0;
        self.model_costs.clear();
        self.model_tokens.clear();
        self.model_requests.clear();
        self.period_start = chrono::Utc::now().to_rfc3339();
    }

    /// Read-only access to the provider cost table.
    pub fn providers(&self) -> &[ProviderCost] {
        &self.providers
    }

    /// Current running cost total.
    pub fn running_cost(&self) -> f64 {
        self.running_cost
    }

    /// Monthly budget limit, if set.
    pub fn monthly_budget(&self) -> Option<f64> {
        self.monthly_budget
    }

    /// Per-model token counts: model -> (input_tokens, output_tokens).
    pub fn model_tokens(&self) -> &HashMap<String, (u64, u64)> {
        &self.model_tokens
    }

    /// Per-model request counts.
    pub fn model_requests(&self) -> &HashMap<String, u64> {
        &self.model_requests
    }

    /// Total input tokens across all models.
    pub fn total_input_tokens(&self) -> u64 {
        self.model_tokens.values().map(|(i, _)| i).sum()
    }

    /// Total output tokens across all models.
    pub fn total_output_tokens(&self) -> u64 {
        self.model_tokens.values().map(|(_, o)| o).sum()
    }

    /// Total request count across all models.
    pub fn total_requests(&self) -> u64 {
        self.model_requests.values().sum()
    }

    /// Classify a free-text task description into a `TaskTier`.
    ///
    /// Uses simple keyword heuristics — intentionally cheap so it can run on
    /// every request without an LLM call.
    pub fn classify_task(description: &str) -> TaskTier {
        let lower = description.to_lowercase();

        // Expert indicators
        let expert_keywords = [
            "architect",
            "design system",
            "research paper",
            "long-term plan",
            "security audit",
            "threat model",
            "formal verification",
            "proof",
            "novel algorithm",
            "deep analysis",
        ];
        if expert_keywords.iter().any(|k| lower.contains(k)) {
            return TaskTier::Expert;
        }

        // Complex indicators
        let complex_keywords = [
            "implement",
            "refactor",
            "debug",
            "optimize",
            "generate code",
            "write code",
            "multi-step",
            "analyze",
            "compare",
            "evaluate",
            "test plan",
            "migration",
        ];
        if complex_keywords.iter().any(|k| lower.contains(k)) {
            return TaskTier::Complex;
        }

        // Standard indicators
        let standard_keywords = [
            "summarize",
            "summarise",
            "explain",
            "translate",
            "review",
            "describe",
            "list",
            "outline",
            "convert",
            "rewrite",
        ];
        if standard_keywords.iter().any(|k| lower.contains(k)) {
            return TaskTier::Standard;
        }

        // Everything else (greetings, simple lookups, formatting)
        TaskTier::Simple
    }
}

// ============================================================================
// Default cost table
// ============================================================================

/// Pre-populated pricing for common models across Anthropic, OpenAI, Groq, and
/// Ollama (local).  Prices are approximate as of early 2026.
pub fn default_cost_table() -> Vec<ProviderCost> {
    vec![
        // -- Ollama (local, zero cost) --
        ProviderCost {
            model: "ollama/llama3.2".into(),
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            max_tokens: 128_000,
            latency_ms_estimate: 200,
            max_tier: TaskTier::Simple,
        },
        // -- Groq (hosted, very cheap) --
        ProviderCost {
            model: "groq/llama-3.3-70b-versatile".into(),
            cost_per_1k_input: 0.00059,
            cost_per_1k_output: 0.00079,
            max_tokens: 128_000,
            latency_ms_estimate: 150,
            max_tier: TaskTier::Standard,
        },
        // -- OpenAI --
        ProviderCost {
            model: "openai/gpt-4o-mini".into(),
            cost_per_1k_input: 0.00015,
            cost_per_1k_output: 0.0006,
            max_tokens: 128_000,
            latency_ms_estimate: 400,
            max_tier: TaskTier::Standard,
        },
        ProviderCost {
            model: "openai/gpt-4o".into(),
            cost_per_1k_input: 0.0025,
            cost_per_1k_output: 0.01,
            max_tokens: 128_000,
            latency_ms_estimate: 600,
            max_tier: TaskTier::Complex,
        },
        ProviderCost {
            model: "openai/gpt-4".into(),
            cost_per_1k_input: 0.03,
            cost_per_1k_output: 0.06,
            max_tokens: 8_192,
            latency_ms_estimate: 800,
            max_tier: TaskTier::Expert,
        },
        // -- Anthropic --
        ProviderCost {
            model: "anthropic/claude-haiku".into(),
            cost_per_1k_input: 0.00025,
            cost_per_1k_output: 0.00125,
            max_tokens: 200_000,
            latency_ms_estimate: 300,
            max_tier: TaskTier::Standard,
        },
        ProviderCost {
            model: "anthropic/claude-sonnet-4-20250514".into(),
            cost_per_1k_input: 0.003,
            cost_per_1k_output: 0.015,
            max_tokens: 200_000,
            latency_ms_estimate: 500,
            max_tier: TaskTier::Complex,
        },
        ProviderCost {
            model: "anthropic/claude-opus-4".into(),
            cost_per_1k_input: 0.015,
            cost_per_1k_output: 0.075,
            max_tokens: 200_000,
            latency_ms_estimate: 1000,
            max_tier: TaskTier::Expert,
        },
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_router() -> CostRouter {
        CostRouter::with_defaults(Some(10.0))
    }

    // -- routing tests -------------------------------------------------------

    #[test]
    fn simple_tier_picks_cheapest() {
        let router = test_router();
        let pick = router.recommend(TaskTier::Simple).unwrap();
        // Ollama is free, should always win for Simple
        assert_eq!(pick.model, "ollama/llama3.2");
    }

    #[test]
    fn standard_tier_picks_cheapest_capable() {
        let router = test_router();
        let pick = router.recommend(TaskTier::Standard).unwrap();
        // Among Standard-capable: ollama (free), gpt-4o-mini (cheap), haiku, groq
        // ollama max_tier is Simple, so it should NOT be picked
        assert_ne!(pick.model, "ollama/llama3.2");
        // Should be one of the cheap Standard-capable models
        let cheap_standard = [
            "openai/gpt-4o-mini",
            "anthropic/claude-haiku",
            "groq/llama-3.3-70b-versatile",
        ];
        assert!(
            cheap_standard.contains(&pick.model.as_str()),
            "expected a cheap standard model, got {}",
            pick.model
        );
    }

    #[test]
    fn complex_tier_excludes_simple_and_standard_only() {
        let router = test_router();
        let pick = router.recommend(TaskTier::Complex).unwrap();
        assert!(
            pick.max_tier >= TaskTier::Complex,
            "model {} has tier {:?}, expected >= Complex",
            pick.model,
            pick.max_tier
        );
    }

    #[test]
    fn expert_tier_returns_expert_model() {
        let router = test_router();
        let pick = router.recommend(TaskTier::Expert).unwrap();
        assert!(
            pick.max_tier >= TaskTier::Expert,
            "model {} has tier {:?}, expected Expert",
            pick.model,
            pick.max_tier
        );
    }

    #[test]
    fn empty_provider_list_returns_none() {
        let router = CostRouter::new(vec![], None);
        assert!(router.recommend(TaskTier::Simple).is_none());
    }

    // -- budget tracking tests -----------------------------------------------

    #[test]
    fn record_usage_accumulates_cost() {
        let mut router = test_router();
        // Use Claude Sonnet: 0.003/1k in, 0.015/1k out
        let cost = router.record_usage(1000, 1000, "anthropic/claude-sonnet-4-20250514");
        let expected = 0.003 + 0.015; // 0.018
        assert!(
            (cost - expected).abs() < 1e-9,
            "cost was {cost}, expected {expected}"
        );
        assert!((router.running_cost() - expected).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_records_zero_cost() {
        let mut router = test_router();
        let cost = router.record_usage(5000, 5000, "unknown/model-xyz");
        assert!((cost - 0.0).abs() < 1e-9);
    }

    #[test]
    fn budget_exceeded_detected() {
        let mut router = CostRouter::with_defaults(Some(0.01));
        assert!(router.check_budget());
        // Record enough to blow the budget
        router.record_usage(10_000, 10_000, "anthropic/claude-opus-4");
        assert!(!router.check_budget());
    }

    #[test]
    fn no_budget_always_within() {
        let mut router = CostRouter::with_defaults(None);
        router.record_usage(1_000_000, 1_000_000, "anthropic/claude-opus-4");
        assert!(router.check_budget());
    }

    #[test]
    fn reset_period_clears_totals() {
        let mut router = test_router();
        router.record_usage(5000, 5000, "openai/gpt-4o");
        assert!(router.running_cost() > 0.0);
        router.reset_period();
        assert!((router.running_cost() - 0.0).abs() < 1e-9);
        let summary = router.summary();
        assert!(summary.top_models.is_empty());
    }

    #[test]
    fn summary_shows_top_models_descending() {
        let mut router = test_router();
        // Record cheap usage
        router.record_usage(1000, 1000, "openai/gpt-4o-mini");
        // Record expensive usage
        router.record_usage(1000, 1000, "anthropic/claude-opus-4");
        let summary = router.summary();
        assert_eq!(summary.top_models.len(), 2);
        // Most expensive first
        assert_eq!(summary.top_models[0].0, "anthropic/claude-opus-4");
    }

    #[test]
    fn summary_budget_remaining_correct() {
        let mut router = CostRouter::with_defaults(Some(50.0));
        router.record_usage(1000, 1000, "openai/gpt-4o"); // ~0.0125
        let summary = router.summary();
        let remaining = summary.budget_remaining.unwrap();
        assert!(remaining < 50.0);
        assert!(remaining > 49.0);
    }

    // -- task classification tests -------------------------------------------

    #[test]
    fn classify_simple_task() {
        assert_eq!(CostRouter::classify_task("hello there"), TaskTier::Simple);
        assert_eq!(
            CostRouter::classify_task("what time is it"),
            TaskTier::Simple
        );
    }

    #[test]
    fn classify_standard_task() {
        assert_eq!(
            CostRouter::classify_task("summarize this document"),
            TaskTier::Standard
        );
        assert_eq!(
            CostRouter::classify_task("explain how TCP works"),
            TaskTier::Standard
        );
    }

    #[test]
    fn classify_complex_task() {
        assert_eq!(
            CostRouter::classify_task("implement a binary search tree"),
            TaskTier::Complex
        );
        assert_eq!(
            CostRouter::classify_task("refactor the authentication module"),
            TaskTier::Complex
        );
    }

    #[test]
    fn classify_expert_task() {
        assert_eq!(
            CostRouter::classify_task("architect a distributed event bus"),
            TaskTier::Expert
        );
        assert_eq!(
            CostRouter::classify_task("write a research paper on transformer efficiency"),
            TaskTier::Expert
        );
    }

    #[test]
    fn default_cost_table_has_entries() {
        let table = default_cost_table();
        assert!(
            table.len() >= 8,
            "expected at least 8 models, got {}",
            table.len()
        );
        // Every tier should have at least one model
        for tier in [
            TaskTier::Simple,
            TaskTier::Standard,
            TaskTier::Complex,
            TaskTier::Expert,
        ] {
            assert!(
                table.iter().any(|p| p.max_tier == tier),
                "no model for tier {tier:?}"
            );
        }
    }

    #[test]
    fn multiple_usages_accumulate() {
        let mut router = test_router();
        router.record_usage(1000, 1000, "openai/gpt-4o-mini");
        router.record_usage(2000, 2000, "openai/gpt-4o-mini");
        let summary = router.summary();
        // Should have one entry for gpt-4o-mini
        assert_eq!(summary.top_models.len(), 1);
        // Cost should be sum of both calls
        let model_cost = summary.top_models[0].1;
        // 3000 tokens input + 3000 tokens output at mini prices
        let expected = (3.0 * 0.00015) + (3.0 * 0.0006);
        assert!(
            (model_cost - expected).abs() < 1e-9,
            "cost was {model_cost}, expected {expected}"
        );
    }

    // -- token and request tracking tests ------------------------------------

    #[test]
    fn record_usage_tracks_tokens() {
        let mut router = test_router();
        router.record_usage(1000, 500, "anthropic/claude-sonnet-4-20250514");
        router.record_usage(2000, 1000, "openai/gpt-4o");

        assert_eq!(router.total_input_tokens(), 3000);
        assert_eq!(router.total_output_tokens(), 1500);

        let tokens = router.model_tokens();
        assert_eq!(tokens["anthropic/claude-sonnet-4-20250514"], (1000, 500));
        assert_eq!(tokens["openai/gpt-4o"], (2000, 1000));
    }

    #[test]
    fn record_usage_tracks_requests() {
        let mut router = test_router();
        router.record_usage(100, 50, "openai/gpt-4o-mini");
        router.record_usage(200, 100, "openai/gpt-4o-mini");
        router.record_usage(300, 150, "anthropic/claude-haiku");

        assert_eq!(router.total_requests(), 3);
        assert_eq!(router.model_requests()["openai/gpt-4o-mini"], 2);
        assert_eq!(router.model_requests()["anthropic/claude-haiku"], 1);
    }

    #[test]
    fn multiple_usages_accumulate_tokens() {
        let mut router = test_router();
        router.record_usage(1000, 500, "openai/gpt-4o-mini");
        router.record_usage(2000, 1000, "openai/gpt-4o-mini");

        let tokens = router.model_tokens();
        assert_eq!(tokens["openai/gpt-4o-mini"], (3000, 1500));
        assert_eq!(router.model_requests()["openai/gpt-4o-mini"], 2);
    }

    #[test]
    fn reset_period_clears_tokens_and_requests() {
        let mut router = test_router();
        router.record_usage(5000, 5000, "openai/gpt-4o");
        assert!(router.total_input_tokens() > 0);
        assert!(router.total_requests() > 0);

        router.reset_period();
        assert_eq!(router.total_input_tokens(), 0);
        assert_eq!(router.total_output_tokens(), 0);
        assert_eq!(router.total_requests(), 0);
        assert!(router.model_tokens().is_empty());
        assert!(router.model_requests().is_empty());
    }
}
