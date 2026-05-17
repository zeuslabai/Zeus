//! Critic Module - Evaluation feedback loop
//!
//! Evaluates execution outcomes to determine quality, timing, and error patterns.
//! The critic provides the "did it work?" assessment that feeds into the learning
//! loop and strategy adjustment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

// ============================================================================
// Types
// ============================================================================

/// Outcome classification for a task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskOutcome {
    Success,
    PartialSuccess { details: String },
    Failure { reason: String },
}

/// Timing evaluation for an execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingEval {
    /// Actual execution time in milliseconds
    pub actual_ms: u64,
    /// Expected time from learning engine (if available)
    pub expected_ms: Option<u64>,
    /// Ratio of actual to expected (if expected is known)
    pub ratio: Option<f32>,
}

/// Analysis of errors encountered during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorAnalysis {
    /// Number of retries attempted
    pub retry_count: usize,
    /// Types of errors encountered
    pub error_types: Vec<String>,
    /// Inferred root cause (if determinable)
    pub root_cause: Option<String>,
}

/// A complete evaluation of a task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    /// Unique evaluation ID
    pub id: String,
    /// Associated goal ID (if applicable)
    pub goal_id: Option<String>,
    /// Description of the task that was evaluated
    pub task_description: String,
    /// Overall outcome
    pub outcome: TaskOutcome,
    /// Timing evaluation
    pub timing: TimingEval,
    /// Error analysis (if errors occurred)
    pub error_analysis: Option<ErrorAnalysis>,
    /// Overall quality score (0.0 - 1.0)
    pub quality_score: f32,
    /// Suggested improvements
    pub improvements: Vec<String>,
    /// When this evaluation was created
    pub timestamp: DateTime<Utc>,
}

/// Input context for evaluating an execution
pub struct ExecutionContext {
    /// Description of what was executed
    pub task_description: String,
    /// Whether the execution succeeded
    pub success: bool,
    /// Whether it partially succeeded (completed some but not all goals)
    pub partial: bool,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Expected duration (from feedback loop / learning)
    pub expected_ms: Option<u64>,
    /// Number of retries that occurred
    pub retry_count: usize,
    /// Error messages encountered (empty if no errors)
    pub errors: Vec<String>,
    /// Number of tool calls executed
    pub tool_call_count: usize,
    /// Associated goal ID
    pub goal_id: Option<String>,
}

// ============================================================================
// CriticEngine
// ============================================================================

/// The Critic Engine evaluates execution outcomes
pub struct CriticEngine {
    /// Ring buffer of recent evaluations (most recent last)
    evaluations: RwLock<Vec<Evaluation>>,
    /// Maximum evaluations to keep in memory
    max_evaluations: usize,
}

impl CriticEngine {
    /// Create a new critic engine
    pub fn new() -> Self {
        Self {
            evaluations: RwLock::new(Vec::new()),
            max_evaluations: 200,
        }
    }

    /// Evaluate an execution outcome
    pub fn evaluate(&self, ctx: &ExecutionContext) -> Evaluation {
        let outcome = self.classify_outcome(ctx);
        let timing = self.evaluate_timing(ctx);
        let error_analysis = self.analyze_errors(ctx);
        let quality_score = self.compute_quality(&outcome, &timing, &error_analysis);
        let improvements = self.suggest_improvements(ctx, &outcome, &timing, &error_analysis);

        let evaluation = Evaluation {
            id: ulid::Ulid::new().to_string(),
            goal_id: ctx.goal_id.clone(),
            task_description: ctx.task_description.clone(),
            outcome,
            timing,
            error_analysis,
            quality_score,
            improvements,
            timestamp: Utc::now(),
        };

        // Store in ring buffer
        if let Ok(mut evals) = self.evaluations.write() {
            evals.push(evaluation.clone());
            if evals.len() > self.max_evaluations {
                evals.remove(0);
            }
        }

        evaluation
    }

