//! Proactive Agent Spawner
//!
//! Analyzes task complexity and automatically recommends spawning new
//! agents when a task would benefit from parallelism or specialization.
//!
//! This is the OpenClaw-equivalent capability: the system proactively
//! creates additional agents to handle subtasks rather than doing
//! everything sequentially in a single agent.
//!
//! # How It Works
//!
//! 1. The autonomy engine's `decide()` detects a complex or parallelizable task.
//! 2. It calls `ProactiveSpawner::analyze()` which examines the task and
//!    produces `SpawnRecommendation` with concrete `SpawnRequest` entries.
//! 3. `Prometheus::process_autonomous()` sees `Decision::SpawnAgents` and
//!    executes the spawns via the `DynamicOrchestrator`.
//! 4. Each spawned agent gets a scoped task, tools, and system prompt.
//! 5. Results are collected and aggregated back into the parent session.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::intent::{IntentAnalysis, TaskComplexity};
use crate::planner::{Plan, Step};

// ============================================================================
// Types
// ============================================================================

/// A request to spawn a new agent for a specific subtask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRequest {
    /// Unique identifier for this spawn request.
    pub id: String,
    /// Human-readable role/title for the agent (e.g., "code-reviewer", "test-runner").
    pub role: String,
    /// The subtask this agent should work on.
    pub task: String,
    /// Tools the agent needs access to.
    pub tools: Vec<String>,
    /// Optional system prompt override for specialization.
    pub system_prompt: Option<String>,
    /// Capabilities this agent should register with the orchestrator.
    pub capabilities: Vec<String>,
    /// Whether this agent should run concurrently with others.
    pub parallel: bool,
    /// Dependency: IDs of spawn requests that must complete first.
    pub depends_on: Vec<String>,
    /// Spawn depth (0 = top-level, incremented for sub-spawns).
    /// Used to enforce the max depth guard and prevent infinite delegation.
    #[serde(default)]
    pub depth: u8,
}

/// The recommendation produced by the spawner after analyzing a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRecommendation {
    /// Whether spawning is recommended.
    pub should_spawn: bool,
    /// The agents to spawn.
    pub agents: Vec<SpawnRequest>,
    /// Human-readable explanation of why spawning was recommended (or not).
    pub rationale: String,
    /// Estimated total parallelism gain (1.0 = no gain, 2.0 = 2x faster, etc.).
    pub estimated_speedup: f32,
}

impl SpawnRecommendation {
    /// Create a "no spawn needed" recommendation.
    pub fn none(reason: &str) -> Self {
        Self {
            should_spawn: false,
            agents: Vec::new(),
            rationale: reason.to_string(),
            estimated_speedup: 1.0,
        }
    }
}

/// Outcome of a spawned agent's execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnOutcome {
    /// The spawn request ID that produced this outcome.
    pub request_id: String,
    /// Agent ID assigned by the orchestrator.
    pub agent_id: String,
    /// Whether the agent completed its task successfully.
    pub success: bool,
    /// The agent's output/result.
    pub output: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// When the agent started.
    pub started_at: DateTime<Utc>,
    /// When the agent finished.
    pub finished_at: DateTime<Utc>,
    /// Spawn depth from the original request (preserved for retry depth inheritance).
    #[serde(default)]
    pub depth: u8,
}

/// Tracks active and completed spawns for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpawnTracker {
    /// Active spawns (not yet completed).
    pub active: Vec<ActiveSpawn>,
    /// Completed spawns with outcomes.
    pub completed: Vec<SpawnOutcome>,
    /// Subagent depth counter for busy-aware fire-decision (`busy: subagent`).
    /// Mirrors the inbox-queue-depth shape from `6656bec3`: `Arc<AtomicUsize>`
    /// on the tracker, `fetch_add` at `track_spawn` BEFORE push, conditional
    /// `fetch_sub` INSIDE the `position(...).Some(idx)` arm of `complete_spawn`.
    /// Reads `> 0` indicate one or more subagent cooks in flight, distinct
    /// from `channel_active` (this-cook in-flight).
    ///
    /// `#[serde(skip, default)]` — `AtomicUsize: !Serialize + !Deserialize`.
    /// Snapshot-semantics justification: a deserialized `SpawnTracker` is a
    /// snapshot from a prior runtime that is dead; fresh-counter-on-deserialize
    /// is correct semantics, not a workaround. The `active` Vec is also a
    /// dead snapshot — those agents are gone with their runtime.
    #[serde(skip, default)]
    pub active_count: Arc<AtomicUsize>,
}

/// An active spawn that hasn't completed yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSpawn {
    pub request: SpawnRequest,
    pub agent_id: String,
    pub started_at: DateTime<Utc>,
}

