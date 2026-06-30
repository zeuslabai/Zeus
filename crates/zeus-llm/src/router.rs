//! Model Router — intelligent per-task model selection
//!
//! Selects the optimal LLM provider+model for a given task based on:
//! - Task type (reasoning, code, research, speed, creative)
//! - Complexity level (simple/moderate/complex)
//! - Budget constraints (model_tier_limit from ComputeProvisioner)
//!
//! Configurable via `[model_routing]` in config.toml. Falls back to the
//! default model from config if no routing rules match or routing is disabled.

use serde::{Deserialize, Serialize};
use tracing::debug;
use zeus_core::{ModelRoutingCoreConfig, Provider};

// ============================================================================
// Task Classification
// ============================================================================

/// The type of task to route to an appropriate model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Deep reasoning, planning, complex analysis — needs strongest model
    Reasoning,
    /// Code generation, refactoring, debugging — needs code-optimized model
    Code,
    /// Web research, summarization, information retrieval
    Research,
    /// Quick responses, simple lookups, formatting — needs fastest model
    Speed,
    /// Creative writing, brainstorming, content generation
    Creative,
    /// Review, validation, approval — moderate capability needed
    Review,
    /// General-purpose fallback
    General,
}

impl TaskType {
    /// Classify a task description into a type using multi-signal weighted scoring.
    ///
    /// Unlike `classify()` (first-match keyword search), this walks every
    /// task-type scorer and picks the highest-scoring one. When signals are
    /// ambiguous, falls back to `General`.
    pub fn classify_with_context(ctx: &ClassificationContext) -> Self {
        let mut scores: [(Self, i32); 7] = [
            (Self::Reasoning, 0),
            (Self::Code, 0),
            (Self::Research, 0),
            (Self::Review, 0),
            (Self::Creative, 0),
            (Self::Speed, 0),
            (Self::General, 0),
        ];

        let lower = ctx.message.to_lowercase();
        let len = ctx.message_len();
        let words = ctx.word_count();

        // Keyword-weighted signals (score = # of matched keywords * weight)
        scores[0].1 += score_keywords(
            &lower,
            &[
                "plan", "architect", "design", "strategy", "analyze", "analysis",
                "decompos", "reason", "decide", "trade-off", "tradeoff", "evaluate",
                "should we", "why does", "root cause",
            ],
        );
        scores[1].1 += score_keywords(
            &lower,
            &[
                "implement", "refactor", "debug", "fix bug", "fix the",
                "write function", "compile", "unit test", "cargo", "npm ",
                "pytest", "function", "method", "class", "module",
            ],
        );
        // "code" keyword gets a smaller weight — it's too generic
        if lower.contains("code") {
            scores[1].1 += 1;
        }
        scores[2].1 += score_keywords(
            &lower,
            &[
                "research", "look up", "find out", "summariz", "gather",
                "collect", "who is", "what is the", "search for",
            ],
        );
        scores[3].1 += score_keywords(
            &lower,
            &[
                "review", "validate", "approve", "verify", "audit",
                "double-check", "sanity check", "lgtm",
            ],
        );
        scores[4].1 += score_keywords(
            &lower,
            &[
                "write a", "draft", "compose", "creative", "story",
                "blog", "poem", "tagline", "caption", "headline",
            ],
        );
        scores[5].1 += score_keywords(
            &lower,
            &[
                "format", "list", "convert", "translate", "rename",
                "count", "capitalize", "uppercase", "lowercase",
            ],
        );

        // Code-context signal: boost Code
        if ctx.has_code_context {
            scores[1].1 += 3;
        }

        // Hint signal
        if let Some(ref hint) = ctx.hint {
            let h = hint.to_lowercase();
            match h.as_str() {
                "code" | "code-review" | "engineering" | "dev" => scores[1].1 += 5,
                "research" | "search" => scores[2].1 += 5,
                "review" | "audit" => scores[3].1 += 5,
                "creative" | "content" | "social" => scores[4].1 += 5,
                "reasoning" | "planning" | "architecture" => scores[0].1 += 5,
                "speed" | "quick" => scores[5].1 += 5,
                _ => {}
            }
        }

        // Length-based signal: very short messages lean toward Speed/General;
        // very long ones carry more Reasoning weight. Only apply the short→Speed
        // bonus if we already have at least one Speed-flavoured signal —
        // otherwise a bare greeting like "hello there" would misclassify.
        if len < 40 && words < 8 && scores[5].1 > 0 {
            scores[5].1 += 2; // short + already looks like speed → speed
        } else if len > 500 || words > 80 {
            scores[0].1 += 2; // long → reasoning
        }

        // Tool-call history: if the agent has already hammered tools this turn,
        // we're likely deep in a code/debug loop. Strong enough to override
        // ambiguity.
        if ctx.tool_calls >= 3 {
            scores[1].1 += 4;
        }

        // Pick the highest score; ties broken by declaration order (Reasoning wins).
        let (best, best_score) = scores
            .iter()
            .copied()
            .max_by_key(|(_, s)| *s)
            .unwrap_or((Self::General, 0));

        if best_score == 0 {
            // Fall back to keyword-only classifier so we don't regress
            // short/simple messages like "hello world".
            return Self::classify(&ctx.message);
        }

        best
    }

