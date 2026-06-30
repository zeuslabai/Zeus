//! Confidence Scoring System — ECC-inspired instinct lifecycle
//!
//! Adds a lifecycle layer on top of `Lesson` with four states:
//!
//! ```text
//! Tentative → Established → Confident → Instinct
//!     ↑            ↑           ↑          ↑
//!   new lesson   3+ reinf.   6+ reinf.  10+ reinf.
//!   conf < 0.4   conf ≥ 0.4  conf ≥ 0.7 conf ≥ 0.85
//! ```
//!
//! Instinct-level lessons are injected into the system prompt so
//! the agent automatically applies proven strategies.
//!
//! Time-based decay ensures stale lessons gradually lose confidence
//! and demote back down the lifecycle if not reinforced.

use crate::learning::{LearningEngine, Lesson, LessonCategory};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Lifecycle state for a confidence-scored behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConfidenceState {
    /// Newly learned, unproven — may be wrong
    Tentative,
    /// Reinforced several times, likely correct
    Established,
    /// Strongly reinforced, high confidence
    Confident,
    /// Battle-tested pattern — injected into system prompt
    Instinct,
}

impl std::fmt::Display for ConfidenceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tentative => write!(f, "tentative"),
            Self::Established => write!(f, "established"),
            Self::Confident => write!(f, "confident"),
            Self::Instinct => write!(f, "instinct"),
        }
    }
}

/// Thresholds for lifecycle transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceThresholds {
    /// Minimum confidence to reach Established
    pub established_confidence: f32,
    /// Minimum reinforcements to reach Established
    pub established_reinforcements: u32,
    /// Minimum confidence to reach Confident
    pub confident_confidence: f32,
    /// Minimum reinforcements to reach Confident
    pub confident_reinforcements: u32,
    /// Minimum confidence to reach Instinct
    pub instinct_confidence: f32,
    /// Minimum reinforcements to reach Instinct
    pub instinct_reinforcements: u32,
    /// Confidence decay per day without reinforcement
    pub decay_per_day: f32,
    /// Days before decay starts
    pub decay_grace_days: i64,
}

impl Default for ConfidenceThresholds {
    fn default() -> Self {
        Self {
            established_confidence: 0.4,
            established_reinforcements: 3,
            confident_confidence: 0.7,
            confident_reinforcements: 6,
            instinct_confidence: 0.85,
            instinct_reinforcements: 10,
            decay_per_day: 0.02,
            decay_grace_days: 7,
        }
    }
}

/// Compute the lifecycle state for a lesson given thresholds.
pub fn confidence_state(lesson: &Lesson, thresholds: &ConfidenceThresholds) -> ConfidenceState {
    if lesson.confidence >= thresholds.instinct_confidence
        && lesson.reinforcements >= thresholds.instinct_reinforcements
    {
        ConfidenceState::Instinct
    } else if lesson.confidence >= thresholds.confident_confidence
        && lesson.reinforcements >= thresholds.confident_reinforcements
    {
        ConfidenceState::Confident
    } else if lesson.confidence >= thresholds.established_confidence
        && lesson.reinforcements >= thresholds.established_reinforcements
    {
        ConfidenceState::Established
    } else {
        ConfidenceState::Tentative
    }
}

/// Apply time-based decay to a lesson's confidence.
///
/// Returns the amount of confidence lost (0.0 if within grace period).
pub fn apply_decay(lesson: &mut Lesson, now: DateTime<Utc>, thresholds: &ConfidenceThresholds) -> f32 {
    let days_since = (now - lesson.last_reinforced).num_days();
    if days_since <= thresholds.decay_grace_days {
        return 0.0;
    }

    let decay_days = days_since - thresholds.decay_grace_days;
    let decay = decay_days as f32 * thresholds.decay_per_day;
    let old = lesson.confidence;
    lesson.confidence = (lesson.confidence - decay).max(0.1);
    old - lesson.confidence
}

/// A scored lesson with its computed lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredLesson {
    /// The underlying lesson
    pub lesson: Lesson,
    /// Computed lifecycle state
    pub state: ConfidenceState,
    /// Days since last reinforcement
    pub days_since_reinforcement: i64,
}

/// The Confidence Scorer — manages lifecycle transitions and decay.
pub struct ConfidenceScorer {
    thresholds: ConfidenceThresholds,
}