impl SpawnTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new active spawn.
    ///
    /// Counter-invariant: `fetch_add(1, Relaxed)` BEFORE the infallible push,
    /// mirroring the inbox-counter sender-side ordering at `6656bec3`. Push is
    /// infallible (`Vec::push` never fails), so no rollback arm is needed —
    /// the increment is paired with the push as a single observable transition.
    pub fn track_spawn(&mut self, request: SpawnRequest, agent_id: String) {
        // Counter-invariant: fetch_add BEFORE push (always-increment, push infallible).
        self.active_count.fetch_add(1, Ordering::Relaxed);
        self.active.push(ActiveSpawn {
            request,
            agent_id,
            started_at: Utc::now(),
        });
    }

    /// Mark a spawn as completed and record the outcome.
    ///
    /// Counter-invariant: `fetch_sub(1, Relaxed)` ONLY inside the
    /// `position(...).Some(idx)` arm — conditional-decrement-mirrors-conditional-removal.
    /// Unconditional decrement on a `None` path would violate counter-≥-0 invariant
    /// (caller may invoke `complete_spawn` for an agent_id that was already
    /// completed or never tracked). The `fetch_sub` is co-located with the
    /// `active.remove(idx)` call: same conditional, same observable transition.
    pub fn complete_spawn(&mut self, agent_id: &str, success: bool, output: String) {
        if let Some(idx) = self.active.iter().position(|s| s.agent_id == agent_id) {
            // Counter-invariant: fetch_sub INSIDE the Some(idx) arm, paired with active.remove(idx).
            self.active_count.fetch_sub(1, Ordering::Relaxed);
            let active = self.active.remove(idx);
            self.completed.push(SpawnOutcome {
                request_id: active.request.id,
                agent_id: active.agent_id,
                success,
                output,
                duration_ms: (Utc::now() - active.started_at).num_milliseconds().max(0) as u64,
                started_at: active.started_at,
                finished_at: Utc::now(),
                depth: active.request.depth,
            });
        }
    }

    /// Get the number of currently active spawns.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Get a clone of the `Arc<AtomicUsize>` handle for sharing with `Heartbeat`.
    /// Used by `Prometheus` construction wire-up to pass the busy-aware fire-decision
    /// `busy: subagent` signal to the heartbeat manager. The counter is updated by
    /// `track_spawn` / `complete_spawn` and read via `load(Relaxed)` in fire-decision.
    pub fn active_count_handle(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.active_count)
    }

    /// Get the number of completed spawns.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Check if all spawns have completed.
    pub fn all_done(&self) -> bool {
        self.active.is_empty()
    }

    /// Get the success rate of completed spawns.
    pub fn success_rate(&self) -> f32 {
        if self.completed.is_empty() {
            return 1.0;
        }
        let successes = self.completed.iter().filter(|o| o.success).count();
        successes as f32 / self.completed.len() as f32
    }

    /// Collect all successful outputs.
    pub fn successful_outputs(&self) -> Vec<&str> {
        self.completed
            .iter()
            .filter(|o| o.success)
            .map(|o| o.output.as_str())
            .collect()
    }
}

// ============================================================================
// SpawnCriteria
// ============================================================================

/// Configuration controlling when the spawner recommends new agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnCriteria {
    /// Minimum task complexity to consider spawning.
    #[serde(default = "default_min_complexity")]
    pub min_complexity: TaskComplexity,
    /// Maximum number of agents to spawn in a single recommendation.
    #[serde(default = "default_max_spawn_count")]
    pub max_spawn_count: usize,
    /// Maximum total active agents (including parent).
    #[serde(default = "default_max_active_agents")]
    pub max_active_agents: usize,
    /// Whether to enable specialization-based spawning.
    #[serde(default = "default_enable_specialization")]
    pub enable_specialization: bool,
    /// Whether to enable parallelism-based spawning.
    #[serde(default = "default_enable_parallel")]
    pub enable_parallel: bool,
    /// Minimum number of parallelizable steps in a plan to trigger spawning.
    #[serde(default = "default_min_parallel_steps")]
    pub min_parallel_steps: usize,
    /// Maximum spawn depth (0 = no sub-spawns allowed, 2 = default).
    /// Prevents infinite delegation loops where agents keep spawning sub-agents.
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
}

fn default_min_complexity() -> TaskComplexity {
    TaskComplexity::Moderate
}
fn default_max_spawn_count() -> usize {
    5
}
fn default_max_active_agents() -> usize {
    8
}
fn default_enable_specialization() -> bool {
    true
}
fn default_enable_parallel() -> bool {
    true
}
fn default_min_parallel_steps() -> usize {
    2
}
fn default_max_depth() -> u8 {
    2
}

impl Default for SpawnCriteria {
    fn default() -> Self {
        Self {
            min_complexity: default_min_complexity(),
            max_spawn_count: default_max_spawn_count(),
            max_active_agents: default_max_active_agents(),
            enable_specialization: default_enable_specialization(),
            enable_parallel: default_enable_parallel(),
            min_parallel_steps: default_min_parallel_steps(),
            max_depth: default_max_depth(),
        }
    }
}

// ============================================================================
// ProactiveSpawner
// ============================================================================

/// The proactive agent spawner.
///
/// Analyzes tasks and recommends spawning additional agents when it
/// would improve throughput, quality, or specialization.
pub struct ProactiveSpawner {
    criteria: SpawnCriteria,
    tracker: SpawnTracker,
}

impl ProactiveSpawner {
    pub fn new(criteria: SpawnCriteria) -> Self {
        Self {
            criteria,
            tracker: SpawnTracker::new(),
        }
    }

    /// Get the spawn criteria.
    pub fn criteria(&self) -> &SpawnCriteria {
        &self.criteria
    }

    /// Get the spawn tracker.
    pub fn tracker(&self) -> &SpawnTracker {
        &self.tracker
    }

