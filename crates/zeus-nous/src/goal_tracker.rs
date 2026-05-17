//! Goal Tracker — hierarchical goal decomposition and progress tracking.
//!
//! Tracks user goals as trees of subgoals with progress, priority, and deadlines:
//!
//! - **GoalTracker** — manages a set of active goals
//! - **Goal** — a single goal with optional subgoals and progress
//! - **GoalStatus** — lifecycle states (Active, Paused, Completed, Abandoned)
//! - **GoalPriority** — urgency levels (Low, Medium, High, Critical)
//! - **Milestone** — named checkpoint within a goal
//! - **GoalSummary** — aggregated view of goal tree progress

use std::collections::HashMap;

use chrono::{DateTime, Utc};

// ============================================================================
// GoalPriority
// ============================================================================

/// Priority level for a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl GoalPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalPriority::Low => "low",
            GoalPriority::Medium => "medium",
            GoalPriority::High => "high",
            GoalPriority::Critical => "critical",
        }
    }

    /// Numeric weight for sorting (higher = more important).
    pub fn weight(&self) -> u8 {
        match self {
            GoalPriority::Low => 1,
            GoalPriority::Medium => 2,
            GoalPriority::High => 3,
            GoalPriority::Critical => 4,
        }
    }
}

// ============================================================================
// GoalStatus
// ============================================================================

/// Lifecycle status of a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    /// Goal is actively being pursued.
    Active,
    /// Goal is temporarily paused.
    Paused,
    /// Goal has been completed.
    Completed,
    /// Goal has been abandoned.
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Abandoned => "abandoned",
        }
    }

    /// Whether this status represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, GoalStatus::Completed | GoalStatus::Abandoned)
    }
}

// ============================================================================
// Milestone
// ============================================================================

/// A named checkpoint within a goal.
#[derive(Debug, Clone)]
pub struct Milestone {
    /// Milestone name.
    pub name: String,
    /// Whether the milestone has been reached.
    pub reached: bool,
    /// When the milestone was reached.
    pub reached_at: Option<DateTime<Utc>>,
    /// Optional notes.
    pub notes: Option<String>,
}

impl Milestone {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            reached: false,
            reached_at: None,
            notes: None,
        }
    }

    /// Mark this milestone as reached.
    pub fn mark_reached(&mut self) {
        self.reached = true;
        self.reached_at = Some(Utc::now());
    }

    /// Mark reached with a note.
    pub fn mark_reached_with_note(&mut self, note: &str) {
        self.mark_reached();
        self.notes = Some(note.to_string());
    }
}

// ============================================================================
// Goal
// ============================================================================

/// A single goal with optional subgoals, milestones, and progress tracking.
#[derive(Debug, Clone)]
pub struct Goal {
    /// Unique identifier.
    pub id: String,
    /// Goal title.
    pub title: String,
    /// Detailed description.
    pub description: Option<String>,
    /// Current status.
    pub status: GoalStatus,
    /// Priority level.
    pub priority: GoalPriority,
    /// Manual progress override (0.0–1.0). If None, computed from subgoals/milestones.
    pub progress: Option<f64>,
    /// Subgoal IDs (tracked in the GoalTracker).
    pub subgoal_ids: Vec<String>,
    /// Milestones within this goal.
    pub milestones: Vec<Milestone>,
    /// When the goal was created.
    pub created_at: DateTime<Utc>,
    /// When the goal was last updated.
    pub updated_at: DateTime<Utc>,
    /// Optional deadline.
    pub deadline: Option<DateTime<Utc>>,
    /// Parent goal ID (if this is a subgoal).
    pub parent_id: Option<String>,
    /// Free-form tags.
    pub tags: Vec<String>,
}

impl Goal {
    /// Create a new active goal.
    pub fn new(id: &str, title: &str) -> Self {
        let now = Utc::now();
        Self {
            id: id.to_string(),
            title: title.to_string(),
            description: None,
            status: GoalStatus::Active,
            priority: GoalPriority::Medium,
            progress: None,
            subgoal_ids: Vec::new(),
            milestones: Vec::new(),
            created_at: now,
            updated_at: now,
            deadline: None,
            parent_id: None,
            tags: Vec::new(),
        }
    }