    /// Classify a task description into a type using keyword heuristics.
    pub fn classify(description: &str) -> Self {
        let lower = description.to_lowercase();

        // Reasoning signals
        if lower.contains("plan")
            || lower.contains("architect")
            || lower.contains("design")
            || lower.contains("strategy")
            || lower.contains("analyze")
            || lower.contains("decompos")
            || lower.contains("reason")
            || lower.contains("decide")
        {
            return Self::Reasoning;
        }

        // Code signals
        if lower.contains("implement")
            || lower.contains("code")
            || lower.contains("refactor")
            || lower.contains("debug")
            || lower.contains("fix bug")
            || lower.contains("write function")
            || lower.contains("build")
            || lower.contains("compile")
            || lower.contains("test")
        {
            return Self::Code;
        }

        // Research signals
        if lower.contains("research")
            || lower.contains("search")
            || lower.contains("find")
            || lower.contains("summariz")
            || lower.contains("look up")
            || lower.contains("gather")
            || lower.contains("collect")
        {
            return Self::Research;
        }

        // Review signals (check before speed — "validate" is more specific than "format")
        if lower.contains("review")
            || lower.contains("validate")
            || lower.contains("check")
            || lower.contains("approve")
            || lower.contains("verify")
        {
            return Self::Review;
        }

        // Creative signals
        if lower.contains("write")
            || lower.contains("draft")
            || lower.contains("compose")
            || lower.contains("creative")
            || lower.contains("story")
            || lower.contains("blog")
        {
            return Self::Creative;
        }

        // Speed signals (simple tasks)
        if lower.contains("format")
            || lower.contains("list")
            || lower.contains("convert")
            || lower.contains("translate")
            || lower.contains("rename")
            || lower.contains("count")
        {
            return Self::Speed;
        }

        Self::General
    }
}

/// Count how many of `keywords` appear in `lower` (case-insensitive lookup
/// already done by caller). Returns a score equal to the number of matches.
fn score_keywords(lower: &str, keywords: &[&str]) -> i32 {
    keywords.iter().filter(|k| lower.contains(*k)).count() as i32
}

/// Complexity level for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Simple,
    Moderate,
    Complex,
}

