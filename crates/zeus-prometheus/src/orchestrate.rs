//! Orchestration State Machine
//!
//! Manages the full lifecycle of an autonomous project orchestration:
//! GoalReceived → Analyzing → Onboarding (Q&A) → TeamRecommendation
//!   → AutonomyChoice → Executing → Packaging → Delivered
//!
//! Each orchestration session holds its own state, Q&A history, team
//! recommendation, and produced artifacts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// State Machine
// ============================================================================

/// The current phase of an orchestration session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum OrchestrationPhase {
    /// Goal received, waiting for LLM analysis.
    GoalReceived,
    /// LLM is analyzing the goal to determine scope + questions.
    Analyzing,
    /// Conversational Q&A to refine requirements (max 5 questions).
    Onboarding {
        questions: Vec<OnboardingQuestion>,
        current_index: usize,
    },
    /// LLM has recommended a team composition.
    TeamRecommendation { recommendation: TeamRecommendation },
    /// User must choose autonomy level before execution.
    AutonomyChoice { recommendation: TeamRecommendation },
    /// Plan is being executed with streaming progress.
    Executing {
        plan_id: String,
        steps_total: usize,
        steps_completed: usize,
    },
    /// Collecting artifacts and building deliverable package.
    Packaging,
    /// Orchestration complete, deliverable ready.
    Delivered {
        artifact_path: String,
        summary: String,
    },
    /// Orchestration failed at some phase.
    Failed {
        reason: String,
        phase_when_failed: String,
    },
}

impl OrchestrationPhase {
    pub fn label(&self) -> &str {
        match self {
            Self::GoalReceived => "goal_received",
            Self::Analyzing => "analyzing",
            Self::Onboarding { .. } => "onboarding",
            Self::TeamRecommendation { .. } => "team_recommendation",
            Self::AutonomyChoice { .. } => "autonomy_choice",
            Self::Executing { .. } => "executing",
            Self::Packaging => "packaging",
            Self::Delivered { .. } => "delivered",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Delivered { .. } | Self::Failed { .. })
    }
}

// ============================================================================
// Supporting Types
// ============================================================================

/// A question asked during the onboarding Q&A phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingQuestion {
    pub question: String,
    pub answer: Option<String>,
    pub purpose: String,
}

/// Recommended team composition for executing a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecommendation {
    pub team_name: String,
    pub coordinators: Vec<AgentSuggestion>,
    pub workers: Vec<AgentSuggestion>,
    pub rationale: String,
    pub estimated_complexity: String,
    pub estimated_steps: usize,
}

/// A suggested agent for the team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSuggestion {
    pub role: String,
    pub capabilities: Vec<String>,
    pub model_tier: String,
}

/// Autonomy level chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationAutonomy {
    /// Agent executes everything without confirmation.
    Full,
    /// Agent pauses for confirmation on destructive/critical steps.
    Supervised,
}

/// An artifact produced during orchestration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
}

/// LLM analysis of a goal (produced during Analyzing phase).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalAnalysis {
    pub summary: String,
    pub scope: String,
    pub complexity: String,
    pub suggested_approach: String,
    pub needs_clarification: bool,
    pub clarification_questions: Vec<OnboardingQuestion>,
}

// ============================================================================
// Orchestration Session
// ============================================================================