    /// Get a mutable reference to the spawn tracker.
    pub fn tracker_mut(&mut self) -> &mut SpawnTracker {
        &mut self.tracker
    }

    /// Analyze a task and determine if agent spawning is recommended.
    ///
    /// Uses the intent analysis to check complexity and the plan (if
    /// available) to detect parallelizable subtasks.
    pub fn analyze(
        &self,
        intent: &IntentAnalysis,
        plan: Option<&Plan>,
        current_active: usize,
    ) -> SpawnRecommendation {
        self.analyze_at_depth(intent, plan, current_active, 0)
    }

    /// Analyze with an explicit spawn depth. Sub-agents call this with depth > 0.
    pub fn analyze_at_depth(
        &self,
        intent: &IntentAnalysis,
        plan: Option<&Plan>,
        current_active: usize,
        depth: u8,
    ) -> SpawnRecommendation {
        // Check depth guard — prevent infinite delegation loops
        if depth >= self.criteria.max_depth {
            return SpawnRecommendation::none(&format!(
                "Spawn depth {} reached max_depth {} — delegation halted",
                depth, self.criteria.max_depth
            ));
        }

        // Check if we're at capacity
        if current_active >= self.criteria.max_active_agents {
            return SpawnRecommendation::none("At maximum active agent capacity");
        }

        let available_slots = self.criteria.max_active_agents - current_active;

        // Check complexity threshold
        if !meets_complexity_threshold(&intent.complexity, &self.criteria.min_complexity) {
            return SpawnRecommendation::none(&format!(
                "Task complexity {:?} below threshold {:?}",
                intent.complexity, self.criteria.min_complexity
            ));
        }

        // Try plan-based analysis first (most accurate)
        if let Some(plan) = plan
            && let Some(rec) = self.analyze_plan(plan, available_slots, depth)
        {
            return rec;
        }

        // Fall back to intent-based analysis
        self.analyze_intent(intent, available_slots, depth)
    }

    /// Analyze a plan for parallelizable subtasks.
    fn analyze_plan(&self, plan: &Plan, available_slots: usize, depth: u8) -> Option<SpawnRecommendation> {
        if !self.criteria.enable_parallel {
            return None;
        }

        // Find steps that can run in parallel (same dependency level)
        let parallel_groups = find_parallel_groups(&plan.steps);

        // Look for the largest parallel group
        let max_group = parallel_groups
            .iter()
            .max_by_key(|g| g.len())
            .cloned()
            .unwrap_or_default();

        if max_group.len() < self.criteria.min_parallel_steps {
            return None;
        }

        let spawn_count = max_group
            .len()
            .min(available_slots)
            .min(self.criteria.max_spawn_count);

        let agents: Vec<SpawnRequest> = max_group
            .iter()
            .take(spawn_count)
            .map(|step| {
                let tools = step.tool.iter().cloned().collect();
                SpawnRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: format!("worker-{}", step.id),
                    task: step.description.clone(),
                    tools,
                    system_prompt: Some(format!(
                        "You are a focused worker agent. Complete this single task:\n{}",
                        step.description
                    )),
                    capabilities: vec!["task-execution".to_string()],
                    parallel: true,
                    depends_on: Vec::new(),
                    depth: depth + 1,
                }
            })
            .collect();

        let speedup = if agents.is_empty() {
            1.0
        } else {
            agents.len() as f32 * 0.7 // Assume 70% efficiency per parallel agent
        };

        info!(
            agents = agents.len(),
            parallel_group_size = max_group.len(),
            estimated_speedup = speedup,
            "Plan-based spawn recommendation"
        );