    /// Set description.
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: GoalPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set deadline.
    pub fn with_deadline(mut self, deadline: DateTime<Utc>) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set tags.
    pub fn with_tags(mut self, tags: Vec<&str>) -> Self {
        self.tags = tags.into_iter().map(|s| s.to_string()).collect();
        self
    }

    /// Set parent goal.
    pub fn with_parent(mut self, parent_id: &str) -> Self {
        self.parent_id = Some(parent_id.to_string());
        self
    }

    /// Add a milestone.
    pub fn add_milestone(&mut self, name: &str) {
        self.milestones.push(Milestone::new(name));
        self.updated_at = Utc::now();
    }

    /// Mark a milestone as reached by name. Returns true if found.
    pub fn reach_milestone(&mut self, name: &str) -> bool {
        if let Some(ms) = self.milestones.iter_mut().find(|m| m.name == name) {
            ms.mark_reached();
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Set manual progress (0.0–1.0, clamped).
    pub fn set_progress(&mut self, pct: f64) {
        self.progress = Some(pct.clamp(0.0, 1.0));
        self.updated_at = Utc::now();
    }

    /// Compute effective progress.
    ///
    /// - If manual progress is set, use that.
    /// - If milestones exist, compute from milestone completion ratio.
    /// - Otherwise 0.0 for Active, 1.0 for Completed.
    pub fn effective_progress(&self) -> f64 {
        if let Some(p) = self.progress {
            return p;
        }
        if !self.milestones.is_empty() {
            let reached = self.milestones.iter().filter(|m| m.reached).count();
            return reached as f64 / self.milestones.len() as f64;
        }
        match self.status {
            GoalStatus::Completed => 1.0,
            _ => 0.0,
        }
    }

    /// Mark goal as completed.
    pub fn complete(&mut self) {
        self.status = GoalStatus::Completed;
        self.progress = Some(1.0);
        self.updated_at = Utc::now();
    }

    /// Mark goal as abandoned.
    pub fn abandon(&mut self) {
        self.status = GoalStatus::Abandoned;
        self.updated_at = Utc::now();
    }

    /// Pause the goal.
    pub fn pause(&mut self) {
        self.status = GoalStatus::Paused;
        self.updated_at = Utc::now();
    }

    /// Resume a paused goal.
    pub fn resume(&mut self) {
        if self.status == GoalStatus::Paused {
            self.status = GoalStatus::Active;
            self.updated_at = Utc::now();
        }
    }

    /// Whether the goal is past its deadline.
    pub fn is_overdue(&self) -> bool {
        if let Some(deadline) = self.deadline {
            !self.status.is_terminal() && Utc::now() > deadline
        } else {
            false
        }
    }

    /// Milestone completion ratio as a string (e.g., "3/5").
    pub fn milestone_summary(&self) -> String {
        let reached = self.milestones.iter().filter(|m| m.reached).count();
        format!("{}/{}", reached, self.milestones.len())
    }
}

// ============================================================================
// GoalSummary
// ============================================================================

/// Aggregated summary of the goal tracker.
#[derive(Debug, Clone, Default)]
pub struct GoalSummary {
    pub total_goals: usize,
    pub active: usize,
    pub paused: usize,
    pub completed: usize,
    pub abandoned: usize,
    pub overdue: usize,
    pub avg_progress: f64,
    pub by_priority: HashMap<GoalPriority, usize>,
}

// ============================================================================
// GoalTracker
// ============================================================================

/// Manages a set of goals with hierarchical decomposition.
pub struct GoalTracker {
    goals: HashMap<String, Goal>,
    next_id: u64,
}

impl GoalTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self {
            goals: HashMap::new(),
            next_id: 1,
        }
    }

