//! Feedback Loop - Strategy learning from execution outcomes
//!
//! Closes the feedback loop: execute → evaluate → learn → adjust strategy.
//! Records which strategies work best for which intent types and calibrates
//! time estimates from actual execution data.

use crate::autonomy::Decision;
use crate::intent::IntentAnalysis;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::debug;

// ============================================================================
// Types
// ============================================================================

/// Tracks strategy effectiveness and time estimates for strategy learning
pub struct FeedbackLoop {
    preferences: RwLock<StrategyPreferences>,
}

/// Aggregated strategy preferences learned from execution outcomes
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyPreferences {
    /// For each intent type string, the preferred strategy and its stats
    pub preferred_strategies: HashMap<String, StrategyRecord>,
    /// Time estimates calibrated from actual execution
    pub time_estimates: HashMap<String, TimeEstimate>,
}

/// Record of a strategy's effectiveness for a particular intent type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyRecord {
    /// Strategy name (matches Decision variant: "respond_directly", "plan_and_execute", etc.)
    pub strategy: String,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f32,
    /// Number of executions tracked
    pub sample_count: usize,
    /// Average processing time in ms
    pub avg_time_ms: f64,
}

/// Calibrated time estimate for a task type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeEstimate {
    pub task_type: String,
    pub estimated_ms: u64,
    pub actual_avg_ms: u64,
    pub sample_count: usize,
}

// ============================================================================
// FeedbackLoop
// ============================================================================

impl FeedbackLoop {
    /// Create a new feedback loop
    pub fn new() -> Self {
        Self {
            preferences: RwLock::new(StrategyPreferences::default()),
        }
    }

    /// Record the outcome of a decision for future strategy learning
    pub fn record_outcome(
        &self,
        intent: &IntentAnalysis,
        decision: &Decision,
        success: bool,
        duration_ms: u64,
    ) {
        let intent_type = intent.intent.to_string();
        let strategy = decision_to_strategy(decision);

        let mut prefs = match self.preferences.write() {
            Ok(p) => p,
            Err(_) => return,
        };

        // Update strategy record
        let record = prefs
            .preferred_strategies
            .entry(intent_type.clone())
            .or_insert_with(|| StrategyRecord {
                strategy: strategy.clone(),
                success_rate: 0.0,
                sample_count: 0,
                avg_time_ms: 0.0,
            });

        // Exponential moving average for success rate and time
        let n = record.sample_count as f64;
        let alpha = if n < 10.0 { 1.0 / (n + 1.0) } else { 0.1 };

        record.success_rate = (1.0 - alpha as f32) * record.success_rate
            + alpha as f32 * if success { 1.0 } else { 0.0 };
        record.avg_time_ms = (1.0 - alpha) * record.avg_time_ms + alpha * duration_ms as f64;
        record.sample_count += 1;

        // If the current strategy is working better, adopt it
        if strategy != record.strategy && success && record.success_rate < 0.5 {
            debug!(
                intent = %intent_type,
                old = %record.strategy,
                new = %strategy,
                "Switching preferred strategy"
            );
            record.strategy = strategy;
        }

        // Update time estimate
        let estimate = prefs
            .time_estimates
            .entry(intent_type)
            .or_insert_with(|| TimeEstimate {
                task_type: String::new(),
                estimated_ms: duration_ms,
                actual_avg_ms: duration_ms,
                sample_count: 0,
            });
        let en = estimate.sample_count as f64;
        let ealpha = if en < 10.0 { 1.0 / (en + 1.0) } else { 0.1 };
        estimate.actual_avg_ms =
            ((1.0 - ealpha) * estimate.actual_avg_ms as f64 + ealpha * duration_ms as f64) as u64;
        estimate.sample_count += 1;
    }

    /// Suggest a strategy override based on learned preferences.
    /// Returns None if not enough data or the default is fine.
    pub fn suggest_strategy(&self, intent: &IntentAnalysis) -> Option<Decision> {
        let prefs = self.preferences.read().ok()?;
        let intent_type = intent.intent.to_string();
        let record = prefs.preferred_strategies.get(&intent_type)?;

        // Only suggest if we have enough samples and the strategy differs
        if record.sample_count < 5 || record.success_rate < 0.6 {
            return None;
        }

        strategy_to_decision(&record.strategy)
    }