        Some(SpawnRecommendation {
            should_spawn: !agents.is_empty(),
            agents,
            rationale: format!(
                "Plan has {} parallelizable steps; spawning agents for concurrent execution",
                max_group.len()
            ),
            estimated_speedup: speedup,
        })
    }

    /// Analyze an intent for specialization opportunities.
    fn analyze_intent(
        &self,
        intent: &IntentAnalysis,
        available_slots: usize,
        depth: u8,
    ) -> SpawnRecommendation {
        if !self.criteria.enable_specialization {
            return SpawnRecommendation::none("Specialization disabled");
        }

        // Only spawn for complex tasks with multiple suggested tools
        if intent.suggested_tools.len() < 2 {
            return SpawnRecommendation::none("Not enough tool diversity for specialization");
        }

        let is_complex = matches!(intent.complexity, TaskComplexity::Complex);

        if !is_complex {
            return SpawnRecommendation::none("Task not complex enough for specialization");
        }

        // Group tools by category for specialization
        let tool_groups = categorize_tools(&intent.suggested_tools);

        if tool_groups.len() < 2 {
            return SpawnRecommendation::none("Tools are all same category");
        }

        let spawn_count = tool_groups
            .len()
            .min(available_slots)
            .min(self.criteria.max_spawn_count);

        let agents: Vec<SpawnRequest> = tool_groups
            .into_iter()
            .take(spawn_count)
            .map(|(category, tools)| SpawnRequest {
                id: uuid::Uuid::new_v4().to_string(),
                role: format!("{}-specialist", category),
                task: format!(
                    "Handle the {} aspect of: {}",
                    category,
                    intent.suggested_tools.join(", ")
                ),
                tools,
                system_prompt: Some(format!(
                    "You are a specialist agent focused on {} operations. \
                     Use only the tools in your domain.",
                    category
                )),
                capabilities: vec![category.clone()],
                parallel: true,
                depends_on: Vec::new(),
                depth: depth + 1,
            })
            .collect();

        let speedup = if agents.is_empty() {
            1.0
        } else {
            1.0 + (agents.len() as f32 - 1.0) * 0.5
        };

        debug!(
            agents = agents.len(),
            speedup = speedup,
            "Specialization-based spawn recommendation"
        );

        SpawnRecommendation {
            should_spawn: !agents.is_empty(),
            agents,
            rationale: "Task benefits from specialized agents for different tool categories"
                .to_string(),
            estimated_speedup: speedup,
        }
    }

    /// Handle a spawn failure: mark the failed spawn, and optionally produce a
    /// replacement recommendation (e.g., a single-agent fallback for the failed task).
    ///
    /// Returns `Some(SpawnRequest)` if a retry/replacement is viable, `None` if
    /// the failure should be absorbed (e.g., max retries exceeded).
    pub fn handle_spawn_failure(
        &mut self,
        agent_id: &str,
        error: &str,
        max_retries: usize,
    ) -> Option<SpawnRequest> {
        // Record the failure in the tracker
        self.tracker
            .complete_spawn(agent_id, false, format!("Spawn failure: {}", error));

        // Find the original request from completed outcomes
        let failed_outcome = self
            .tracker
            .completed
            .iter()
            .rev()
            .find(|o| o.agent_id == agent_id && !o.success);

        let request_id = match failed_outcome {
            Some(o) => o.request_id.clone(),
            None => {
                info!(agent_id, "Spawn failure for unknown agent — cannot retry");
                return None;
            }
        };

        // Count how many times this request_id has failed
        let failure_count = self
            .tracker
            .completed
            .iter()
            .filter(|o| o.request_id == request_id && !o.success)
            .count();

        if failure_count > max_retries {
            info!(
                request_id,
                failure_count, max_retries, "Spawn retry budget exhausted — absorbing failure"
            );
            return None;
        }

        // Build a replacement request with a new ID but same task
        let replacement = SpawnRequest {
            id: uuid::Uuid::new_v4().to_string(),
            role: format!("retry-{}", request_id.chars().take(8).collect::<String>()),
            task: failed_outcome
                .map(|o| {
                    // Strip the "Spawn failure: " prefix from output to get original error context
                    format!(
                        "[Retry after failure: {}] Original task for request {}",
                        error, o.request_id
                    )
                })
                .unwrap_or_else(|| "Retry of failed spawn".to_string()),
            tools: Vec::new(), // Will be filled by the caller from the original request
            system_prompt: None,
            capabilities: vec!["task-execution".to_string()],
            parallel: false, // Retries run sequentially
            depends_on: Vec::new(),
            depth: failed_outcome.map(|o| o.depth).unwrap_or(0), // Retries inherit parent depth
        };

        info!(
            original_request = %request_id,
            retry_id = %replacement.id,
            attempt = failure_count + 1,
            "Recommending spawn retry"
        );

        Some(replacement)
    }

    /// Handle a route_task failure where the orchestrator couldn't create an agent.
    ///
    /// Unlike `handle_spawn_failure()` (which requires a pre-tracked agent_id),
    /// this takes the original `SpawnRequest` directly — useful when `route_task()`
    /// itself returns an error before any agent exists.
    ///
    /// Returns `Some(SpawnRequest)` if retry is viable (under budget),
    /// or `None` if the retry budget is exhausted and the caller should fall back.
    pub fn handle_route_failure(
        &mut self,
        original: &SpawnRequest,
        error: &str,
        max_retries: usize,
    ) -> Option<SpawnRequest> {
        // Record a synthetic failure outcome so retry counting works
        let failure_id = format!("route-fail-{}", uuid::Uuid::new_v4());
        self.tracker.completed.push(SpawnOutcome {
            request_id: original.id.clone(),
            agent_id: failure_id,
            success: false,
            output: format!("Route failure: {}", error),
            duration_ms: 0,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            depth: original.depth,
        });

        // Count total failures for this request
        let failure_count = self
            .tracker
            .completed
            .iter()
            .filter(|o| o.request_id == original.id && !o.success)
            .count();

        if failure_count > max_retries {
            info!(
                request_id = %original.id,
                failure_count,
                max_retries,
                "Route retry budget exhausted for '{}'",
                original.role
            );
            return None;
        }

        info!(
            request_id = %original.id,
            role = %original.role,
            attempt = failure_count + 1,
            "Recommending route retry after failure"
        );

        // Return the same request for another attempt
        Some(original.clone())
    }

    /// Get a summary of spawn health for the current session.
    pub fn health_summary(&self) -> SpawnHealthSummary {
        let total = self.tracker.completed.len();
        let failures = self.tracker.completed.iter().filter(|o| !o.success).count();
        let active = self.tracker.active.len();

        SpawnHealthSummary {
            active_spawns: active,
            completed_total: total,
            completed_failures: failures,
            success_rate: self.tracker.success_rate(),
            is_healthy: failures as f32 / (total.max(1) as f32) < 0.5,
        }
    }
}