    /// Get recent evaluations
    pub fn recent_evaluations(&self, limit: usize) -> Vec<Evaluation> {
        self.evaluations
            .read()
            .map(|evals| evals.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Get the average quality score across all evaluations
    pub fn average_quality(&self) -> f32 {
        self.evaluations
            .read()
            .map(|evals| {
                if evals.is_empty() {
                    0.0
                } else {
                    evals.iter().map(|e| e.quality_score).sum::<f32>() / evals.len() as f32
                }
            })
            .unwrap_or(0.0)
    }

    /// Get the most common failure types and their counts
    pub fn common_failures(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();

        if let Ok(evals) = self.evaluations.read() {
            for eval in evals.iter() {
                if let Some(ref err) = eval.error_analysis {
                    for error_type in &err.error_types {
                        *counts.entry(error_type.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted
    }

    /// Get the total number of evaluations stored
    pub fn evaluation_count(&self) -> usize {
        self.evaluations.read().map(|e| e.len()).unwrap_or(0)
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    fn classify_outcome(&self, ctx: &ExecutionContext) -> TaskOutcome {
        if ctx.partial {
            TaskOutcome::PartialSuccess {
                details: if ctx.errors.is_empty() {
                    "Completed with partial results".to_string()
                } else {
                    format!("Completed with {} errors", ctx.errors.len())
                },
            }
        } else if ctx.success && ctx.errors.is_empty() {
            TaskOutcome::Success
        } else if ctx.success && !ctx.errors.is_empty() {
            TaskOutcome::PartialSuccess {
                details: format!("Completed with {} errors", ctx.errors.len()),
            }
        } else {
            TaskOutcome::Failure {
                reason: ctx
                    .errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Unknown failure".to_string()),
            }
        }
    }

    fn evaluate_timing(&self, ctx: &ExecutionContext) -> TimingEval {
        let ratio = ctx.expected_ms.map(|expected| {
            if expected == 0 {
                1.0
            } else {
                ctx.duration_ms as f32 / expected as f32
            }
        });

        TimingEval {
            actual_ms: ctx.duration_ms,
            expected_ms: ctx.expected_ms,
            ratio,
        }
    }

    fn analyze_errors(&self, ctx: &ExecutionContext) -> Option<ErrorAnalysis> {
        if ctx.errors.is_empty() && ctx.retry_count == 0 {
            return None;
        }

        let error_types: Vec<String> = ctx
            .errors
            .iter()
            .map(|e| categorize_error(e))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let root_cause = if ctx.errors.len() == 1 {
            Some(ctx.errors[0].clone())
        } else if error_types.len() == 1 {
            Some(format!(
                "Repeated {} error ({} occurrences)",
                error_types[0],
                ctx.errors.len()
            ))
        } else {
            None
        };

        Some(ErrorAnalysis {
            retry_count: ctx.retry_count,
            error_types,
            root_cause,
        })
    }

    fn compute_quality(
        &self,
        outcome: &TaskOutcome,
        timing: &TimingEval,
        errors: &Option<ErrorAnalysis>,
    ) -> f32 {
        // Base score from outcome
        let mut score = match outcome {
            TaskOutcome::Success => 1.0,
            TaskOutcome::PartialSuccess { .. } => 0.6,
            TaskOutcome::Failure { .. } => 0.1,
        };

        // Timing penalty: if > 2x expected, penalize (check worst case first)
        if let Some(ratio) = timing.ratio {
            if ratio > 3.0 {
                score *= 0.6;
            } else if ratio > 2.0 {
                score *= 0.8;
            }
        }

        // Error/retry penalty
        if let Some(err) = errors
            && err.retry_count > 0
        {
            score *= 1.0 - (err.retry_count as f32 * 0.1).min(0.3);
        }

        score.clamp(0.0, 1.0)
    }

    fn suggest_improvements(
        &self,
        ctx: &ExecutionContext,
        _outcome: &TaskOutcome,
        timing: &TimingEval,
        errors: &Option<ErrorAnalysis>,
    ) -> Vec<String> {
        let mut improvements = Vec::new();

        if let Some(err) = errors {
            if err.retry_count > 0 {
                improvements.push(format!(
                    "Optimize tool parameters to reduce retries (had {} retries)",
                    err.retry_count
                ));
            }
            for error_type in &err.error_types {
                match error_type.as_str() {
                    "timeout" => improvements.push(
                        "Consider simpler approach or breaking task into smaller steps".to_string(),
                    ),
                    "network" => {
                        improvements.push("Add retry logic for network operations".to_string())
                    }
                    "permission" => improvements
                        .push("Verify permissions before attempting operation".to_string()),
                    _ => {}
                }
            }
        }

        if let Some(ratio) = timing.ratio
            && ratio > 2.0
        {
            improvements.push("Consider a simpler approach to reduce execution time".to_string());
        }

        if ctx.tool_call_count > 10 {
            improvements.push(format!(
                "High tool call count ({}). Consider consolidating operations.",
                ctx.tool_call_count
            ));
        }

        improvements
    }
}

impl Default for CriticEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Categorize an error message into a type
fn categorize_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".to_string()
    } else if lower.contains("network") || lower.contains("connection") || lower.contains("dns") {
        "network".to_string()
    } else if lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("forbidden")
    {
        "permission".to_string()
    } else if lower.contains("not found") || lower.contains("404") {
        "not_found".to_string()
    } else if lower.contains("parse") || lower.contains("invalid") || lower.contains("syntax") {
        "parse".to_string()
    } else if lower.contains("rate limit") || lower.contains("429") || lower.contains("throttl") {
        "rate_limit".to_string()
    } else {
        "other".to_string()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn success_ctx() -> ExecutionContext {
        ExecutionContext {
            task_description: "Run tests".to_string(),
            success: true,
            partial: false,
            duration_ms: 500,
            expected_ms: Some(600),
            retry_count: 0,
            errors: vec![],
            tool_call_count: 3,
            goal_id: None,
        }
    }

    fn failure_ctx() -> ExecutionContext {
        ExecutionContext {
            task_description: "Deploy app".to_string(),
            success: false,
            partial: false,
            duration_ms: 2000,
            expected_ms: Some(1000),
            retry_count: 2,
            errors: vec!["Connection timed out".to_string()],
            tool_call_count: 5,
            goal_id: Some("goal-1".to_string()),
        }
    }

    #[test]
    fn test_critic_creation() {
        let critic = CriticEngine::new();
        assert_eq!(critic.evaluation_count(), 0);
        assert_eq!(critic.average_quality(), 0.0);
    }

    #[test]
    fn test_evaluate_success() {
        let critic = CriticEngine::new();
        let eval = critic.evaluate(&success_ctx());

        assert!(matches!(eval.outcome, TaskOutcome::Success));
        assert!(eval.quality_score > 0.9);
        assert!(eval.error_analysis.is_none());
        assert!(eval.improvements.is_empty());
        assert_eq!(eval.timing.actual_ms, 500);
    }

    #[test]
    fn test_evaluate_failure() {
        let critic = CriticEngine::new();
        let eval = critic.evaluate(&failure_ctx());

        assert!(matches!(eval.outcome, TaskOutcome::Failure { .. }));
        assert!(eval.quality_score < 0.2);
        assert!(eval.error_analysis.is_some());
        assert!(!eval.improvements.is_empty());
        assert_eq!(eval.goal_id, Some("goal-1".to_string()));
    }

    #[test]
    fn test_evaluate_partial_success() {
        let critic = CriticEngine::new();
        let ctx = ExecutionContext {
            task_description: "Process data".to_string(),
            success: true,
            partial: true,
            duration_ms: 1000,
            expected_ms: None,
            retry_count: 0,
            errors: vec![],
            tool_call_count: 2,
            goal_id: None,
        };

        let eval = critic.evaluate(&ctx);
        assert!(matches!(eval.outcome, TaskOutcome::PartialSuccess { .. }));
        assert!(eval.quality_score > 0.5 && eval.quality_score < 0.8);
    }

    #[test]
    fn test_timing_evaluation() {
        let critic = CriticEngine::new();

        // Within expected time
        let eval = critic.evaluate(&success_ctx());
        assert!(eval.timing.ratio.expect("operation should succeed") < 1.5);

        // Moderately slow execution (2x-3x expected)
        let mut slow_ctx = success_ctx();
        slow_ctx.duration_ms = 1500;
        slow_ctx.expected_ms = Some(600);
        let eval_moderate = critic.evaluate(&slow_ctx);
        assert!(
            eval_moderate
                .timing
                .ratio
                .expect("operation should succeed")
                > 2.0
        );
        assert!(
            eval_moderate
                .timing
                .ratio
                .expect("operation should succeed")
                < 3.0
        );
        // Should get 0.8x penalty
        assert!((eval_moderate.quality_score - 0.8).abs() < 0.01);

        // Very slow execution (>3x expected) should get worse penalty
        let mut very_slow_ctx = success_ctx();
        very_slow_ctx.duration_ms = 2000;
        very_slow_ctx.expected_ms = Some(500);
        let eval_very_slow = critic.evaluate(&very_slow_ctx);
        assert!(
            eval_very_slow
                .timing
                .ratio
                .expect("operation should succeed")
                > 3.0
        );
        // Should get 0.6x penalty (worse than moderate)
        assert!((eval_very_slow.quality_score - 0.6).abs() < 0.01);
        assert!(eval_very_slow.quality_score < eval_moderate.quality_score);
    }

    #[test]
    fn test_error_analysis() {
        let critic = CriticEngine::new();
        let eval = critic.evaluate(&failure_ctx());

        let err = eval.error_analysis.expect("operation should succeed");
        assert_eq!(err.retry_count, 2);
        assert!(err.error_types.contains(&"timeout".to_string()));
        assert!(err.root_cause.is_some());
    }

    #[test]
    fn test_recent_evaluations() {
        let critic = CriticEngine::new();

        critic.evaluate(&success_ctx());
        critic.evaluate(&failure_ctx());
        critic.evaluate(&success_ctx());

        let recent = critic.recent_evaluations(2);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].task_description, "Run tests");
    }

    #[test]
    fn test_average_quality() {
        let critic = CriticEngine::new();

        critic.evaluate(&success_ctx());
        critic.evaluate(&success_ctx());
        critic.evaluate(&failure_ctx());

        let avg = critic.average_quality();
        assert!(avg > 0.3 && avg < 0.9);
    }

    #[test]
    fn test_common_failures() {
        let critic = CriticEngine::new();

        // Record multiple failures with same error type
        for _ in 0..3 {
            critic.evaluate(&failure_ctx());
        }

        let failures = critic.common_failures();
        assert!(!failures.is_empty());
        assert_eq!(failures[0].0, "timeout");
        assert_eq!(failures[0].1, 3);
    }

    #[test]
    fn test_ring_buffer_limit() {
        let critic = CriticEngine {
            evaluations: RwLock::new(Vec::new()),
            max_evaluations: 5,
        };

        for _ in 0..10 {
            critic.evaluate(&success_ctx());
        }

        assert_eq!(critic.evaluation_count(), 5);
    }

    #[test]
    fn test_categorize_error() {
        assert_eq!(categorize_error("Connection timed out"), "timeout");
        assert_eq!(categorize_error("Network unreachable"), "network");
        assert_eq!(categorize_error("Permission denied"), "permission");
        assert_eq!(categorize_error("File not found"), "not_found");
        assert_eq!(categorize_error("Parse error: invalid JSON"), "parse");
        assert_eq!(categorize_error("Rate limit exceeded"), "rate_limit");
        assert_eq!(categorize_error("Something unexpected"), "other");
    }

    #[test]
    fn test_improvements_for_retries() {
        let critic = CriticEngine::new();
        let mut ctx = success_ctx();
        ctx.retry_count = 3;
        ctx.errors = vec!["Temporary error".to_string()];

        let eval = critic.evaluate(&ctx);
        assert!(eval.improvements.iter().any(|i| i.contains("retries")));
    }

    #[test]
    fn test_improvements_for_high_tool_count() {
        let critic = CriticEngine::new();
        let mut ctx = success_ctx();
        ctx.tool_call_count = 15;

        let eval = critic.evaluate(&ctx);
        assert!(
            eval.improvements
                .iter()
                .any(|i| i.contains("tool call count"))
        );
    }

    #[test]
    fn test_evaluate_with_zero_duration() {
        let critic = CriticEngine::new();
        let ctx = ExecutionContext {
            task_description: "Instant task".to_string(),
            success: true,
            partial: false,
            duration_ms: 0,
            expected_ms: Some(100),
            retry_count: 0,
            errors: vec![],
            tool_call_count: 1,
            goal_id: None,
        };

        let eval = critic.evaluate(&ctx);
        assert!(matches!(eval.outcome, TaskOutcome::Success));
        assert_eq!(eval.timing.actual_ms, 0);
        // ratio = 0 / 100 = 0.0, which is fine (no penalty)
        assert!(eval.timing.ratio.expect("operation should succeed") < 1.0);
    }

    #[test]
    fn test_evaluate_with_no_expected_time() {
        let critic = CriticEngine::new();
        let ctx = ExecutionContext {
            task_description: "No expected time".to_string(),
            success: true,
            partial: false,
            duration_ms: 5000,
            expected_ms: None,
            retry_count: 0,
            errors: vec![],
            tool_call_count: 2,
            goal_id: None,
        };

        let eval = critic.evaluate(&ctx);
        assert!(matches!(eval.outcome, TaskOutcome::Success));
        assert!(eval.timing.expected_ms.is_none());
        assert!(eval.timing.ratio.is_none());
        // No timing penalty, so quality should be 1.0
        assert!((eval.quality_score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_evaluate_high_retry_count() {
        let critic = CriticEngine::new();
        let ctx = ExecutionContext {
            task_description: "Many retries".to_string(),
            success: true,
            partial: false,
            duration_ms: 3000,
            expected_ms: Some(3000),
            retry_count: 5,
            errors: vec!["Transient error".to_string()],
            tool_call_count: 6,
            goal_id: None,
        };

        let eval = critic.evaluate(&ctx);
        // Success with errors => PartialSuccess
        assert!(matches!(eval.outcome, TaskOutcome::PartialSuccess { .. }));
        // retry_count = 5 => penalty = min(5*0.1, 0.3) = 0.3 => score *= 0.7
        // Base for PartialSuccess = 0.6, so 0.6 * 0.7 = 0.42
        assert!(eval.quality_score < 0.6);
        assert!(eval.error_analysis.is_some());
        let err = eval.error_analysis.expect("operation should succeed");
        assert_eq!(err.retry_count, 5);
    }

    #[test]
    fn test_evaluate_multiple_errors() {
        let critic = CriticEngine::new();
        let ctx = ExecutionContext {
            task_description: "Multiple errors".to_string(),
            success: false,
            partial: false,
            duration_ms: 1000,
            expected_ms: Some(500),
            retry_count: 3,
            errors: vec![
                "Connection timed out".to_string(),
                "DNS resolution failed".to_string(),
                "Permission denied".to_string(),
            ],
            tool_call_count: 4,
            goal_id: None,
        };

        let eval = critic.evaluate(&ctx);
        assert!(matches!(eval.outcome, TaskOutcome::Failure { .. }));
        let err = eval.error_analysis.expect("operation should succeed");
        // Three different error types: timeout, network, permission
        assert!(err.error_types.len() >= 2);
        // Multiple different error types => root_cause is None
        assert!(err.root_cause.is_none());
    }

    #[test]
    fn test_quality_score_ranges() {
        let critic = CriticEngine::new();

        // Success should be high
        let s_eval = critic.evaluate(&success_ctx());
        assert!(s_eval.quality_score >= 0.0 && s_eval.quality_score <= 1.0);

        // Failure should be low
        let f_eval = critic.evaluate(&failure_ctx());
        assert!(f_eval.quality_score >= 0.0 && f_eval.quality_score <= 1.0);

        // Edge case: max retries on failure
        let ctx = ExecutionContext {
            task_description: "Worst case".to_string(),
            success: false,
            partial: false,
            duration_ms: 100000,
            expected_ms: Some(100),
            retry_count: 100,
            errors: vec!["catastrophic".to_string()],
            tool_call_count: 50,
            goal_id: None,
        };
        let worst = critic.evaluate(&ctx);
        assert!(worst.quality_score >= 0.0 && worst.quality_score <= 1.0);
    }

    #[test]
    fn test_categorize_error_timeout() {
        assert_eq!(categorize_error("Request timed out after 30s"), "timeout");
        assert_eq!(categorize_error("Operation timeout exceeded"), "timeout");
    }

    #[test]
    fn test_categorize_error_network() {
        assert_eq!(categorize_error("Connection refused"), "network");
        assert_eq!(categorize_error("DNS lookup failed"), "network");
        assert_eq!(categorize_error("Network unreachable"), "network");
    }

    #[test]
    fn test_categorize_error_permission() {
        assert_eq!(
            categorize_error("Permission denied: /etc/shadow"),
            "permission"
        );
        assert_eq!(categorize_error("Access forbidden"), "permission");
        assert_eq!(categorize_error("Operation denied by policy"), "permission");
    }

    #[test]
    fn test_categorize_error_not_found() {
        assert_eq!(categorize_error("File not found: config.toml"), "not_found");
        assert_eq!(categorize_error("HTTP 404 response"), "not_found");
    }

    #[test]
    fn test_categorize_error_rate_limit() {
        assert_eq!(
            categorize_error("Rate limit exceeded, retry after 60s"),
            "rate_limit"
        );
        assert_eq!(categorize_error("HTTP 429 Too Many Requests"), "rate_limit");
        assert_eq!(categorize_error("Request throttled"), "rate_limit");
    }

    #[test]
    fn test_average_quality_single_eval() {
        let critic = CriticEngine::new();
        critic.evaluate(&success_ctx());

        let avg = critic.average_quality();
        // Single success evaluation, quality ~1.0
        assert!(avg > 0.9);
        assert_eq!(critic.evaluation_count(), 1);
    }

    #[test]
    fn test_common_failures_empty() {
        let critic = CriticEngine::new();
        // No evaluations yet
        let failures = critic.common_failures();
        assert!(failures.is_empty());

        // Add only successes (no errors)
        critic.evaluate(&success_ctx());
        critic.evaluate(&success_ctx());
        let failures = critic.common_failures();
        assert!(failures.is_empty());
    }
}