    /// Get the estimated time for a task type (if known)
    pub fn estimated_time(&self, task_type: &str) -> Option<u64> {
        let prefs = self.preferences.read().ok()?;
        prefs
            .time_estimates
            .get(task_type)
            .filter(|e| e.sample_count >= 3)
            .map(|e| e.actual_avg_ms)
    }

    /// Get current strategy preferences (for inspection/debugging)
    pub fn preferences(&self) -> StrategyPreferences {
        self.preferences
            .read()
            .map(|p| p.clone())
            .unwrap_or_default()
    }
}

impl Default for FeedbackLoop {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn decision_to_strategy(decision: &Decision) -> String {
    match decision {
        Decision::RespondDirectly(_) => "respond_directly".to_string(),
        Decision::ExecuteTool(_) => "execute_tool".to_string(),
        Decision::PlanAndExecute => "plan_and_execute".to_string(),
        Decision::AskUser(_) => "ask_user".to_string(),
        Decision::Delegate(_) => "delegate".to_string(),
        Decision::Reflect => "reflect".to_string(),
        Decision::SpawnAgents(_) => "spawn_agents".to_string(),
    }
}

fn strategy_to_decision(strategy: &str) -> Option<Decision> {
    match strategy {
        "respond_directly" => Some(Decision::RespondDirectly("learned strategy".to_string())),
        "plan_and_execute" => Some(Decision::PlanAndExecute),
        "reflect" => Some(Decision::Reflect),
        // Don't auto-suggest strategies that need specific parameters
        _ => None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Intent, IntentAnalysis, TaskComplexity};

    fn make_analysis(intent: Intent) -> IntentAnalysis {
        IntentAnalysis {
            intent,
            complexity: TaskComplexity::Simple,
            confidence: 0.9,
            suggested_tools: vec![],
            requires_confirmation: false,
            reasoning: "test".to_string(),
        }
    }

    #[test]
    fn test_feedback_loop_creation() {
        let fb = FeedbackLoop::new();
        let prefs = fb.preferences();
        assert!(prefs.preferred_strategies.is_empty());
        assert!(prefs.time_estimates.is_empty());
    }

    #[test]
    fn test_record_outcome() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        fb.record_outcome(&analysis, &decision, true, 100);

        let prefs = fb.preferences();
        assert_eq!(prefs.preferred_strategies.len(), 1);
        let record = prefs.preferred_strategies.get("conversation").unwrap();
        assert_eq!(record.sample_count, 1);
        assert!(record.success_rate > 0.9);
    }

    #[test]
    fn test_suggest_strategy_not_enough_data() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        // Record only 3 outcomes (need 5 for suggestion)
        for _ in 0..3 {
            fb.record_outcome(&analysis, &decision, true, 100);
        }