impl Complexity {
    /// Infer complexity from a `ClassificationContext`.
    ///
    /// Blends multiple signals:
    /// * message length (chars + words)
    /// * tool-call history (more tool calls ⇒ we're deeper in the problem)
    /// * complexity keywords ("multi-step", "complicated", "simple", etc.)
    /// * code-context flag
    pub fn infer(ctx: &ClassificationContext) -> Self {
        let lower = ctx.message.to_lowercase();
        let len = ctx.message_len();
        let words = ctx.word_count();
        let mut score: i32 = 0;

        // Length contributes monotonically
        if len > 800 || words > 120 {
            score += 3;
        } else if len > 300 || words > 50 {
            score += 2;
        } else if len > 100 || words > 20 {
            score += 1;
        }

        // Explicit complexity vocabulary
        let complex_kw = [
            "multi-step", "multi step", "architect", "design a system",
            "end-to-end", "refactor the whole", "complicated", "several",
            "multiple files", "cross-crate", "trade-off", "tradeoff",
        ];
        let simple_kw = [
            "quick", "simple", "just", "one-liner", "trivial", "small",
            "tiny", "rename", "format",
        ];
        score += score_keywords(&lower, &complex_kw) * 2;
        score -= score_keywords(&lower, &simple_kw);

        // Tool-call history
        if ctx.tool_calls >= 5 {
            score += 2;
        } else if ctx.tool_calls >= 2 {
            score += 1;
        }

        // Code-context slightly raises the floor
        if ctx.has_code_context {
            score += 1;
        }

        // History depth: very long sessions tend to carry complex tasks
        if ctx.history_len > 30 {
            score += 1;
        }

        if score >= 4 {
            Self::Complex
        } else if score >= 1 {
            Self::Moderate
        } else {
            Self::Simple
        }
    }
}

/// One-shot classifier: returns both `TaskType` and `Complexity` in one call.
pub fn classify_full(ctx: &ClassificationContext) -> (TaskType, Complexity) {
    (TaskType::classify_with_context(ctx), Complexity::infer(ctx))
}

// ============================================================================
// Multi-signal Classification Context
// ============================================================================

/// Rich signal bundle for smarter task classification.
///
/// Rather than looking only at keywords, `ClassificationContext` aggregates
/// everything we know about a request: the message itself, how long it is,
/// how many tool calls the agent has already made this turn, and optional
/// caller hints (e.g. "this came from an @mention in a code review channel").
///
/// Use `ClassificationContext::new(message)` for the common case, then chain
/// `with_tool_calls`, `with_history_len`, `with_hint` as needed.
#[derive(Debug, Clone, Default)]
pub struct ClassificationContext {
    /// The user/task message to classify.
    pub message: String,
    /// Number of tool calls already executed this turn (0 = first pass).
    pub tool_calls: usize,
    /// Number of prior messages in the session history (rough proxy for context depth).
    pub history_len: usize,
    /// Optional freeform hint from the caller (e.g. "code-review", "social").
    pub hint: Option<String>,
    /// Whether the request includes code blocks or file paths.
    pub has_code_context: bool,
}

impl ClassificationContext {
    /// Build from just a message (most common entry point).
    pub fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        let has_code_context = detect_code_context(&message);
        Self {
            message,
            tool_calls: 0,
            history_len: 0,
            hint: None,
            has_code_context,
        }
    }

    /// Attach tool-call count for this turn.
    pub fn with_tool_calls(mut self, n: usize) -> Self {
        self.tool_calls = n;
        self
    }

    /// Attach session history length.
    pub fn with_history_len(mut self, n: usize) -> Self {
        self.history_len = n;
        self
    }

    /// Attach a caller hint.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Manually override code-context detection.
    pub fn with_code_context(mut self, has: bool) -> Self {
        self.has_code_context = has;
        self
    }

    /// Character length of the message.
    pub fn message_len(&self) -> usize {
        self.message.chars().count()
    }

    /// Rough word count.
    pub fn word_count(&self) -> usize {
        self.message.split_whitespace().count()
    }
}

/// Heuristic: does the message look like it carries code or file paths?
fn detect_code_context(msg: &str) -> bool {
    msg.contains("```")
        || msg.contains("fn ")
        || msg.contains("def ")
        || msg.contains("class ")
        || msg.contains(".rs")
        || msg.contains(".py")
        || msg.contains(".ts")
        || msg.contains(".js")
        || msg.contains("cargo ")
        || msg.contains("npm ")
}

// ============================================================================
// Routing Configuration
// ============================================================================

/// Re-export the config type from zeus-core for convenience.
///
/// Example config:
/// ```toml
/// [model_routing]
/// enabled = true
/// reasoning = "anthropic/claude-opus-4-20250514"
/// code = "anthropic/claude-sonnet-4-20250514"
/// research = "google/gemini-2.0-flash"
/// speed = "anthropic/claude-haiku-4-5-20251001"  # or "groq/llama-3.3-70b-versatile"
/// creative = "anthropic/claude-sonnet-4-20250514"
/// review = "anthropic/claude-sonnet-4-20250514"
/// ```
pub type ModelRoutingConfig = ModelRoutingCoreConfig;

