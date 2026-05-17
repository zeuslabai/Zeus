/// Zeus Council — multi-model LLM ensemble with 3-stage pipeline.
///
/// Architecture:
///   Stage 1: All models produce independent opinions (parallel)
///   Stage 2: Models review anonymized peer responses and rank them
///   Stage 3: Chairman synthesizes everything into a final answer

pub mod anonymizer;
pub mod pipeline;
pub mod ranking;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for a council session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilConfig {
    /// Model IDs to include in the council (e.g. "anthropic/claude-sonnet-4", "openai/gpt-4o")
    pub models: Vec<String>,
    /// Model ID of the chairman — synthesizes the final answer
    pub chairman: String,
    /// Timeout per LLM call in seconds
    pub timeout_secs: u64,
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            models: vec![
                "anthropic/claude-sonnet-4-20250514".into(),
                "openai/gpt-4o".into(),
                "google/gemini-2.0-flash".into(),
            ],
            chairman: "anthropic/claude-sonnet-4-20250514".into(),
            timeout_secs: 60,
        }
    }
}

/// One model's raw response from stage 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    /// Internal model identifier
    pub model_id: String,
    /// Anonymized label assigned during stage 2 (e.g. "Model A")
    pub label: String,
    /// The model's response text
    pub response: String,
    /// Tokens used (input + output)
    pub tokens: usize,
    /// Wall-clock latency in milliseconds
    pub latency_ms: u64,
}

/// One model's review of other models' responses from stage 2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReview {
    /// The reviewing model's ID
    pub reviewer_id: String,
    /// Raw review text (includes rankings)
    pub review_text: String,
    /// Parsed ranking: label → score (higher = better)
    pub rankings: std::collections::HashMap<String, f32>,
}

/// Full session state — populated progressively through the 3 stages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilSession {
    pub config: CouncilConfig,
    /// Stage 1 outputs
    pub results: Vec<ModelResponse>,
    /// Stage 2 outputs
    pub reviews: Vec<ModelReview>,
    /// Stage 3 output
    pub final_answer: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl CouncilSession {
    pub fn new(config: CouncilConfig) -> Self {
        Self {
            config,
            results: Vec::new(),
            reviews: Vec::new(),
            final_answer: String::new(),
            started_at: Utc::now(),
            finished_at: None,
        }
    }
}

/// Final result returned to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilResult {
    pub final_answer: String,
    pub session: CouncilSession,
}