        assert!(fb.suggest_strategy(&analysis).is_none());
    }

    #[test]
    fn test_suggest_strategy_sufficient_data() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        for _ in 0..6 {
            fb.record_outcome(&analysis, &decision, true, 100);
        }

        let suggested = fb.suggest_strategy(&analysis);
        assert!(suggested.is_some());
        assert!(matches!(suggested.unwrap(), Decision::RespondDirectly(_)));
    }

    #[test]
    fn test_estimated_time() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::SimpleQuery);
        let decision = Decision::RespondDirectly("test".to_string());

        // Not enough data yet
        assert!(fb.estimated_time("simple_query").is_none());

        for _ in 0..5 {
            fb.record_outcome(&analysis, &decision, true, 200);
        }

        let est = fb.estimated_time("simple_query");
        assert!(est.is_some());
        // Should be close to 200ms
        let t = est.unwrap();
        assert!(t > 100 && t < 300);
    }

    #[test]
    fn test_time_estimate_moving_average() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ToolUse);
        let decision = Decision::ExecuteTool("shell".to_string());

        // Record 5 fast outcomes, then 5 slow ones
        for _ in 0..5 {
            fb.record_outcome(&analysis, &decision, true, 100);
        }
        for _ in 0..5 {
            fb.record_outcome(&analysis, &decision, true, 500);
        }

        let est = fb.estimated_time("tool_use").unwrap();
        // Should be somewhere between 100 and 500, trending toward 500
        assert!(est > 100 && est < 500);
    }

    #[test]
    fn test_success_rate_tracking() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ComplexTask);
        let decision = Decision::PlanAndExecute;

        // 3 successes, 2 failures
        for _ in 0..3 {
            fb.record_outcome(&analysis, &decision, true, 1000);
        }
        for _ in 0..2 {
            fb.record_outcome(&analysis, &decision, false, 2000);
        }

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("complex_task").unwrap();
        assert_eq!(record.sample_count, 5);
        // Success rate should be around 0.6
        assert!(record.success_rate > 0.3 && record.success_rate < 0.9);
    }

    #[test]
    fn test_low_success_no_suggestion() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ComplexTask);
        let decision = Decision::PlanAndExecute;

        // Record mostly failures
        for _ in 0..6 {
            fb.record_outcome(&analysis, &decision, false, 1000);
        }

        // Should not suggest a strategy with low success rate
        assert!(fb.suggest_strategy(&analysis).is_none());
    }

    #[test]
    fn test_feedback_loop_default() {
        let fb = FeedbackLoop::default();
        let prefs = fb.preferences();
        assert!(prefs.preferred_strategies.is_empty());
        assert!(prefs.time_estimates.is_empty());
    }

    #[test]
    fn test_record_multiple_intent_types() {
        let fb = FeedbackLoop::new();

        let conv = make_analysis(Intent::Conversation);
        let tool = make_analysis(Intent::ToolUse);
        let query = make_analysis(Intent::SimpleQuery);

        fb.record_outcome(
            &conv,
            &Decision::RespondDirectly("test".to_string()),
            true,
            50,
        );
        fb.record_outcome(
            &tool,
            &Decision::ExecuteTool("shell".to_string()),
            true,
            200,
        );
        fb.record_outcome(
            &query,
            &Decision::RespondDirectly("test".to_string()),
            true,
            100,
        );

        let prefs = fb.preferences();
        assert_eq!(prefs.preferred_strategies.len(), 3);
        assert!(prefs.preferred_strategies.contains_key("conversation"));
        assert!(prefs.preferred_strategies.contains_key("tool_use"));
        assert!(prefs.preferred_strategies.contains_key("simple_query"));
    }

    #[test]
    fn test_estimated_time_not_enough_samples() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        // Record only 2 outcomes (need 3 for estimate)
        for _ in 0..2 {
            fb.record_outcome(&analysis, &decision, true, 100);
        }

        assert!(fb.estimated_time("conversation").is_none());
    }

    #[test]
    fn test_estimated_time_exactly_three_samples() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        for _ in 0..3 {
            fb.record_outcome(&analysis, &decision, true, 300);
        }

        let est = fb.estimated_time("conversation");
        assert!(est.is_some());
    }

    #[test]
    fn test_estimated_time_unknown_task_type() {
        let fb = FeedbackLoop::new();
        assert!(fb.estimated_time("nonexistent_type").is_none());
    }

    #[test]
    fn test_suggest_strategy_unknown_intent() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::SystemCommand);
        // No data recorded for SystemCommand
        assert!(fb.suggest_strategy(&analysis).is_none());
    }

    #[test]
    fn test_decision_to_strategy_all_variants() {
        assert_eq!(
            decision_to_strategy(&Decision::RespondDirectly("x".to_string())),
            "respond_directly"
        );
        assert_eq!(
            decision_to_strategy(&Decision::ExecuteTool("x".to_string())),
            "execute_tool"
        );
        assert_eq!(
            decision_to_strategy(&Decision::PlanAndExecute),
            "plan_and_execute"
        );
        assert_eq!(
            decision_to_strategy(&Decision::AskUser("x".to_string())),
            "ask_user"
        );
        assert_eq!(
            decision_to_strategy(&Decision::Delegate("x".to_string())),
            "delegate"
        );
        assert_eq!(decision_to_strategy(&Decision::Reflect), "reflect");
    }

    #[test]
    fn test_strategy_to_decision_known_strategies() {
        assert!(matches!(
            strategy_to_decision("respond_directly"),
            Some(Decision::RespondDirectly(_))
        ));
        assert!(matches!(
            strategy_to_decision("plan_and_execute"),
            Some(Decision::PlanAndExecute)
        ));
        assert!(matches!(
            strategy_to_decision("reflect"),
            Some(Decision::Reflect)
        ));
    }

    #[test]
    fn test_strategy_to_decision_unknown_strategies() {
        // Strategies that need specific parameters should return None
        assert!(strategy_to_decision("execute_tool").is_none());
        assert!(strategy_to_decision("ask_user").is_none());
        assert!(strategy_to_decision("delegate").is_none());
        assert!(strategy_to_decision("nonexistent").is_none());
    }

    #[test]
    fn test_strategy_preferences_serialization() {
        let mut prefs = StrategyPreferences::default();
        prefs.preferred_strategies.insert(
            "conversation".to_string(),
            StrategyRecord {
                strategy: "respond_directly".to_string(),
                success_rate: 0.95,
                sample_count: 10,
                avg_time_ms: 150.0,
            },
        );
        prefs.time_estimates.insert(
            "conversation".to_string(),
            TimeEstimate {
                task_type: "conversation".to_string(),
                estimated_ms: 150,
                actual_avg_ms: 145,
                sample_count: 10,
            },
        );

        let json = serde_json::to_string(&prefs).unwrap();
        let deser: StrategyPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.preferred_strategies.len(), 1);
        assert_eq!(deser.time_estimates.len(), 1);
    }

    #[test]
    fn test_strategy_record_fields() {
        let record = StrategyRecord {
            strategy: "plan_and_execute".to_string(),
            success_rate: 0.8,
            sample_count: 20,
            avg_time_ms: 5000.0,
        };
        assert_eq!(record.strategy, "plan_and_execute");
        assert!((record.success_rate - 0.8).abs() < f32::EPSILON);
        assert_eq!(record.sample_count, 20);
        assert!((record.avg_time_ms - 5000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_record_outcome_zero_duration() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        fb.record_outcome(&analysis, &decision, true, 0);

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("conversation").unwrap();
        assert_eq!(record.sample_count, 1);
        assert!((record.avg_time_ms - 0.0).abs() < 1.0);
    }

    #[test]
    fn test_record_outcome_very_large_duration() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ComplexTask);
        let decision = Decision::PlanAndExecute;

        fb.record_outcome(&analysis, &decision, true, 1_000_000);

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("complex_task").unwrap();
        assert_eq!(record.sample_count, 1);
    }

    #[test]
    fn test_suggest_strategy_plan_and_execute() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ComplexTask);
        let decision = Decision::PlanAndExecute;

        // Record enough successful outcomes
        for _ in 0..8 {
            fb.record_outcome(&analysis, &decision, true, 5000);
        }

        let suggested = fb.suggest_strategy(&analysis);
        assert!(suggested.is_some());
        assert!(matches!(suggested.unwrap(), Decision::PlanAndExecute));
    }

    #[test]
    fn test_record_outcome_alternating_success_failure() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ToolUse);
        let decision = Decision::ExecuteTool("shell".to_string());

        // Alternate success and failure
        for i in 0..10 {
            fb.record_outcome(&analysis, &decision, i % 2 == 0, 100);
        }

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("tool_use").unwrap();
        assert_eq!(record.sample_count, 10);
        // Success rate should be around 0.5
        assert!(
            record.success_rate > 0.2 && record.success_rate < 0.8,
            "Success rate {} should be around 0.5",
            record.success_rate
        );
    }

    #[test]
    fn test_time_estimate_fields() {
        let estimate = TimeEstimate {
            task_type: "test".to_string(),
            estimated_ms: 500,
            actual_avg_ms: 480,
            sample_count: 15,
        };
        assert_eq!(estimate.task_type, "test");
        assert_eq!(estimate.estimated_ms, 500);
        assert_eq!(estimate.actual_avg_ms, 480);
        assert_eq!(estimate.sample_count, 15);
    }

    #[test]
    fn test_multiple_task_types_isolation() {
        let fb = FeedbackLoop::new();

        let conv = make_analysis(Intent::Conversation);
        let tool = make_analysis(Intent::ToolUse);
        let complex = make_analysis(Intent::ComplexTask);

        // Record different intents with different decisions and durations
        fb.record_outcome(
            &conv,
            &Decision::RespondDirectly("chat".to_string()),
            true,
            50,
        );
        fb.record_outcome(
            &tool,
            &Decision::ExecuteTool("shell".to_string()),
            false,
            500,
        );
        fb.record_outcome(&complex, &Decision::PlanAndExecute, true, 5000);

        let prefs = fb.preferences();

        // Verify each intent type has its own isolated record
        let conv_rec = prefs.preferred_strategies.get("conversation").unwrap();
        assert_eq!(conv_rec.sample_count, 1);
        assert!(conv_rec.success_rate > 0.9);

        let tool_rec = prefs.preferred_strategies.get("tool_use").unwrap();
        assert_eq!(tool_rec.sample_count, 1);
        assert!(tool_rec.success_rate < 0.1);

        let complex_rec = prefs.preferred_strategies.get("complex_task").unwrap();
        assert_eq!(complex_rec.sample_count, 1);
        assert!(complex_rec.success_rate > 0.9);
    }

    #[test]
    fn test_suggest_strategy_with_high_success() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        // Record 10 all-success outcomes
        for _ in 0..10 {
            fb.record_outcome(&analysis, &decision, true, 80);
        }

        let suggested = fb.suggest_strategy(&analysis);
        assert!(suggested.is_some());
        assert!(matches!(suggested.unwrap(), Decision::RespondDirectly(_)));
    }

    #[test]
    fn test_time_estimate_with_many_samples() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ToolUse);
        let decision = Decision::ExecuteTool("shell".to_string());

        // Record 15 samples with varying durations
        for i in 0..15 {
            fb.record_outcome(&analysis, &decision, true, 100 + i * 10);
        }

        let est = fb.estimated_time("tool_use");
        assert!(est.is_some());
        let t = est.unwrap();
        // Should be somewhere in the range (moving average of 100..240)
        assert!(
            t > 50 && t < 300,
            "Time estimate {} out of expected range",
            t
        );
    }

    #[test]
    fn test_record_outcome_boundary_duration() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::Conversation);
        let decision = Decision::RespondDirectly("test".to_string());

        // 0ms duration
        fb.record_outcome(&analysis, &decision, true, 0);
        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("conversation").unwrap();
        assert_eq!(record.sample_count, 1);
        assert!((record.avg_time_ms - 0.0).abs() < 1.0);

        // Very large duration
        let analysis2 = make_analysis(Intent::ComplexTask);
        let decision2 = Decision::PlanAndExecute;
        fb.record_outcome(&analysis2, &decision2, true, u64::MAX);
        let prefs2 = fb.preferences();
        let record2 = prefs2.preferred_strategies.get("complex_task").unwrap();
        assert_eq!(record2.sample_count, 1);
    }

    #[test]
    fn test_success_rate_all_success() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::SimpleQuery);
        let decision = Decision::RespondDirectly("test".to_string());

        for _ in 0..20 {
            fb.record_outcome(&analysis, &decision, true, 100);
        }

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("simple_query").unwrap();
        assert_eq!(record.sample_count, 20);
        // With all successes, rate should be very close to 1.0
        assert!(
            record.success_rate > 0.95,
            "Expected near 1.0 success rate, got {}",
            record.success_rate
        );
    }

    #[test]
    fn test_success_rate_all_failure() {
        let fb = FeedbackLoop::new();
        let analysis = make_analysis(Intent::ComplexTask);
        let decision = Decision::PlanAndExecute;

        for _ in 0..20 {
            fb.record_outcome(&analysis, &decision, false, 1000);
        }

        let prefs = fb.preferences();
        let record = prefs.preferred_strategies.get("complex_task").unwrap();
        assert_eq!(record.sample_count, 20);
        // With all failures, rate should be very close to 0.0
        assert!(
            record.success_rate < 0.05,
            "Expected near 0.0 success rate, got {}",
            record.success_rate
        );
    }
}