/// Summary of spawn health for monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnHealthSummary {
    pub active_spawns: usize,
    pub completed_total: usize,
    pub completed_failures: usize,
    pub success_rate: f32,
    pub is_healthy: bool,
}

impl Default for ProactiveSpawner {
    fn default() -> Self {
        Self::new(SpawnCriteria::default())
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Check if a task complexity meets the minimum threshold.
fn meets_complexity_threshold(actual: &TaskComplexity, threshold: &TaskComplexity) -> bool {
    complexity_rank(actual) >= complexity_rank(threshold)
}

fn complexity_rank(c: &TaskComplexity) -> u8 {
    match c {
        TaskComplexity::Trivial => 0,
        TaskComplexity::Simple => 1,
        TaskComplexity::Moderate => 2,
        TaskComplexity::Complex => 3,
    }
}

/// Find groups of steps that can run in parallel (independent of each other).
fn find_parallel_groups(steps: &[Step]) -> Vec<Vec<&Step>> {
    // Group by dependency depth level
    let mut depth_map: std::collections::HashMap<usize, Vec<&Step>> =
        std::collections::HashMap::new();

    for step in steps {
        let depth = if step.dependencies.is_empty() {
            0
        } else {
            // Find max depth among dependencies + 1
            step.dependencies
                .iter()
                .filter_map(|dep_id| steps.iter().find(|s| s.id == *dep_id))
                .map(|dep| {
                    if dep.dependencies.is_empty() {
                        1
                    } else {
                        2 // Simplified: just mark as deeper
                    }
                })
                .max()
                .unwrap_or(1)
        };

        depth_map.entry(depth).or_default().push(step);
    }

    // Return groups with more than 1 step (parallelizable)
    depth_map.into_values().filter(|g| g.len() > 1).collect()
}

/// Categorize tools by their functional domain.
fn categorize_tools(tools: &[String]) -> Vec<(String, Vec<String>)> {
    let mut categories: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for tool in tools {
        let category = match tool.as_str() {
            "read_file" | "write_file" | "edit_file" | "list_dir" => "filesystem",
            "shell" => "system",
            "web_fetch" => "network",
            "spawn" => "orchestration",
            "message" => "communication",
            t if t.starts_with("git_") => "version-control",
            t if t.starts_with("talos_") => "automation",
            t if t.starts_with("browser_") => "browser",
            _ => "general",
        };

        categories
            .entry(category.to_string())
            .or_default()
            .push(tool.clone());
    }

    categories.into_iter().collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Intent, IntentAnalysis};
    use crate::planner::{Step, StepStatus};

    fn make_intent(complexity: TaskComplexity, tools: Vec<&str>) -> IntentAnalysis {
        IntentAnalysis {
            intent: Intent::ComplexTask,
            confidence: 0.9,
            complexity,
            suggested_tools: tools.into_iter().map(String::from).collect(),
            requires_confirmation: false,
            reasoning: "test".to_string(),
        }
    }

    fn make_plan(steps: Vec<Step>) -> Plan {
        Plan {
            task: "test task".to_string(),
            steps,
            status: crate::planner::PlanStatus::Created,
        }
    }

    fn make_step(id: usize, tool: Option<&str>, deps: Vec<usize>) -> Step {
        Step {
            id,
            description: format!("Step {}", id),
            tool: tool.map(String::from),
            arguments: None,
            dependencies: deps,
            status: StepStatus::Pending,
            output: None,
        }
    }

    // ========================================================================
    // SpawnRecommendation tests
    // ========================================================================

    #[test]
    fn test_spawn_recommendation_none() {
        let rec = SpawnRecommendation::none("too simple");
        assert!(!rec.should_spawn);
        assert!(rec.agents.is_empty());
        assert_eq!(rec.estimated_speedup, 1.0);
        assert_eq!(rec.rationale, "too simple");
    }

    // ========================================================================
    // SpawnTracker tests
    // ========================================================================

    #[test]
    fn test_spawn_tracker_lifecycle() {
        let mut tracker = SpawnTracker::new();
        assert!(tracker.all_done());
        assert_eq!(tracker.active_count(), 0);
        assert_eq!(tracker.completed_count(), 0);
        assert_eq!(tracker.success_rate(), 1.0);

        let req = SpawnRequest {
            id: "req-1".to_string(),
            role: "worker".to_string(),
            task: "do stuff".to_string(),
            tools: vec!["shell".to_string()],
            system_prompt: None,
            capabilities: vec![],
            parallel: true,
            depends_on: vec![],
            depth: 0,
        };

        tracker.track_spawn(req, "agent-1".to_string());
        assert_eq!(tracker.active_count(), 1);
        assert!(!tracker.all_done());

        tracker.complete_spawn("agent-1", true, "done".to_string());
        assert!(tracker.all_done());
        assert_eq!(tracker.completed_count(), 1);
        assert_eq!(tracker.success_rate(), 1.0);
        assert_eq!(tracker.successful_outputs(), vec!["done"]);
    }

    #[test]
    fn test_spawn_tracker_mixed_outcomes() {
        let mut tracker = SpawnTracker::new();

        for i in 0..4 {
            let req = SpawnRequest {
                id: format!("req-{i}"),
                role: "worker".to_string(),
                task: format!("task {i}"),
                tools: vec![],
                system_prompt: None,
                capabilities: vec![],
                parallel: true,
                depends_on: vec![],
            depth: 0,
            };
            tracker.track_spawn(req, format!("agent-{i}"));
        }

        tracker.complete_spawn("agent-0", true, "ok".to_string());
        tracker.complete_spawn("agent-1", false, "fail".to_string());
        tracker.complete_spawn("agent-2", true, "ok2".to_string());
        tracker.complete_spawn("agent-3", true, "ok3".to_string());

        assert_eq!(tracker.success_rate(), 0.75);
        assert_eq!(tracker.successful_outputs().len(), 3);
    }

    #[test]
    fn test_spawn_tracker_complete_unknown_agent() {
        let mut tracker = SpawnTracker::new();
        tracker.complete_spawn("nonexistent", true, "output".to_string());
        assert_eq!(tracker.completed_count(), 0);
    }

    // ========================================================================
    // ProactiveSpawner tests
    // ========================================================================

    #[test]
    fn test_spawner_rejects_simple_tasks() {
        let spawner = ProactiveSpawner::default();
        let intent = make_intent(TaskComplexity::Simple, vec!["shell"]);

        let rec = spawner.analyze(&intent, None, 1);
        assert!(!rec.should_spawn);
        assert!(rec.rationale.contains("below threshold"));
    }

    #[test]
    fn test_spawner_rejects_at_capacity() {
        let spawner = ProactiveSpawner::new(SpawnCriteria {
            max_active_agents: 3,
            ..Default::default()
        });
        let intent = make_intent(
            TaskComplexity::Complex,
            vec!["shell", "read_file", "web_fetch"],
        );

        let rec = spawner.analyze(&intent, None, 3);
        assert!(!rec.should_spawn);
        assert!(rec.rationale.contains("capacity"));
    }

    #[test]
    fn test_spawner_plan_based_parallel() {
        let spawner = ProactiveSpawner::default();
        let intent = make_intent(TaskComplexity::Complex, vec!["shell", "read_file"]);

        // Plan with 3 independent steps at depth 0
        let plan = make_plan(vec![
            make_step(1, Some("shell"), vec![]),
            make_step(2, Some("read_file"), vec![]),
            make_step(3, Some("shell"), vec![]),
            make_step(4, Some("write_file"), vec![1, 2, 3]),
        ]);

        let rec = spawner.analyze(&intent, Some(&plan), 1);
        assert!(rec.should_spawn);
        assert!(!rec.agents.is_empty());
        assert!(rec.estimated_speedup > 1.0);
        assert!(rec.rationale.contains("parallelizable"));

        // All spawned agents should be parallel
        for agent in &rec.agents {
            assert!(agent.parallel);
        }
    }

    #[test]
    fn test_spawner_intent_based_specialization() {
        let spawner = ProactiveSpawner::default();

        // High complexity with diverse tools
        let intent = make_intent(
            TaskComplexity::Complex,
            vec!["shell", "read_file", "web_fetch", "browser_navigate"],
        );

        let rec = spawner.analyze(&intent, None, 1);
        assert!(rec.should_spawn);
        // Should create specialists for different tool categories
        assert!(rec.agents.len() >= 2);
        assert!(rec.rationale.contains("specialized"));
    }

    #[test]
    fn test_spawner_rejects_single_tool_category() {
        let spawner = ProactiveSpawner::default();

        // All tools in same category
        let intent = make_intent(
            TaskComplexity::Complex,
            vec!["read_file", "write_file", "edit_file"],
        );

        let rec = spawner.analyze(&intent, None, 1);
        // Should not spawn since all tools are filesystem
        assert!(!rec.should_spawn || rec.agents.len() <= 1);
    }

    #[test]
    fn test_spawner_respects_max_spawn_count() {
        let spawner = ProactiveSpawner::new(SpawnCriteria {
            max_spawn_count: 2,
            ..Default::default()
        });

        let plan = make_plan(vec![
            make_step(1, Some("shell"), vec![]),
            make_step(2, Some("read_file"), vec![]),
            make_step(3, Some("shell"), vec![]),
            make_step(4, Some("shell"), vec![]),
            make_step(5, Some("write_file"), vec![1, 2, 3, 4]),
        ]);

        let intent = make_intent(TaskComplexity::Complex, vec!["shell"]);
        let rec = spawner.analyze(&intent, Some(&plan), 1);

        if rec.should_spawn {
            assert!(rec.agents.len() <= 2);
        }
    }

    #[test]
    fn test_spawner_parallel_disabled() {
        let spawner = ProactiveSpawner::new(SpawnCriteria {
            enable_parallel: false,
            enable_specialization: false,
            ..Default::default()
        });

        let intent = make_intent(TaskComplexity::Complex, vec!["shell", "web_fetch"]);
        let plan = make_plan(vec![
            make_step(1, Some("shell"), vec![]),
            make_step(2, Some("web_fetch"), vec![]),
        ]);

        let rec = spawner.analyze(&intent, Some(&plan), 1);
        assert!(!rec.should_spawn);
    }

    // ========================================================================
    // SpawnCriteria tests
    // ========================================================================

    #[test]
    fn test_spawn_criteria_defaults() {
        let criteria = SpawnCriteria::default();
        assert_eq!(criteria.max_spawn_count, 5);
        assert_eq!(criteria.max_active_agents, 8);
        assert!(criteria.enable_specialization);
        assert!(criteria.enable_parallel);
        assert_eq!(criteria.min_parallel_steps, 2);
    }

    #[test]
    fn test_spawn_criteria_serialization() {
        let criteria = SpawnCriteria::default();
        let json = serde_json::to_value(&criteria).unwrap();
        assert!(json.get("max_spawn_count").is_some());

        let back: SpawnCriteria = serde_json::from_value(json).unwrap();
        assert_eq!(back.max_spawn_count, criteria.max_spawn_count);
    }

    // ========================================================================
    // Helper tests
    // ========================================================================

    #[test]
    fn test_complexity_threshold() {
        assert!(meets_complexity_threshold(
            &TaskComplexity::Complex,
            &TaskComplexity::Moderate
        ));
        assert!(meets_complexity_threshold(
            &TaskComplexity::Moderate,
            &TaskComplexity::Moderate
        ));
        assert!(!meets_complexity_threshold(
            &TaskComplexity::Simple,
            &TaskComplexity::Moderate
        ));
        assert!(meets_complexity_threshold(
            &TaskComplexity::Complex,
            &TaskComplexity::Simple
        ));
    }

    #[test]
    fn test_categorize_tools() {
        let tools = vec![
            "read_file".to_string(),
            "shell".to_string(),
            "web_fetch".to_string(),
            "write_file".to_string(),
        ];

        let cats = categorize_tools(&tools);
        assert!(cats.len() >= 2); // At least filesystem + system + network
    }

    #[test]
    fn test_find_parallel_groups() {
        let steps = vec![
            make_step(1, Some("shell"), vec![]),
            make_step(2, Some("read_file"), vec![]),
            make_step(3, Some("shell"), vec![]),
            make_step(4, Some("write_file"), vec![1, 2, 3]),
        ];

        let groups = find_parallel_groups(&steps);
        // Steps 1, 2, 3 are independent (depth 0) → one parallel group
        assert!(!groups.is_empty());
        let biggest = groups.iter().max_by_key(|g| g.len()).unwrap();
        assert!(biggest.len() >= 2);
    }

    #[test]
    fn test_find_parallel_groups_sequential() {
        // All sequential: each depends on the previous
        let steps = vec![
            make_step(1, Some("shell"), vec![]),
            make_step(2, Some("read_file"), vec![1]),
            make_step(3, Some("write_file"), vec![2]),
        ];

        let groups = find_parallel_groups(&steps);
        // No group should have more than 1 step (all sequential)
        assert!(groups.is_empty());
    }

    // ========================================================================
    // Spawn failure recovery tests
    // ========================================================================

    #[test]
    fn test_handle_spawn_failure_produces_retry() {
        let mut spawner = ProactiveSpawner::default();
        let req = SpawnRequest {
            id: "req-fail".to_string(),
            role: "worker".to_string(),
            task: "build module".to_string(),
            tools: vec!["shell".to_string()],
            system_prompt: None,
            capabilities: vec![],
            parallel: true,
            depends_on: vec![],
            depth: 0,
        };

        spawner
            .tracker_mut()
            .track_spawn(req, "agent-fail".to_string());
        let retry = spawner.handle_spawn_failure("agent-fail", "connection refused", 2);
        assert!(retry.is_some());
        let retry = retry.unwrap();
        assert!(!retry.parallel); // Retries are sequential
        assert!(retry.role.starts_with("retry-"));
    }

    #[test]
    fn test_handle_spawn_failure_exhausts_retries() {
        let mut spawner = ProactiveSpawner::default();

        // First failure
        let req = SpawnRequest {
            id: "req-x".to_string(),
            role: "worker".to_string(),
            task: "build module".to_string(),
            tools: vec![],
            system_prompt: None,
            capabilities: vec![],
            parallel: true,
            depends_on: vec![],
            depth: 0,
        };
        spawner
            .tracker_mut()
            .track_spawn(req, "agent-x1".to_string());
        let retry1 = spawner.handle_spawn_failure("agent-x1", "timeout", 1);
        assert!(retry1.is_some()); // First retry OK

        // Second failure — manually complete the retry as failed too
        // The tracker already has 1 failure with request_id "req-x"
        // Simulate: the retry agent also fails
        spawner.tracker_mut().completed.push(SpawnOutcome {
            request_id: "req-x".to_string(),
            agent_id: "agent-x2".to_string(),
            success: false,
            output: "Spawn failure: timeout again".to_string(),
            duration_ms: 100,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            depth: 0,
        });

        // Now there are 2 failures for req-x, max_retries=1 → should be exhausted
        // We need to add agent-x2 as active first then fail it
        let req2 = SpawnRequest {
            id: "req-x".to_string(),
            role: "retry".to_string(),
            task: "retry".to_string(),
            tools: vec![],
            system_prompt: None,
            capabilities: vec![],
            parallel: false,
            depends_on: vec![],
            depth: 0,
        };
        spawner
            .tracker_mut()
            .track_spawn(req2, "agent-x3".to_string());
        let retry2 = spawner.handle_spawn_failure("agent-x3", "timeout again", 1);
        // 3 failures for req-x now, max_retries=1 → exhausted
        assert!(retry2.is_none());
    }

    #[test]
    fn test_handle_spawn_failure_unknown_agent() {
        let mut spawner = ProactiveSpawner::default();
        let retry = spawner.handle_spawn_failure("nonexistent", "error", 3);
        assert!(retry.is_none()); // Can't retry an unknown agent
    }

    #[test]
    fn test_spawn_health_summary() {
        let mut spawner = ProactiveSpawner::default();

        for i in 0..4 {
            let req = SpawnRequest {
                id: format!("req-{i}"),
                role: "worker".to_string(),
                task: format!("task {i}"),
                tools: vec![],
                system_prompt: None,
                capabilities: vec![],
                parallel: true,
                depends_on: vec![],
            depth: 0,
            };
            spawner.tracker_mut().track_spawn(req, format!("agent-{i}"));
        }

        spawner
            .tracker_mut()
            .complete_spawn("agent-0", true, "ok".to_string());
        spawner
            .tracker_mut()
            .complete_spawn("agent-1", false, "fail".to_string());
        spawner
            .tracker_mut()
            .complete_spawn("agent-2", true, "ok".to_string());

        let health = spawner.health_summary();
        assert_eq!(health.active_spawns, 1); // agent-3 still active
        assert_eq!(health.completed_total, 3);
        assert_eq!(health.completed_failures, 1);
        assert!(health.is_healthy); // 1/3 < 50%
    }

    #[test]
    fn test_spawn_request_serialization() {
        let req = SpawnRequest {
            id: "test-1".to_string(),
            role: "worker".to_string(),
            task: "do stuff".to_string(),
            tools: vec!["shell".to_string()],
            system_prompt: Some("You are a worker".to_string()),
            capabilities: vec!["execute".to_string()],
            parallel: true,
            depends_on: vec![],
            depth: 0,
        };

        let json = serde_json::to_string(&req).unwrap();
        let back: SpawnRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "test-1");
        assert_eq!(back.role, "worker");
        assert!(back.parallel);
    }