/// A single orchestration session tracking state from goal to delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationSession {
    pub id: String,
    pub goal: String,
    pub phase: OrchestrationPhase,
    pub analysis: Option<GoalAnalysis>,
    pub autonomy: Option<OrchestrationAutonomy>,
    pub artifacts: Vec<Artifact>,
    pub qa_history: Vec<OnboardingQuestion>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OrchestrationSession {
    /// Create a new session from a goal description.
    pub fn new(goal: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: format!("orch-{}", uuid::Uuid::new_v4()),
            goal: goal.into(),
            phase: OrchestrationPhase::GoalReceived,
            analysis: None,
            autonomy: None,
            artifacts: Vec::new(),
            qa_history: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Transition to Analyzing phase.
    pub fn start_analysis(&mut self) {
        self.phase = OrchestrationPhase::Analyzing;
        self.updated_at = Utc::now();
    }

    /// Complete analysis and transition to Onboarding or TeamRecommendation.
    pub fn complete_analysis(&mut self, analysis: GoalAnalysis) {
        if analysis.needs_clarification && !analysis.clarification_questions.is_empty() {
            let questions = analysis.clarification_questions.clone();
            self.phase = OrchestrationPhase::Onboarding {
                questions: questions.clone(),
                current_index: 0,
            };
            self.qa_history = questions;
        } else {
            // Skip onboarding, go straight to team recommendation
            self.phase = OrchestrationPhase::Analyzing; // Will be set by recommend_team
        }
        self.analysis = Some(analysis);
        self.updated_at = Utc::now();
    }

    /// Record an answer to the current onboarding question.
    /// Returns true if there are more questions, false if onboarding is complete.
    pub fn answer_question(&mut self, answer: String) -> bool {
        if let OrchestrationPhase::Onboarding {
            ref mut questions,
            ref mut current_index,
        } = self.phase
            && *current_index < questions.len()
        {
            questions[*current_index].answer = Some(answer.clone());
            // Also update qa_history
            if *current_index < self.qa_history.len() {
                self.qa_history[*current_index].answer = Some(answer);
            }
            *current_index += 1;
            self.updated_at = Utc::now();
            return *current_index < questions.len();
        }
        false
    }

    /// Get the current onboarding question, if in Onboarding phase.
    pub fn current_question(&self) -> Option<&OnboardingQuestion> {
        if let OrchestrationPhase::Onboarding {
            ref questions,
            current_index,
        } = self.phase
        {
            questions.get(current_index)
        } else {
            None
        }
    }

    /// Set team recommendation and transition to AutonomyChoice.
    pub fn recommend_team(&mut self, recommendation: TeamRecommendation) {
        self.phase = OrchestrationPhase::AutonomyChoice {
            recommendation: recommendation.clone(),
        };
        self.updated_at = Utc::now();
    }

    /// User confirms team + autonomy, transition to Executing.
    pub fn confirm_and_execute(
        &mut self,
        autonomy: OrchestrationAutonomy,
        plan_id: String,
        steps_total: usize,
    ) {
        self.autonomy = Some(autonomy);
        self.phase = OrchestrationPhase::Executing {
            plan_id,
            steps_total,
            steps_completed: 0,
        };
        self.updated_at = Utc::now();
    }

    /// Update execution progress.
    pub fn update_progress(&mut self, steps_completed: usize) {
        if let OrchestrationPhase::Executing {
            steps_completed: ref mut current_completed,
            ..
        } = self.phase
        {
            *current_completed = steps_completed;
            self.updated_at = Utc::now();
        }
    }

    /// Transition to Packaging phase.
    pub fn start_packaging(&mut self) {
        self.phase = OrchestrationPhase::Packaging;
        self.updated_at = Utc::now();
    }

    /// Add an artifact produced during orchestration.
    pub fn add_artifact(&mut self, artifact: Artifact) {
        self.artifacts.push(artifact);
        self.updated_at = Utc::now();
    }

    /// Mark as delivered with final artifact path and summary.
    pub fn deliver(&mut self, artifact_path: String, summary: String) {
        self.phase = OrchestrationPhase::Delivered {
            artifact_path,
            summary,
        };
        self.updated_at = Utc::now();
    }

    /// Mark as failed.
    pub fn fail(&mut self, reason: String) {
        let phase_label = self.phase.label().to_string();
        self.phase = OrchestrationPhase::Failed {
            reason,
            phase_when_failed: phase_label,
        };
        self.updated_at = Utc::now();
    }
}

// ============================================================================
// Orchestration Manager
// ============================================================================

/// Manages all active orchestration sessions.
#[derive(Clone)]
pub struct OrchestrationManager {
    sessions: Arc<RwLock<HashMap<String, OrchestrationSession>>>,
}

impl OrchestrationManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new orchestration session from a goal.
    pub async fn create(&self, goal: impl Into<String>) -> OrchestrationSession {
        let session = OrchestrationSession::new(goal);
        let id = session.id.clone();
        self.sessions.write().await.insert(id, session.clone());
        session
    }

    /// Get a session by ID.
    pub async fn get(&self, id: &str) -> Option<OrchestrationSession> {
        self.sessions.read().await.get(id).cloned()
    }

    /// Update a session in-place via a mutation closure.
    pub async fn update<F>(&self, id: &str, f: F) -> Option<OrchestrationSession>
    where
        F: FnOnce(&mut OrchestrationSession),
    {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(id) {
            f(session);
            Some(session.clone())
        } else {
            None
        }
    }

    /// List all sessions (optionally filtered by terminal/active).
    pub async fn list(&self, include_terminal: bool) -> Vec<OrchestrationSession> {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| include_terminal || !s.phase.is_terminal())
            .cloned()
            .collect()
    }

    /// Remove a terminal session.
    pub async fn remove(&self, id: &str) -> Option<OrchestrationSession> {
        self.sessions.write().await.remove(id)
    }

    /// Count of active (non-terminal) sessions.
    pub async fn active_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| !s.phase.is_terminal())
            .count()
    }
}