impl ConfidenceScorer {
    /// Create a scorer with default thresholds.
    pub fn new() -> Self {
        Self {
            thresholds: ConfidenceThresholds::default(),
        }
    }

    /// Create a scorer with custom thresholds.
    pub fn with_thresholds(thresholds: ConfidenceThresholds) -> Self {
        Self { thresholds }
    }

    /// Get the thresholds.
    pub fn thresholds(&self) -> &ConfidenceThresholds {
        &self.thresholds
    }

    /// Score a single lesson, computing its lifecycle state.
    pub fn score(&self, lesson: &Lesson) -> ScoredLesson {
        let days_since = (Utc::now() - lesson.last_reinforced).num_days();
        ScoredLesson {
            lesson: lesson.clone(),
            state: confidence_state(lesson, &self.thresholds),
            days_since_reinforcement: days_since,
        }
    }

    /// Score all lessons from a learning engine.
    pub async fn score_all(&self, engine: &LearningEngine) -> Vec<ScoredLesson> {
        let lessons = engine.all_lessons().await;
        lessons.iter().map(|l| self.score(l)).collect()
    }

    /// Get instinct-level lessons suitable for system prompt injection.
    ///
    /// Returns lessons that have reached the Instinct state — these are
    /// battle-tested behaviors the agent should always apply.
    pub async fn get_instincts(&self, engine: &LearningEngine) -> Vec<ScoredLesson> {
        self.score_all(engine)
            .await
            .into_iter()
            .filter(|sl| sl.state == ConfidenceState::Instinct)
            .collect()
    }

    /// Get lessons at a minimum lifecycle state.
    pub async fn get_at_least(
        &self,
        engine: &LearningEngine,
        min_state: ConfidenceState,
    ) -> Vec<ScoredLesson> {
        self.score_all(engine)
            .await
            .into_iter()
            .filter(|sl| sl.state >= min_state)
            .collect()
    }

    /// Apply time-based decay to all lessons in the engine.
    ///
    /// Returns the number of lessons that were decayed.
    /// Each decayed lesson is persisted back via `engine.update_lesson` so
    /// that reduced confidence survives a process restart.
    pub async fn apply_decay_all(&self, engine: &LearningEngine) -> usize {
        let mut lessons = engine.all_lessons().await;
        let now = Utc::now();
        let mut decayed = 0;

        for lesson in &mut lessons {
            let lost = apply_decay(lesson, now, &self.thresholds);
            if lost > 0.0 {
                decayed += 1;
                debug!(
                    lesson_id = %lesson.id,
                    lost = lost,
                    new_confidence = lesson.confidence,
                    "Decayed lesson confidence"
                );
                engine.update_lesson(lesson).await;
            }
        }

        decayed
    }

    /// Format instinct-level lessons for system prompt injection.
    ///
    /// Returns a markdown block ready to be appended to the system prompt.
    pub async fn format_for_prompt(&self, engine: &LearningEngine) -> Option<String> {
        let instincts = self.get_instincts(engine).await;
        if instincts.is_empty() {
            return None;
        }

        let mut lines = vec!["## Learned Instincts".to_string()];
        lines.push(String::new());

        for sl in &instincts {
            let category = match sl.lesson.category {
                LessonCategory::UserPreference => "Preference",
                LessonCategory::SuccessfulStrategy => "Strategy",
                LessonCategory::FailedApproach => "Avoid",
                LessonCategory::DomainKnowledge => "Knowledge",
                LessonCategory::ToolUsage => "Tool",
                LessonCategory::Communication => "Communication",
                LessonCategory::Timing => "Timing",
                LessonCategory::ErrorRecovery => "Recovery",
            };

            let rec = sl
                .lesson
                .recommendation
                .as_deref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default();

            lines.push(format!(
                "- [{}] {} ({}x reinforced, {:.0}% confidence){}",
                category,
                sl.lesson.insight,
                sl.lesson.reinforcements,
                sl.lesson.confidence * 100.0,
                rec,
            ));
        }

        Some(lines.join("\n"))
    }