    // ========================================================================
    // Route failure recovery tests
    // ========================================================================

    #[test]
    fn test_handle_route_failure_retries() {
        let mut spawner = ProactiveSpawner::default();
        let req = SpawnRequest {
            id: "req-route".to_string(),
            role: "code-reviewer".to_string(),
            task: "review PR #42".to_string(),
            tools: vec!["read_file".to_string()],
            system_prompt: None,
            capabilities: vec!["review".to_string()],
            parallel: true,
            depends_on: vec![],
            depth: 0,
        };

        // First failure — should get a retry
        let retry = spawner.handle_route_failure(&req, "no agents available", 2);
        assert!(retry.is_some());
        let retry_req = retry.unwrap();
        assert_eq!(retry_req.id, "req-route"); // Same request returned
        assert_eq!(retry_req.task, "review PR #42");

        // Second failure — still within budget (2 retries allowed)
        let retry2 = spawner.handle_route_failure(&req, "still no agents", 2);
        assert!(retry2.is_some());

        // Third failure — now exhausted (> max_retries of 2)
        let retry3 = spawner.handle_route_failure(&req, "giving up", 2);
        assert!(retry3.is_none());
    }

    #[test]
    fn test_handle_route_failure_zero_retries() {
        let mut spawner = ProactiveSpawner::default();
        let req = SpawnRequest {
            id: "req-no-retry".to_string(),
            role: "worker".to_string(),
            task: "task".to_string(),
            tools: vec![],
            system_prompt: None,
            capabilities: vec![],
            parallel: false,
            depends_on: vec![],
            depth: 0,
        };

        // With max_retries=0, first failure should exhaust budget
        let retry = spawner.handle_route_failure(&req, "error", 0);
        assert!(retry.is_none());
    }