impl Default for OrchestrationManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle() {
        let mut session = OrchestrationSession::new("Build a todo app in React");
        assert_eq!(session.phase.label(), "goal_received");

        session.start_analysis();
        assert_eq!(session.phase.label(), "analyzing");

        let analysis = GoalAnalysis {
            summary: "Create a React todo application".into(),
            scope: "frontend".into(),
            complexity: "medium".into(),
            suggested_approach: "Use Vite + React + TypeScript".into(),
            needs_clarification: true,
            clarification_questions: vec![
                OnboardingQuestion {
                    question: "Should it have persistent storage?".into(),
                    answer: None,
                    purpose: "Determine backend needs".into(),
                },
                OnboardingQuestion {
                    question: "Any preferred styling framework?".into(),
                    answer: None,
                    purpose: "Determine UI approach".into(),
                },
            ],
        };
        session.complete_analysis(analysis);
        assert_eq!(session.phase.label(), "onboarding");

        // Answer questions
        let more = session.answer_question("Yes, use localStorage".into());
        assert!(more); // Still one more question
        let more = session.answer_question("Tailwind CSS".into());
        assert!(!more); // Done with questions

        // Team recommendation
        let rec = TeamRecommendation {
            team_name: "react-builders".into(),
            coordinators: vec![AgentSuggestion {
                role: "project-lead".into(),
                capabilities: vec!["planning".into(), "code-review".into()],
                model_tier: "opus".into(),
            }],
            workers: vec![AgentSuggestion {
                role: "frontend-dev".into(),
                capabilities: vec!["react".into(), "typescript".into()],
                model_tier: "sonnet".into(),
            }],
            rationale: "Medium complexity frontend project".into(),
            estimated_complexity: "medium".into(),
            estimated_steps: 5,
        };
        session.recommend_team(rec);
        assert_eq!(session.phase.label(), "autonomy_choice");

        // Confirm
        session.confirm_and_execute(OrchestrationAutonomy::Full, "plan-123".into(), 5);
        assert_eq!(session.phase.label(), "executing");

        // Progress
        session.update_progress(3);

        // Package
        session.start_packaging();
        assert_eq!(session.phase.label(), "packaging");

        // Deliver
        session.deliver(
            "/tmp/deliverables/orch-123.zip".into(),
            "Todo app created successfully".into(),
        );
        assert_eq!(session.phase.label(), "delivered");
        assert!(session.phase.is_terminal());
    }

    #[test]
    fn test_session_failure() {
        let mut session = OrchestrationSession::new("impossible task");
        session.start_analysis();
        session.fail("LLM unavailable".into());
        assert_eq!(session.phase.label(), "failed");
        assert!(session.phase.is_terminal());
        if let OrchestrationPhase::Failed {
            phase_when_failed, ..
        } = &session.phase
        {
            assert_eq!(phase_when_failed, "analyzing");
        }
    }

    #[test]
    fn test_skip_onboarding() {
        let mut session = OrchestrationSession::new("Simple greeting app");
        session.start_analysis();
        let analysis = GoalAnalysis {
            summary: "Simple app".into(),
            scope: "trivial".into(),
            complexity: "low".into(),
            suggested_approach: "Single file".into(),
            needs_clarification: false,
            clarification_questions: vec![],
        };
        session.complete_analysis(analysis);
        // Should skip onboarding since no clarification needed
        assert!(session.analysis.is_some());
    }

    #[tokio::test]
    async fn test_manager() {
        let mgr = OrchestrationManager::new();
        let session = mgr.create("Build a CLI tool").await;
        assert_eq!(mgr.active_count().await, 1);

        let fetched = mgr.get(&session.id).await.unwrap();
        assert_eq!(fetched.goal, "Build a CLI tool");

        mgr.update(&session.id, |s| s.fail("test".into())).await;
        assert_eq!(mgr.active_count().await, 0); // terminal sessions not counted
        assert_eq!(mgr.list(true).await.len(), 1); // but still in list with include_terminal
    }
}