    /// Get a lifecycle summary for diagnostics.
    pub async fn lifecycle_summary(&self, engine: &LearningEngine) -> LifecycleSummary {
        let scored = self.score_all(engine).await;
        let mut summary = LifecycleSummary::default();

        for sl in &scored {
            match sl.state {
                ConfidenceState::Tentative => summary.tentative += 1,
                ConfidenceState::Established => summary.established += 1,
                ConfidenceState::Confident => summary.confident += 1,
                ConfidenceState::Instinct => summary.instinct += 1,
            }
        }
        summary.total = scored.len();
        summary.avg_confidence = if scored.is_empty() {
            0.0
        } else {
            scored.iter().map(|sl| sl.lesson.confidence).sum::<f32>() / scored.len() as f32
        };

        summary
    }
}

impl Default for ConfidenceScorer {
    fn default() -> Self {
        Self::new()
    }
}

/// Diagnostic summary of the lifecycle distribution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleSummary {
    pub total: usize,
    pub tentative: usize,
    pub established: usize,
    pub confident: usize,
    pub instinct: usize,
    pub avg_confidence: f32,
}

impl std::fmt::Display for LifecycleSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} lessons: {} tentative, {} established, {} confident, {} instinct (avg {:.0}%)",
            self.total,
            self.tentative,
            self.established,
            self.confident,
            self.instinct,
            self.avg_confidence * 100.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{Lesson, LessonCategory, LearningEngine, Outcome};

    fn make_lesson(confidence: f32, reinforcements: u32) -> Lesson {
        let mut lesson = Lesson::new("test insight", LessonCategory::SuccessfulStrategy);
        lesson.confidence = confidence;
        lesson.reinforcements = reinforcements;
        lesson
    }

    #[test]
    fn test_confidence_state_tentative() {
        let thresholds = ConfidenceThresholds::default();
        let lesson = make_lesson(0.3, 1);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Tentative);
    }

    #[test]
    fn test_confidence_state_established() {
        let thresholds = ConfidenceThresholds::default();
        let lesson = make_lesson(0.5, 4);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Established);
    }

    #[test]
    fn test_confidence_state_confident() {
        let thresholds = ConfidenceThresholds::default();
        let lesson = make_lesson(0.75, 7);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Confident);
    }

    #[test]
    fn test_confidence_state_instinct() {
        let thresholds = ConfidenceThresholds::default();
        let lesson = make_lesson(0.90, 12);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Instinct);
    }

    #[test]
    fn test_confidence_needs_both_thresholds() {
        let thresholds = ConfidenceThresholds::default();
        // High confidence but low reinforcements → stays lower
        let lesson = make_lesson(0.90, 2);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Tentative);
    }

    #[test]
    fn test_decay_within_grace_period() {
        let thresholds = ConfidenceThresholds::default();
        let mut lesson = make_lesson(0.8, 5);
        lesson.last_reinforced = Utc::now() - chrono::Duration::days(3);
        let lost = apply_decay(&mut lesson, Utc::now(), &thresholds);
        assert_eq!(lost, 0.0);
        assert!((lesson.confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_decay_after_grace_period() {
        let thresholds = ConfidenceThresholds::default();
        let mut lesson = make_lesson(0.8, 5);
        // 17 days since reinforcement, 7 grace = 10 days of decay at 0.02/day = 0.2 lost
        lesson.last_reinforced = Utc::now() - chrono::Duration::days(17);
        let lost = apply_decay(&mut lesson, Utc::now(), &thresholds);
        assert!((lost - 0.2).abs() < 0.01);
        assert!((lesson.confidence - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_decay_floors_at_minimum() {
        let thresholds = ConfidenceThresholds::default();
        let mut lesson = make_lesson(0.3, 2);
        // Huge decay period
        lesson.last_reinforced = Utc::now() - chrono::Duration::days(100);
        apply_decay(&mut lesson, Utc::now(), &thresholds);
        assert!((lesson.confidence - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scorer_score_single() {
        let scorer = ConfidenceScorer::new();
        let lesson = make_lesson(0.5, 4);
        let scored = scorer.score(&lesson);
        assert_eq!(scored.state, ConfidenceState::Established);
    }

    #[tokio::test]
    async fn test_scorer_score_all_empty() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();
        let scored = scorer.score_all(&engine).await;
        assert!(scored.is_empty());
    }

    #[tokio::test]
    async fn test_scorer_with_lessons() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();

        // Add a lesson
        let outcome = Outcome {
            intent_id: "test-1".to_string(),
            success: true,
            feedback: "Strategy worked well for optimization".to_string(),
            timestamp: Utc::now(),
        };
        engine.extract_lesson(&outcome).await.unwrap();

        let scored = scorer.score_all(&engine).await;
        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].state, ConfidenceState::Tentative);
    }

    #[tokio::test]
    async fn test_get_instincts_empty_when_no_instinct_level() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();

        let outcome = Outcome {
            intent_id: "t1".to_string(),
            success: true,
            feedback: "Some approach succeeded".to_string(),
            timestamp: Utc::now(),
        };
        engine.extract_lesson(&outcome).await.unwrap();

        let instincts = scorer.get_instincts(&engine).await;
        assert!(instincts.is_empty());
    }

    #[tokio::test]
    async fn test_format_for_prompt_none_when_empty() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();
        let prompt = scorer.format_for_prompt(&engine).await;
        assert!(prompt.is_none());
    }

    #[tokio::test]
    async fn test_lifecycle_summary_empty() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();
        let summary = scorer.lifecycle_summary(&engine).await;
        assert_eq!(summary.total, 0);
        assert_eq!(summary.tentative, 0);
        assert_eq!(summary.instinct, 0);
    }

    #[tokio::test]
    async fn test_lifecycle_summary_with_lessons() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();

        // Use highly distinct feedback to avoid similarity merging (3-word overlap threshold)
        let feedbacks = [
            "Database migration completed flawlessly",
            "Caching layer dramatically improved latency",
            "WebSocket reconnection logic handled gracefully",
        ];
        for (i, feedback) in feedbacks.iter().enumerate() {
            let outcome = Outcome {
                intent_id: format!("sum-{}", i),
                success: true,
                feedback: feedback.to_string(),
                timestamp: Utc::now(),
            };
            engine.extract_lesson(&outcome).await.unwrap();
        }

        let summary = scorer.lifecycle_summary(&engine).await;
        assert_eq!(summary.total, 3);
        assert_eq!(summary.tentative, 3); // All start tentative
        assert!(summary.avg_confidence > 0.0);
    }

    #[test]
    fn test_confidence_state_ordering() {
        assert!(ConfidenceState::Tentative < ConfidenceState::Established);
        assert!(ConfidenceState::Established < ConfidenceState::Confident);
        assert!(ConfidenceState::Confident < ConfidenceState::Instinct);
    }

    #[test]
    fn test_confidence_state_display() {
        assert_eq!(ConfidenceState::Tentative.to_string(), "tentative");
        assert_eq!(ConfidenceState::Established.to_string(), "established");
        assert_eq!(ConfidenceState::Confident.to_string(), "confident");
        assert_eq!(ConfidenceState::Instinct.to_string(), "instinct");
    }

    #[test]
    fn test_lifecycle_summary_display() {
        let summary = LifecycleSummary {
            total: 10,
            tentative: 4,
            established: 3,
            confident: 2,
            instinct: 1,
            avg_confidence: 0.55,
        };
        let s = summary.to_string();
        assert!(s.contains("10 lessons"));
        assert!(s.contains("4 tentative"));
        assert!(s.contains("1 instinct"));
        assert!(s.contains("55%"));
    }

    #[test]
    fn test_custom_thresholds() {
        let thresholds = ConfidenceThresholds {
            established_confidence: 0.3,
            established_reinforcements: 2,
            confident_confidence: 0.5,
            confident_reinforcements: 4,
            instinct_confidence: 0.7,
            instinct_reinforcements: 6,
            decay_per_day: 0.05,
            decay_grace_days: 3,
        };
        let scorer = ConfidenceScorer::with_thresholds(thresholds);

        // With lower thresholds, a lesson at 0.5/4 is Confident
        let lesson = make_lesson(0.5, 4);
        let scored = scorer.score(&lesson);
        assert_eq!(scored.state, ConfidenceState::Confident);
    }

    #[test]
    fn test_scored_lesson_serialization() {
        let lesson = make_lesson(0.7, 5);
        let scored = ScoredLesson {
            lesson,
            state: ConfidenceState::Confident,
            days_since_reinforcement: 2,
        };
        let json = serde_json::to_string(&scored).unwrap();
        let deser: ScoredLesson = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.state, ConfidenceState::Confident);
        assert_eq!(deser.days_since_reinforcement, 2);
    }

    #[test]
    fn test_thresholds_serialization() {
        let thresholds = ConfidenceThresholds::default();
        let json = serde_json::to_string(&thresholds).unwrap();
        let deser: ConfidenceThresholds = serde_json::from_str(&json).unwrap();
        assert!((deser.decay_per_day - 0.02).abs() < f32::EPSILON);
        assert_eq!(deser.decay_grace_days, 7);
    }

    #[tokio::test]
    async fn test_get_at_least_established() {
        let scorer = ConfidenceScorer::new();
        let engine = LearningEngine::new().await.unwrap();

        // Create one lesson — it starts Tentative (conf 0.5, reinf 1)
        let outcome = Outcome {
            intent_id: "filter-1".to_string(),
            success: true,
            feedback: "Approach worked".to_string(),
            timestamp: Utc::now(),
        };
        engine.extract_lesson(&outcome).await.unwrap();

        let established = scorer.get_at_least(&engine, ConfidenceState::Established).await;
        assert!(established.is_empty()); // 0.5/1 is Tentative, not Established

        let tentative = scorer.get_at_least(&engine, ConfidenceState::Tentative).await;
        assert_eq!(tentative.len(), 1);
    }

    #[tokio::test]
    async fn test_apply_decay_all_persisted() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("decay_persist.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(
            zeus_mnemosyne::Mnemosyne::new(config)
                .await
                .expect("mnemosyne"),
        );
        let engine = LearningEngine::with_mnemosyne(mnemosyne)
            .await
            .expect("engine");

        // Create a lesson
        let outcome = Outcome {
            intent_id: "decay-persist-1".to_string(),
            success: true,
            feedback: "Optimisation approach worked efficiently here".to_string(),
            timestamp: Utc::now(),
        };
        let lesson = engine.extract_lesson(&outcome).await.expect("extract");

        // Backdate last_reinforced past the 7-day grace period.
        // 17 days since reinforcement: 10 decay days × 0.02/day = 0.2 lost.
        // Starting confidence 0.5 → expected 0.3 after decay.
        let mut stale = lesson.clone();
        stale.last_reinforced = Utc::now() - chrono::Duration::days(17);
        stale.confidence = 0.5;
        engine.update_lesson(&stale).await;

        // Apply decay
        let scorer = ConfidenceScorer::new();
        let count = scorer.apply_decay_all(&engine).await;
        assert_eq!(count, 1, "exactly one lesson should be decayed");

        // In-memory reflects decayed confidence
        let in_mem = engine.all_lessons().await;
        assert_eq!(in_mem.len(), 1);
        assert!(
            (in_mem[0].confidence - 0.3).abs() < 0.01,
            "in-memory confidence should be ~0.3 after decay; got {}",
            in_mem[0].confidence
        );

        // Reload from Mnemosyne — decayed confidence must survive restart
        let config2 = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("decay_persist.db"),
            ..Default::default()
        };
        let mnemosyne2 = Arc::new(
            zeus_mnemosyne::Mnemosyne::new(config2)
                .await
                .expect("mnemosyne2"),
        );
        let engine2 = LearningEngine::with_mnemosyne(mnemosyne2)
            .await
            .expect("engine2");

        let reloaded = engine2.all_lessons().await;
        assert_eq!(reloaded.len(), 1, "deduplication: exactly one entry per id");
        assert!(
            (reloaded[0].confidence - 0.3).abs() < 0.01,
            "decayed confidence must survive restart; got {}",
            reloaded[0].confidence
        );
    }

    #[test]
    fn test_boundary_confidence_state() {
        let thresholds = ConfidenceThresholds::default();

        // Exactly at Established boundary
        let lesson = make_lesson(0.4, 3);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Established);

        // Just below Established confidence
        let lesson = make_lesson(0.39, 3);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Tentative);

        // Exactly at Confident boundary
        let lesson = make_lesson(0.7, 6);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Confident);

        // Exactly at Instinct boundary
        let lesson = make_lesson(0.85, 10);
        assert_eq!(confidence_state(&lesson, &thresholds), ConfidenceState::Instinct);
    }
}