    #[test]
    fn test_handle_spawn_failure_inherits_depth() {
        let mut spawner = ProactiveSpawner::default();
        let req = SpawnRequest {
            id: "req-deep".to_string(),
            role: "worker".to_string(),
            task: "deep task".to_string(),
            tools: vec!["shell".to_string()],
            system_prompt: None,
            capabilities: vec![],
            parallel: true,
            depends_on: vec![],
            depth: 2, // Non-zero depth
        };

        spawner
            .tracker_mut()
            .track_spawn(req, "agent-deep".to_string());
        let retry = spawner.handle_spawn_failure("agent-deep", "timeout", 3);
        assert!(retry.is_some());
        let retry = retry.unwrap();
        assert_eq!(retry.depth, 2); // Must inherit parent depth, not reset to 0
    }

    #[test]
    fn test_handle_route_failure_records_outcomes() {
        let mut spawner = ProactiveSpawner::default();
        let req = SpawnRequest {
            id: "req-track".to_string(),
            role: "worker".to_string(),
            task: "task".to_string(),
            tools: vec![],
            system_prompt: None,
            capabilities: vec![],
            parallel: false,
            depends_on: vec![],
            depth: 0,
        };

        // Two failures
        spawner.handle_route_failure(&req, "err1", 5);
        spawner.handle_route_failure(&req, "err2", 5);

        // Should have recorded 2 failed outcomes
        let failures = spawner
            .tracker()
            .completed
            .iter()
            .filter(|o| o.request_id == "req-track" && !o.success)
            .count();
        assert_eq!(failures, 2);

        // Health summary should reflect failures
        let health = spawner.health_summary();
        assert_eq!(health.completed_failures, 2);
    }
}