// ============================================================================
// Model Router
// ============================================================================

/// Route selection result.
#[derive(Debug, Clone)]
pub struct RouteSelection {
    /// Provider to use
    pub provider: Provider,
    /// Model name to use
    pub model: String,
    /// Full "provider/model" string
    pub model_string: String,
    /// Why this model was selected
    pub reason: &'static str,
}

/// Intelligent per-task model router.
///
/// When routing is enabled, selects the best model for each task type.
/// When disabled, always returns the default model.
pub struct ModelRouter {
    config: ModelRoutingConfig,
    default_model: String,
}

impl ModelRouter {
    /// Create a new router with config and default model fallback.
    pub fn new(config: ModelRoutingConfig, default_model: String) -> Self {
        Self {
            config,
            default_model,
        }
    }

    /// Create from a zeus-core Config (reads `[model_routing]` section + default model).
    pub fn from_config(config: &zeus_core::Config) -> Self {
        let routing = config.model_routing.clone().unwrap_or_default();
        Self {
            config: routing,
            default_model: config.model.clone(),
        }
    }

    /// Create a passthrough router that always returns the default model.
    pub fn passthrough(default_model: String) -> Self {
        Self {
            config: ModelRoutingConfig::default(),
            default_model,
        }
    }

    /// Whether intelligent routing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Select the best model for a task.
    ///
    /// `model_tier_limit`: 0=any, 1=small only, 2=small+medium (from ComputeProvisioner)
    pub fn select(
        &self,
        task_type: TaskType,
        complexity: Complexity,
        model_tier_limit: u8,
    ) -> RouteSelection {
        if !self.config.enabled {
            return self.default_route("routing disabled");
        }

        // If tier-restricted to small models only, always use speed model
        if model_tier_limit == 1 {
            if let Some(ref speed) = self.config.speed {
                return self.parse_route(speed, "tier-restricted to small models");
            }
            return self.default_route("tier-restricted, no speed model configured");
        }

        // For simple tasks, prefer speed regardless of type
        if complexity == Complexity::Simple
            && task_type != TaskType::Reasoning
            && let Some(ref speed) = self.config.speed
        {
            return self.parse_route(speed, "simple task → speed model");
        }

        // Route by task type
        let configured = match task_type {
            TaskType::Reasoning => &self.config.reasoning,
            TaskType::Code => &self.config.code,
            TaskType::Research => &self.config.research,
            TaskType::Speed => &self.config.speed,
            TaskType::Creative => &self.config.creative,
            TaskType::Review => &self.config.review,
            TaskType::General => &self.config.general,
        };

        if let Some(model_str) = configured {
            let reason = match task_type {
                TaskType::Reasoning => "reasoning task → strongest model",
                TaskType::Code => "code task → code-optimized model",
                TaskType::Research => "research task → retrieval model",
                TaskType::Speed => "speed task → fastest model",
                TaskType::Creative => "creative task → creative model",
                TaskType::Review => "review task → review model",
                TaskType::General => "general task → default model",
            };
            return self.parse_route(model_str, reason);
        }

        self.default_route("no route configured for task type")
    }

    /// Select model for a Pantheon agent role.
    pub fn select_for_role(&self, role: &str) -> RouteSelection {
        if !self.config.enabled {
            return self.default_route("routing disabled");
        }

        match role {
            "coordinator" => {
                if let Some(ref m) = self.config.reasoning {
                    return self.parse_route(m, "coordinator → reasoning model");
                }
            }
            "worker" => {
                if let Some(ref m) = self.config.code {
                    return self.parse_route(m, "worker → code model");
                }
            }
            "reviewer" => {
                if let Some(ref m) = self.config.review {
                    return self.parse_route(m, "reviewer → review model");
                }
            }
            _ => {}
        }

        self.default_route("no role-specific route")
    }

    fn default_route(&self, reason: &'static str) -> RouteSelection {
        self.parse_route(&self.default_model, reason)
    }