    /// Generate a unique goal ID.
    fn gen_id(&mut self) -> String {
        let id = format!("goal-{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Add a new top-level goal. Returns the generated ID.
    pub fn add_goal(&mut self, title: &str) -> String {
        let id = self.gen_id();
        let goal = Goal::new(&id, title);
        self.goals.insert(id.clone(), goal);
        id
    }

    /// Add a goal with full configuration. Returns the generated ID.
    pub fn add_goal_with(&mut self, mut goal: Goal) -> String {
        let id = self.gen_id();
        goal.id = id.clone();
        self.goals.insert(id.clone(), goal);
        id
    }

    /// Add a subgoal under a parent. Returns the subgoal ID, or None if parent not found.
    pub fn add_subgoal(&mut self, parent_id: &str, title: &str) -> Option<String> {
        if !self.goals.contains_key(parent_id) {
            return None;
        }

        let id = self.gen_id();
        let subgoal = Goal::new(&id, title).with_parent(parent_id);
        self.goals.insert(id.clone(), subgoal);

        if let Some(parent) = self.goals.get_mut(parent_id) {
            parent.subgoal_ids.push(id.clone());
            parent.updated_at = Utc::now();
        }

        Some(id)
    }

    /// Get a goal by ID.
    pub fn get(&self, id: &str) -> Option<&Goal> {
        self.goals.get(id)
    }

    /// Get a mutable goal by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Goal> {
        self.goals.get_mut(id)
    }

    /// Remove a goal (and unlink from parent). Returns the removed goal.
    pub fn remove(&mut self, id: &str) -> Option<Goal> {
        let goal = self.goals.remove(id)?;

        // Unlink from parent
        if let Some(ref parent_id) = goal.parent_id
            && let Some(parent) = self.goals.get_mut(parent_id)
        {
            parent.subgoal_ids.retain(|sid| sid != id);
        }

        // Remove subgoals recursively
        for sub_id in &goal.subgoal_ids {
            self.goals.remove(sub_id);
        }

        Some(goal)
    }

    /// List all top-level goals (no parent).
    pub fn top_level(&self) -> Vec<&Goal> {
        let mut goals: Vec<&Goal> = self
            .goals
            .values()
            .filter(|g| g.parent_id.is_none())
            .collect();
        goals.sort_by(|a, b| b.priority.weight().cmp(&a.priority.weight()));
        goals
    }

    /// List subgoals of a goal.
    pub fn subgoals(&self, parent_id: &str) -> Vec<&Goal> {
        self.goals
            .get(parent_id)
            .map(|parent| {
                parent
                    .subgoal_ids
                    .iter()
                    .filter_map(|sid| self.goals.get(sid))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List active goals sorted by priority (highest first).
    pub fn active_goals(&self) -> Vec<&Goal> {
        let mut goals: Vec<&Goal> = self
            .goals
            .values()
            .filter(|g| g.status == GoalStatus::Active)
            .collect();
        goals.sort_by(|a, b| b.priority.weight().cmp(&a.priority.weight()));
        goals
    }

    /// List overdue goals.
    pub fn overdue_goals(&self) -> Vec<&Goal> {
        self.goals.values().filter(|g| g.is_overdue()).collect()
    }

    /// Search goals by title or tag substring.
    pub fn search(&self, query: &str) -> Vec<&Goal> {
        let q = query.to_lowercase();
        self.goals
            .values()
            .filter(|g| {
                g.title.to_lowercase().contains(&q)
                    || g.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    || g.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
            })
            .collect()
    }

    /// Total number of goals (including subgoals).
    pub fn len(&self) -> usize {
        self.goals.len()
    }

    /// Check if tracker is empty.
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }

    /// Compute a summary of all goals.
    pub fn summary(&self) -> GoalSummary {
        let mut s = GoalSummary {
            total_goals: self.goals.len(),
            ..Default::default()
        };

        let mut total_progress = 0.0;

        for goal in self.goals.values() {
            match goal.status {
                GoalStatus::Active => s.active += 1,
                GoalStatus::Paused => s.paused += 1,
                GoalStatus::Completed => s.completed += 1,
                GoalStatus::Abandoned => s.abandoned += 1,
            }
            if goal.is_overdue() {
                s.overdue += 1;
            }
            total_progress += goal.effective_progress();
            *s.by_priority.entry(goal.priority).or_insert(0) += 1;
        }

        s.avg_progress = if s.total_goals > 0 {
            total_progress / s.total_goals as f64
        } else {
            0.0
        };

        s
    }

    /// Compute recursive progress for a goal including its subgoals.
    /// Each subgoal contributes equally to the parent's progress.
    pub fn recursive_progress(&self, id: &str) -> f64 {
        let goal = match self.goals.get(id) {
            Some(g) => g,
            None => return 0.0,
        };

        // If manual progress is set, use that
        if goal.progress.is_some() {
            return goal.effective_progress();
        }

        // If subgoals exist, average their recursive progress
        if !goal.subgoal_ids.is_empty() {
            let sum: f64 = goal
                .subgoal_ids
                .iter()
                .map(|sid| self.recursive_progress(sid))
                .sum();
            return sum / goal.subgoal_ids.len() as f64;
        }

        // Fall back to effective_progress (milestones or status)
        goal.effective_progress()
    }

    /// Suggest the next goal to work on.
    ///
    /// Returns the highest-priority active goal with the lowest progress.
    pub fn suggest_next(&self) -> Option<&Goal> {
        let mut candidates: Vec<&Goal> = self
            .goals
            .values()
            .filter(|g| g.status == GoalStatus::Active && g.parent_id.is_none())
            .collect();

        // Sort by: priority desc, then progress asc
        candidates.sort_by(|a, b| {
            b.priority.weight().cmp(&a.priority.weight()).then_with(|| {
                a.effective_progress()
                    .partial_cmp(&b.effective_progress())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        candidates.first().copied()
    }
}

impl Default for GoalTracker {
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

    // -- GoalPriority -------------------------------------------------------

    #[test]
    fn test_priority_as_str() {
        assert_eq!(GoalPriority::Low.as_str(), "low");
        assert_eq!(GoalPriority::Medium.as_str(), "medium");
        assert_eq!(GoalPriority::High.as_str(), "high");
        assert_eq!(GoalPriority::Critical.as_str(), "critical");
    }

    #[test]
    fn test_priority_weight_ordering() {
        assert!(GoalPriority::Critical.weight() > GoalPriority::High.weight());
        assert!(GoalPriority::High.weight() > GoalPriority::Medium.weight());
        assert!(GoalPriority::Medium.weight() > GoalPriority::Low.weight());
    }

    // -- GoalStatus ---------------------------------------------------------

    #[test]
    fn test_status_as_str() {
        assert_eq!(GoalStatus::Active.as_str(), "active");
        assert_eq!(GoalStatus::Paused.as_str(), "paused");
        assert_eq!(GoalStatus::Completed.as_str(), "completed");
        assert_eq!(GoalStatus::Abandoned.as_str(), "abandoned");
    }

    #[test]
    fn test_status_terminal() {
        assert!(!GoalStatus::Active.is_terminal());
        assert!(!GoalStatus::Paused.is_terminal());
        assert!(GoalStatus::Completed.is_terminal());
        assert!(GoalStatus::Abandoned.is_terminal());
    }

    // -- Milestone ----------------------------------------------------------

    #[test]
    fn test_milestone_new() {
        let ms = Milestone::new("Alpha release");
        assert_eq!(ms.name, "Alpha release");
        assert!(!ms.reached);
        assert!(ms.reached_at.is_none());
        assert!(ms.notes.is_none());
    }

    #[test]
    fn test_milestone_mark_reached() {
        let mut ms = Milestone::new("Beta");
        ms.mark_reached();
        assert!(ms.reached);
        assert!(ms.reached_at.is_some());
    }

    #[test]
    fn test_milestone_reached_with_note() {
        let mut ms = Milestone::new("Launch");
        ms.mark_reached_with_note("Deployed successfully");
        assert!(ms.reached);
        assert_eq!(ms.notes.as_deref(), Some("Deployed successfully"));
    }

    // -- Goal ---------------------------------------------------------------

    #[test]
    fn test_goal_new() {
        let g = Goal::new("g-1", "Build Zeus");
        assert_eq!(g.id, "g-1");
        assert_eq!(g.title, "Build Zeus");
        assert_eq!(g.status, GoalStatus::Active);
        assert_eq!(g.priority, GoalPriority::Medium);
        assert!(g.subgoal_ids.is_empty());
        assert!(g.milestones.is_empty());
        assert!(g.parent_id.is_none());
    }

    #[test]
    fn test_goal_builders() {
        let deadline = Utc::now() + chrono::Duration::days(30);
        let g = Goal::new("g-1", "Test")
            .with_description("A test goal")
            .with_priority(GoalPriority::High)
            .with_deadline(deadline)
            .with_tags(vec!["sprint", "p1"])
            .with_parent("parent-1");

        assert_eq!(g.description.as_deref(), Some("A test goal"));
        assert_eq!(g.priority, GoalPriority::High);
        assert!(g.deadline.is_some());
        assert_eq!(g.tags, vec!["sprint", "p1"]);
        assert_eq!(g.parent_id.as_deref(), Some("parent-1"));
    }

    #[test]
    fn test_goal_milestones() {
        let mut g = Goal::new("g-1", "Build");
        g.add_milestone("Design");
        g.add_milestone("Implement");
        g.add_milestone("Test");

        assert_eq!(g.milestones.len(), 3);
        assert_eq!(g.milestone_summary(), "0/3");

        assert!(g.reach_milestone("Design"));
        assert_eq!(g.milestone_summary(), "1/3");

        assert!(!g.reach_milestone("NonExistent"));
    }

    #[test]
    fn test_goal_effective_progress_manual() {
        let mut g = Goal::new("g-1", "Test");
        g.set_progress(0.75);
        assert!((g.effective_progress() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_effective_progress_milestones() {
        let mut g = Goal::new("g-1", "Test");
        g.add_milestone("A");
        g.add_milestone("B");
        g.reach_milestone("A");
        assert!((g.effective_progress() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_effective_progress_default() {
        let g = Goal::new("g-1", "Test");
        assert!((g.effective_progress() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_effective_progress_completed() {
        let mut g = Goal::new("g-1", "Test");
        g.complete();
        assert!((g.effective_progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_progress_clamped() {
        let mut g = Goal::new("g-1", "Test");
        g.set_progress(1.5);
        assert!((g.effective_progress() - 1.0).abs() < f64::EPSILON);
        g.set_progress(-0.5);
        assert!((g.effective_progress() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_complete() {
        let mut g = Goal::new("g-1", "Test");
        g.complete();
        assert_eq!(g.status, GoalStatus::Completed);
        assert!((g.effective_progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_goal_abandon() {
        let mut g = Goal::new("g-1", "Test");
        g.abandon();
        assert_eq!(g.status, GoalStatus::Abandoned);
    }

    #[test]
    fn test_goal_pause_resume() {
        let mut g = Goal::new("g-1", "Test");
        g.pause();
        assert_eq!(g.status, GoalStatus::Paused);
        g.resume();
        assert_eq!(g.status, GoalStatus::Active);
    }

    #[test]
    fn test_goal_resume_only_from_paused() {
        let mut g = Goal::new("g-1", "Test");
        g.complete();
        g.resume(); // should not change status
        assert_eq!(g.status, GoalStatus::Completed);
    }

    #[test]
    fn test_goal_overdue() {
        let past = Utc::now() - chrono::Duration::days(1);
        let g = Goal::new("g-1", "Test").with_deadline(past);
        assert!(g.is_overdue());
    }

    #[test]
    fn test_goal_not_overdue_no_deadline() {
        let g = Goal::new("g-1", "Test");
        assert!(!g.is_overdue());
    }

    #[test]
    fn test_goal_not_overdue_completed() {
        let past = Utc::now() - chrono::Duration::days(1);
        let mut g = Goal::new("g-1", "Test").with_deadline(past);
        g.complete();
        assert!(!g.is_overdue()); // terminal goals aren't overdue
    }

    // -- GoalTracker --------------------------------------------------------

    #[test]
    fn test_tracker_new() {
        let t = GoalTracker::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn test_tracker_add_goal() {
        let mut t = GoalTracker::new();
        let id = t.add_goal("Build Zeus");
        assert_eq!(t.len(), 1);
        let g = t.get(&id).unwrap();
        assert_eq!(g.title, "Build Zeus");
    }

    #[test]
    fn test_tracker_add_goal_with() {
        let mut t = GoalTracker::new();
        let goal = Goal::new("", "Custom").with_priority(GoalPriority::Critical);
        let id = t.add_goal_with(goal);
        let g = t.get(&id).unwrap();
        assert_eq!(g.priority, GoalPriority::Critical);
    }

    #[test]
    fn test_tracker_add_subgoal() {
        let mut t = GoalTracker::new();
        let parent_id = t.add_goal("Parent");
        let sub_id = t.add_subgoal(&parent_id, "Child").unwrap();

        assert_eq!(t.len(), 2);
        let child = t.get(&sub_id).unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(parent_id.as_str()));

        let parent = t.get(&parent_id).unwrap();
        assert!(parent.subgoal_ids.contains(&sub_id));
    }

    #[test]
    fn test_tracker_add_subgoal_missing_parent() {
        let mut t = GoalTracker::new();
        assert!(t.add_subgoal("nonexistent", "Child").is_none());
    }

    #[test]
    fn test_tracker_remove() {
        let mut t = GoalTracker::new();
        let id = t.add_goal("To remove");
        let removed = t.remove(&id);
        assert!(removed.is_some());
        assert!(t.is_empty());
    }

    #[test]
    fn test_tracker_remove_with_subgoals() {
        let mut t = GoalTracker::new();
        let parent_id = t.add_goal("Parent");
        let _sub1 = t.add_subgoal(&parent_id, "Sub1");
        let _sub2 = t.add_subgoal(&parent_id, "Sub2");
        assert_eq!(t.len(), 3);

        t.remove(&parent_id);
        // Parent + both subgoals removed
        assert!(t.is_empty());
    }

    #[test]
    fn test_tracker_remove_unlinks_parent() {
        let mut t = GoalTracker::new();
        let parent_id = t.add_goal("Parent");
        let sub_id = t.add_subgoal(&parent_id, "Sub").unwrap();

        t.remove(&sub_id);
        let parent = t.get(&parent_id).unwrap();
        assert!(parent.subgoal_ids.is_empty());
    }

    #[test]
    fn test_tracker_top_level() {
        let mut t = GoalTracker::new();
        let p1 = t.add_goal("High");
        let _p2 = t.add_goal("Low");
        let _sub = t.add_subgoal(&p1, "Subgoal");

        let top = t.top_level();
        assert_eq!(top.len(), 2); // subgoal excluded
    }

    #[test]
    fn test_tracker_top_level_sorted_by_priority() {
        let mut t = GoalTracker::new();
        t.add_goal_with(Goal::new("", "Low").with_priority(GoalPriority::Low));
        t.add_goal_with(Goal::new("", "Critical").with_priority(GoalPriority::Critical));
        t.add_goal_with(Goal::new("", "Medium").with_priority(GoalPriority::Medium));

        let top = t.top_level();
        assert_eq!(top[0].title, "Critical");
        assert_eq!(top[2].title, "Low");
    }

    #[test]
    fn test_tracker_subgoals() {
        let mut t = GoalTracker::new();
        let pid = t.add_goal("Parent");
        t.add_subgoal(&pid, "Sub A");
        t.add_subgoal(&pid, "Sub B");

        let subs = t.subgoals(&pid);
        assert_eq!(subs.len(), 2);
    }

    #[test]
    fn test_tracker_subgoals_missing_parent() {
        let t = GoalTracker::new();
        assert!(t.subgoals("missing").is_empty());
    }

    #[test]
    fn test_tracker_active_goals() {
        let mut t = GoalTracker::new();
        let id1 = t.add_goal("Active 1");
        let id2 = t.add_goal("Active 2");
        let id3 = t.add_goal("Will complete");

        t.get_mut(&id3).unwrap().complete();

        let active = t.active_goals();
        assert_eq!(active.len(), 2);
        let ids: Vec<&str> = active.iter().map(|g| g.id.as_str()).collect();
        assert!(ids.contains(&id1.as_str()));
        assert!(ids.contains(&id2.as_str()));
    }

    #[test]
    fn test_tracker_overdue_goals() {
        let mut t = GoalTracker::new();
        let past = Utc::now() - chrono::Duration::days(1);
        t.add_goal_with(Goal::new("", "Overdue").with_deadline(past));
        t.add_goal("Not overdue");

        let overdue = t.overdue_goals();
        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].title, "Overdue");
    }

    #[test]
    fn test_tracker_search_by_title() {
        let mut t = GoalTracker::new();
        t.add_goal("Build Zeus");
        t.add_goal("Deploy Nova");

        let results = t.search("zeus");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Build Zeus");
    }

    #[test]
    fn test_tracker_search_by_tag() {
        let mut t = GoalTracker::new();
        t.add_goal_with(Goal::new("", "Goal A").with_tags(vec!["sprint1"]));
        t.add_goal_with(Goal::new("", "Goal B").with_tags(vec!["sprint2"]));

        let results = t.search("sprint1");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_tracker_search_by_description() {
        let mut t = GoalTracker::new();
        t.add_goal_with(Goal::new("", "Goal").with_description("implement authentication"));

        let results = t.search("authentication");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_tracker_search_no_results() {
        let mut t = GoalTracker::new();
        t.add_goal("Zeus");
        assert!(t.search("nonexistent_xyz").is_empty());
    }

    // -- Summary ------------------------------------------------------------

    #[test]
    fn test_summary_empty() {
        let t = GoalTracker::new();
        let s = t.summary();
        assert_eq!(s.total_goals, 0);
        assert_eq!(s.active, 0);
        assert!((s.avg_progress - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_summary_mixed() {
        let mut t = GoalTracker::new();
        let id1 = t.add_goal("Active");
        let id2 = t.add_goal("To complete");
        let id3 = t.add_goal("To pause");

        t.get_mut(&id1).unwrap().set_progress(0.5);
        t.get_mut(&id2).unwrap().complete();
        t.get_mut(&id3).unwrap().pause();

        let s = t.summary();
        assert_eq!(s.total_goals, 3);
        assert_eq!(s.active, 1);
        assert_eq!(s.completed, 1);
        assert_eq!(s.paused, 1);
    }

    // -- Recursive progress -------------------------------------------------

    #[test]
    fn test_recursive_progress_leaf() {
        let mut t = GoalTracker::new();
        let id = t.add_goal("Leaf");
        t.get_mut(&id).unwrap().set_progress(0.6);
        assert!((t.recursive_progress(&id) - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn test_recursive_progress_with_subgoals() {
        let mut t = GoalTracker::new();
        let pid = t.add_goal("Parent");
        let s1 = t.add_subgoal(&pid, "Sub1").unwrap();
        let s2 = t.add_subgoal(&pid, "Sub2").unwrap();

        t.get_mut(&s1).unwrap().complete(); // 1.0
        t.get_mut(&s2).unwrap().set_progress(0.5); // 0.5

        // Parent has no manual progress, so recursive = avg(1.0, 0.5) = 0.75
        assert!((t.recursive_progress(&pid) - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_recursive_progress_missing_id() {
        let t = GoalTracker::new();
        assert!((t.recursive_progress("missing") - 0.0).abs() < f64::EPSILON);
    }

    // -- suggest_next -------------------------------------------------------

    #[test]
    fn test_suggest_next_empty() {
        let t = GoalTracker::new();
        assert!(t.suggest_next().is_none());
    }

    #[test]
    fn test_suggest_next_highest_priority() {
        let mut t = GoalTracker::new();
        t.add_goal_with(Goal::new("", "Low").with_priority(GoalPriority::Low));
        t.add_goal_with(Goal::new("", "Critical").with_priority(GoalPriority::Critical));

        let next = t.suggest_next().unwrap();
        assert_eq!(next.title, "Critical");
    }

    #[test]
    fn test_suggest_next_lowest_progress_at_same_priority() {
        let mut t = GoalTracker::new();
        let id1 = t.add_goal_with(Goal::new("", "Far along").with_priority(GoalPriority::High));
        let _id2 = t.add_goal_with(Goal::new("", "Just started").with_priority(GoalPriority::High));

        t.get_mut(&id1).unwrap().set_progress(0.8);
        // id2 has 0.0 progress

        let next = t.suggest_next().unwrap();
        assert_eq!(next.title, "Just started");
    }

    #[test]
    fn test_suggest_next_skips_subgoals() {
        let mut t = GoalTracker::new();
        let pid = t.add_goal("Parent");
        t.add_subgoal(&pid, "Child");

        let next = t.suggest_next().unwrap();
        assert_eq!(next.title, "Parent");
    }

    #[test]
    fn test_suggest_next_skips_completed() {
        let mut t = GoalTracker::new();
        let id1 = t.add_goal("Done");
        t.add_goal("Active");
        t.get_mut(&id1).unwrap().complete();

        let next = t.suggest_next().unwrap();
        assert_eq!(next.title, "Active");
    }
}