    fn parse_route(&self, model_string: &str, reason: &'static str) -> RouteSelection {
        let (provider, model) = parse_model_string(model_string);
        debug!(model = model_string, reason, "Model route selected");
        RouteSelection {
            provider,
            model,
            model_string: model_string.to_string(),
            reason,
        }
    }
}

/// Parse "provider/model" string into (Provider, model_name).
/// Falls back to Ollama if no provider prefix.
pub fn parse_model_string(s: &str) -> (Provider, String) {
    if let Some((provider, model)) = s.split_once('/') {
        let p = match provider.to_lowercase().as_str() {
            "anthropic" => Provider::Anthropic,
            "openai" => Provider::OpenAI,
            "ollama" => Provider::Ollama,
            "openrouter" => Provider::OpenRouter,
            "google" | "gemini" => Provider::Google,
            "groq" => Provider::Groq,
            "mistral" => Provider::Mistral,
            "together" => Provider::Together,
            "fireworks" => Provider::Fireworks,
            "azure" => Provider::Azure,
            "bedrock" | "aws" => Provider::Bedrock,
            _ => Provider::Ollama,
        };
        (p, model.to_string())
    } else {
        (Provider::Ollama, s.to_string())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ModelRoutingConfig {
        ModelRoutingConfig {
            enabled: true,
            reasoning: Some("anthropic/claude-opus-4-20250514".into()),
            code: Some("anthropic/claude-sonnet-4-20250514".into()),
            research: Some("google/gemini-2.0-flash".into()),
            speed: Some("anthropic/claude-haiku-4-5-20251001".into()),
            creative: Some("anthropic/claude-sonnet-4-20250514".into()),
            review: Some("anthropic/claude-sonnet-4-20250514".into()),
            general: Some("anthropic/claude-sonnet-4-20250514".into()),
        }
    }

    #[test]
    fn test_classify_reasoning() {
        assert_eq!(
            TaskType::classify("Plan the architecture for a new system"),
            TaskType::Reasoning
        );
        assert_eq!(
            TaskType::classify("Analyze the trade-offs"),
            TaskType::Reasoning
        );
        assert_eq!(
            TaskType::classify("Design a migration strategy"),
            TaskType::Reasoning
        );
    }

    #[test]
    fn test_classify_code() {
        assert_eq!(
            TaskType::classify("Implement the user login flow"),
            TaskType::Code
        );
        assert_eq!(TaskType::classify("Fix bug in the parser"), TaskType::Code);
        assert_eq!(
            TaskType::classify("Refactor the database module"),
            TaskType::Code
        );
        assert_eq!(TaskType::classify("Build the API endpoint"), TaskType::Code);
    }

    #[test]
    fn test_classify_research() {
        assert_eq!(
            TaskType::classify("Research competitor pricing"),
            TaskType::Research
        );
        assert_eq!(
            TaskType::classify("Summarize the findings"),
            TaskType::Research
        );
        assert_eq!(
            TaskType::classify("Find all references to this API"),
            TaskType::Research
        );
    }

    #[test]
    fn test_classify_speed() {
        assert_eq!(
            TaskType::classify("Format the JSON output"),
            TaskType::Speed
        );
        assert_eq!(TaskType::classify("List all active users"), TaskType::Speed);
        assert_eq!(TaskType::classify("Convert CSV to JSON"), TaskType::Speed);
    }

    #[test]
    fn test_classify_creative() {
        assert_eq!(
            TaskType::classify("Write a blog post about AI"),
            TaskType::Creative
        );
        assert_eq!(
            TaskType::classify("Draft the press release"),
            TaskType::Creative
        );
    }

    #[test]
    fn test_classify_review() {
        assert_eq!(
            TaskType::classify("Review the pull request"),
            TaskType::Review
        );
        assert_eq!(
            TaskType::classify("Validate the output format"),
            TaskType::Review
        );
    }

    #[test]
    fn test_classify_general() {
        assert_eq!(TaskType::classify("hello world"), TaskType::General);
    }

    #[test]
    fn test_route_reasoning() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select(TaskType::Reasoning, Complexity::Complex, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("opus"));
    }

    #[test]
    fn test_route_code() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select(TaskType::Code, Complexity::Moderate, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("sonnet"));
    }

    #[test]
    fn test_route_speed() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select(TaskType::Speed, Complexity::Simple, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("haiku"));
    }

    #[test]
    fn test_route_research() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select(TaskType::Research, Complexity::Moderate, 0);
        assert_eq!(route.provider, Provider::Google);
    }

    #[test]
    fn test_simple_task_prefers_speed() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        // Even a code task that is simple should get the speed model (Haiku)
        let route = router.select(TaskType::Code, Complexity::Simple, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("haiku"));
    }

    #[test]
    fn test_simple_reasoning_stays_strong() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        // Reasoning tasks should use strong model even when simple
        let route = router.select(TaskType::Reasoning, Complexity::Simple, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("opus"));
    }

    #[test]
    fn test_tier_restricted_uses_speed() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        // tier_limit=1 (small only) should always use speed model (Haiku)
        let route = router.select(TaskType::Reasoning, Complexity::Complex, 1);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("haiku"));
    }

    #[test]
    fn test_disabled_routing_uses_default() {
        let mut config = test_config();
        config.enabled = false;
        let router = ModelRouter::new(config, "ollama/llama3.2".into());
        let route = router.select(TaskType::Reasoning, Complexity::Complex, 0);
        assert_eq!(route.provider, Provider::Ollama);
        assert_eq!(route.model, "llama3.2");
    }

    #[test]
    fn test_passthrough_router() {
        let router = ModelRouter::passthrough("anthropic/claude-sonnet-4-20250514".into());
        let route = router.select(TaskType::Code, Complexity::Complex, 0);
        assert_eq!(route.provider, Provider::Anthropic);
        assert!(route.model.contains("sonnet"));
    }

    #[test]
    fn test_role_coordinator() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select_for_role("coordinator");
        assert!(route.model.contains("opus"));
    }

    #[test]
    fn test_role_worker() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select_for_role("worker");
        assert!(route.model.contains("sonnet"));
    }

    #[test]
    fn test_role_reviewer() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select_for_role("reviewer");
        assert!(route.model.contains("sonnet"));
    }

    #[test]
    fn test_role_unknown() {
        let router = ModelRouter::new(test_config(), "ollama/llama3.2".into());
        let route = router.select_for_role("unknown");
        assert_eq!(route.provider, Provider::Ollama);
    }

    #[test]
    fn test_parse_model_string() {
        let (p, m) = parse_model_string("anthropic/claude-opus-4-20250514");
        assert_eq!(p, Provider::Anthropic);
        assert_eq!(m, "claude-opus-4-20250514");

        let (p, m) = parse_model_string("groq/llama-3.3-70b-versatile");
        assert_eq!(p, Provider::Groq);
        assert_eq!(m, "llama-3.3-70b-versatile");

        let (p, m) = parse_model_string("llama3.2");
        assert_eq!(p, Provider::Ollama);
        assert_eq!(m, "llama3.2");
    }

    // ------------------------------------------------------------------
    // Multi-signal classification tests
    // ------------------------------------------------------------------

    #[test]
    fn test_context_classify_code_with_code_context() {
        let ctx = ClassificationContext::new(
            "Here's the function I'm stuck on:\n```rust\nfn foo() {}\n```\nwhat's wrong?",
        );
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::Code);
    }

    #[test]
    fn test_context_classify_hint_overrides() {
        let ctx = ClassificationContext::new("write something nice").with_hint("code");
        // "write" would normally score Creative; hint pushes to Code.
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::Code);
    }

    #[test]
    fn test_context_classify_short_lean_speed() {
        let ctx = ClassificationContext::new("list users");
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::Speed);
    }

    #[test]
    fn test_context_classify_long_lean_reasoning() {
        let msg = "I need to analyze the trade-offs between our current microservice \
            architecture and a potential monolith. We've got scaling concerns in the order \
            service, deployment complexity across 14 repos, and the team is split on whether \
            we should invest in better tooling or simplify the architecture. Please help me \
            think through this decision — what questions should we ask first? What data would \
            change our minds? How do we avoid the sunk-cost trap here?";
        let ctx = ClassificationContext::new(msg);
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::Reasoning);
    }

    #[test]
    fn test_context_classify_tool_calls_push_code() {
        // Ambiguous message, but 5 tool calls already ⇒ we're deep in code work.
        let ctx = ClassificationContext::new("what now?").with_tool_calls(5);
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::Code);
    }

    #[test]
    fn test_context_classify_empty_falls_back_to_general() {
        let ctx = ClassificationContext::new("hello there");
        assert_eq!(TaskType::classify_with_context(&ctx), TaskType::General);
    }

    #[test]
    fn test_complexity_infer_simple() {
        let ctx = ClassificationContext::new("just rename the file");
        assert_eq!(Complexity::infer(&ctx), Complexity::Simple);
    }

    #[test]
    fn test_complexity_infer_moderate() {
        let ctx = ClassificationContext::new(
            "Can you refactor the parser module so it handles escaped quotes? \
             There are a few edge cases in the existing tests that need updating too.",
        );
        // ~30 words + code context → moderate
        assert_eq!(Complexity::infer(&ctx), Complexity::Moderate);
    }

    #[test]
    fn test_complexity_infer_complex() {
        let msg = "We need a multi-step migration: first move the auth tables across crates, \
            then refactor the whole session layer, then update every call site in the \
            handlers. It's complicated because several cross-crate dependencies will break. \
            End-to-end we're probably looking at changes in at least 10 files across 4 crates. \
            I want to think through the trade-offs before touching anything.";
        let ctx = ClassificationContext::new(msg).with_tool_calls(4);
        assert_eq!(Complexity::infer(&ctx), Complexity::Complex);
    }

    #[test]
    fn test_complexity_infer_history_depth() {
        // Short message but deep history → should bump complexity.
        let ctx = ClassificationContext::new("continue")
            .with_history_len(50)
            .with_tool_calls(3);
        assert!(matches!(
            Complexity::infer(&ctx),
            Complexity::Moderate | Complexity::Complex
        ));
    }

    #[test]
    fn test_classify_full_roundtrip() {
        let ctx = ClassificationContext::new("implement a quick sort function in rust");
        let (task, complexity) = classify_full(&ctx);
        assert_eq!(task, TaskType::Code);
        // Short-ish code request → Simple or Moderate
        assert!(matches!(
            complexity,
            Complexity::Simple | Complexity::Moderate
        ));
    }

    #[test]
    fn test_score_keywords_basic() {
        assert_eq!(score_keywords("hello world", &["hello", "foo"]), 1);
        assert_eq!(score_keywords("", &["a", "b"]), 0);
        assert_eq!(score_keywords("abc abc", &["abc"]), 1); // `contains`, not count
    }

    #[test]
    fn test_detect_code_context() {
        assert!(detect_code_context("run `cargo test`"));
        assert!(detect_code_context("see main.rs"));
        assert!(detect_code_context("```python\ndef f(): pass\n```"));
        assert!(!detect_code_context("hello there"));
    }

    #[test]
    fn test_context_builder_chains() {
        let ctx = ClassificationContext::new("hi")
            .with_tool_calls(2)
            .with_history_len(5)
            .with_hint("code")
            .with_code_context(true);
        assert_eq!(ctx.tool_calls, 2);
        assert_eq!(ctx.history_len, 5);
        assert_eq!(ctx.hint.as_deref(), Some("code"));
        assert!(ctx.has_code_context);
    }

    #[test]
    fn test_missing_config_falls_back() {
        let config = ModelRoutingConfig {
            enabled: true,
            reasoning: Some("anthropic/claude-opus-4-20250514".into()),
            ..Default::default()
        };
        let router = ModelRouter::new(config, "ollama/llama3.2".into());

        // Reasoning has a configured model
        let route = router.select(TaskType::Reasoning, Complexity::Complex, 0);
        assert!(route.model.contains("opus"));

        // Code has no config → falls back to default
        let route = router.select(TaskType::Code, Complexity::Complex, 0);
        assert_eq!(route.provider, Provider::Ollama);
    }
}
