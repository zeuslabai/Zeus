//! Strategic Planner - DAG analysis on top of the existing Planner
//!
//! Converts a `Plan` (from `planner.rs`) into a `TaskDAG` with:
//! - Topological ordering (Kahn's algorithm, detects cycles)
//! - Parallel group detection (BFS level ordering)
//! - Critical path computation (DP longest path)
//! - Ready-task tracking for incremental execution

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::planner::Plan;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Status of a node in the DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// A single node in the task DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// Step ID (1-indexed, from the Plan).
    pub step_id: usize,
    pub description: String,
    pub tool: Option<String>,
    /// Estimated duration in milliseconds.
    pub est_duration_ms: u64,
    /// Agent assigned to this task (filled during execution).
    pub assigned_agent: Option<String>,
    pub status: TaskNodeStatus,
}

/// Directed Acyclic Graph of tasks derived from a Plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDAG {
    /// Nodes keyed by step_id.
    pub nodes: HashMap<usize, TaskNode>,
    /// Forward edges: step_id -> set of dependent step_ids.
    pub forward_edges: HashMap<usize, Vec<usize>>,
    /// Reverse edges: step_id -> set of prerequisite step_ids.
    pub reverse_edges: HashMap<usize, Vec<usize>>,
    /// Topologically sorted step IDs.
    pub topological_order: Vec<usize>,
}

impl TaskDAG {
    /// Build a DAG from a `Plan`, using Kahn's algorithm for topological sort.
    /// Returns an error if the dependency graph contains a cycle.
    pub fn from_plan(plan: &Plan, default_duration_ms: u64) -> Result<Self, String> {
        let mut nodes = HashMap::new();
        let mut forward_edges: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut reverse_edges: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut in_degree: HashMap<usize, usize> = HashMap::new();

        // Build nodes and edges
        for step in &plan.steps {
            nodes.insert(
                step.id,
                TaskNode {
                    step_id: step.id,
                    description: step.description.clone(),
                    tool: step.tool.clone(),
                    est_duration_ms: default_duration_ms,
                    assigned_agent: None,
                    status: TaskNodeStatus::Pending,
                },
            );
            forward_edges.entry(step.id).or_default();
            reverse_edges.entry(step.id).or_default();
            in_degree.entry(step.id).or_insert(0);

            for &dep in &step.dependencies {
                forward_edges.entry(dep).or_default().push(step.id);
                reverse_edges.entry(step.id).or_default().push(dep);
                *in_degree.entry(step.id).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm
        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| *id)
            .collect();
        // Sort for determinism
        let mut sorted_start: Vec<usize> = queue.drain(..).collect();
        sorted_start.sort_unstable();
        queue.extend(sorted_start);

        let mut topo_order = Vec::with_capacity(nodes.len());
        while let Some(node) = queue.pop_front() {
            topo_order.push(node);
            if let Some(deps) = forward_edges.get(&node) {
                let mut next_ready = Vec::new();
                for &dep in deps {
                    if let Some(deg) = in_degree.get_mut(&dep) {
                        *deg -= 1;
                        if *deg == 0 {
                            next_ready.push(dep);
                        }
                    }
                }
                next_ready.sort_unstable();
                queue.extend(next_ready);
            }
        }

        if topo_order.len() != nodes.len() {
            return Err("dependency cycle detected in plan".to_string());
        }

        Ok(Self {
            nodes,
            forward_edges,
            reverse_edges,
            topological_order: topo_order,
        })
    }

    /// Identify parallel execution groups via BFS level ordering.
    /// Tasks in the same group have no mutual dependencies and can run concurrently.
    pub fn parallel_groups(&self) -> Vec<Vec<usize>> {
        if self.nodes.is_empty() {
            return vec![];
        }

        let mut levels: HashMap<usize, usize> = HashMap::new();
        let mut max_level = 0;

        for &node_id in &self.topological_order {
            let deps = self
                .reverse_edges
                .get(&node_id)
                .cloned()
                .unwrap_or_default();
            let level = if deps.is_empty() {
                0
            } else {
                deps.iter()
                    .map(|dep| levels.get(dep).copied().unwrap_or(0) + 1)
                    .max()
                    .unwrap_or(0)
            };
            levels.insert(node_id, level);
            if level > max_level {
                max_level = level;
            }
        }

        let mut groups: Vec<Vec<usize>> = vec![vec![]; max_level + 1];
        for (&id, &level) in &levels {
            groups[level].push(id);
        }
        // Sort each group for determinism
        for group in &mut groups {
            group.sort_unstable();
        }
        groups
    }

    /// Compute the critical path (longest path by estimated duration).
    /// Returns step IDs in execution order.
    pub fn critical_path(&self) -> Vec<usize> {
        if self.nodes.is_empty() {
            return vec![];
        }

        // DP: dist[node] = max total duration ending at node
        let mut dist: HashMap<usize, u64> = HashMap::new();
        let mut predecessor: HashMap<usize, Option<usize>> = HashMap::new();

        for &node_id in &self.topological_order {
            let node_dur = self
                .nodes
                .get(&node_id)
                .map(|n| n.est_duration_ms)
                .unwrap_or(0);
            let deps = self
                .reverse_edges
                .get(&node_id)
                .cloned()
                .unwrap_or_default();

            if deps.is_empty() {
                dist.insert(node_id, node_dur);
                predecessor.insert(node_id, None);
            } else {
                let (best_pred, best_dist) = deps
                    .iter()
                    .map(|&d| (d, dist.get(&d).copied().unwrap_or(0)))
                    .max_by_key(|&(_, d)| d)
                    .expect("deps is non-empty");
                dist.insert(node_id, best_dist + node_dur);
                predecessor.insert(node_id, Some(best_pred));
            }
        }

        // Find the node with the max distance
        let &end_node = dist
            .iter()
            .max_by_key(|&(_, &d)| d)
            .map(|(id, _)| id)
            .unwrap_or(&0);

        // Trace back
        let mut path = vec![end_node];
        let mut current = end_node;
        while let Some(Some(pred)) = predecessor.get(&current) {
            path.push(*pred);
            current = *pred;
        }
        path.reverse();
        path
    }

    /// Get step IDs that are ready to execute (all deps completed).
    pub fn next_ready(&self) -> Vec<usize> {
        let mut ready = Vec::new();
        for (&id, node) in &self.nodes {
            if node.status != TaskNodeStatus::Pending {
                continue;
            }
            let deps = self.reverse_edges.get(&id).cloned().unwrap_or_default();
            let all_done = deps.iter().all(|dep| {
                self.nodes
                    .get(dep)
                    .map(|n| n.status == TaskNodeStatus::Completed)
                    .unwrap_or(true)
            });
            if all_done {
                ready.push(id);
            }
        }
        ready.sort_unstable();
        ready
    }

    /// Mark a node as running.
    pub fn set_running(&mut self, step_id: usize) {
        if let Some(node) = self.nodes.get_mut(&step_id) {
            node.status = TaskNodeStatus::Running;
        }
    }

    /// Mark a node as completed.
    pub fn complete_node(&mut self, step_id: usize) {
        if let Some(node) = self.nodes.get_mut(&step_id) {
            node.status = TaskNodeStatus::Completed;
        }
    }

    /// Mark a node as failed.
    pub fn fail_node(&mut self, step_id: usize) {
        if let Some(node) = self.nodes.get_mut(&step_id) {
            node.status = TaskNodeStatus::Failed;
        }
    }

    /// Estimated total time along the critical path.
    pub fn estimated_total_ms(&self) -> u64 {
        let path = self.critical_path();
        path.iter()
            .filter_map(|id| self.nodes.get(id))
            .map(|n| n.est_duration_ms)
            .sum()
    }

    /// Check if the DAG execution is finished (all nodes in a terminal state).
    pub fn is_finished(&self) -> bool {
        self.nodes.values().all(|n| {
            matches!(
                n.status,
                TaskNodeStatus::Completed | TaskNodeStatus::Failed | TaskNodeStatus::Skipped
            )
        })
    }
}

// ---------------------------------------------------------------------------
// StrategicPlanner
// ---------------------------------------------------------------------------

/// Wraps the existing Planner output and adds DAG analysis.
pub struct StrategicPlanner {
    /// Default estimated duration per step (ms).
    pub default_est_duration_ms: u64,
}

impl StrategicPlanner {
    pub fn new() -> Self {
        Self {
            default_est_duration_ms: 5000,
        }
    }

    /// Analyze a Plan and produce a TaskDAG.
    pub fn analyze(&self, plan: &Plan) -> Result<TaskDAG, String> {
        TaskDAG::from_plan(plan, self.default_est_duration_ms)
    }
}

impl Default for StrategicPlanner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{Plan, PlanStatus, Step, StepStatus};

    fn make_plan(steps: Vec<(usize, &str, Option<&str>, Vec<usize>)>) -> Plan {
        Plan {
            task: "test task".to_string(),
            steps: steps
                .into_iter()
                .map(|(id, desc, tool, deps)| Step {
                    id,
                    description: desc.to_string(),
                    tool: tool.map(|t| t.to_string()),
                    arguments: None,
                    dependencies: deps,
                    status: StepStatus::Pending,
                    output: None,
                })
                .collect(),
            status: PlanStatus::Created,
        }
    }

    #[test]
    fn test_linear_plan() {
        // A -> B -> C
        let plan = make_plan(vec![
            (1, "Step A", Some("shell"), vec![]),
            (2, "Step B", Some("shell"), vec![1]),
            (3, "Step C", Some("shell"), vec![2]),
        ]);
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();

        assert_eq!(dag.topological_order, vec![1, 2, 3]);
        let groups = dag.parallel_groups();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec![1]);
        assert_eq!(groups[1], vec![2]);
        assert_eq!(groups[2], vec![3]);
    }

    #[test]
    fn test_diamond_dag() {
        //   1
        //  / \
        // 2   3
        //  \ /
        //   4
        let plan = make_plan(vec![
            (1, "Start", None, vec![]),
            (2, "Left", Some("shell"), vec![1]),
            (3, "Right", Some("shell"), vec![1]),
            (4, "Merge", None, vec![2, 3]),
        ]);
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();

        assert_eq!(dag.topological_order[0], 1);
        assert_eq!(dag.topological_order[3], 4);
        // 2 and 3 should be in the middle (order may vary but both before 4)

        let groups = dag.parallel_groups();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec![1]);
        assert_eq!(groups[1], vec![2, 3]); // parallel
        assert_eq!(groups[2], vec![4]);
    }

    #[test]
    fn test_critical_path() {
        // 1 (1000ms) -> 2 (5000ms) -> 4 (1000ms)
        // 1 (1000ms) -> 3 (1000ms) -> 4 (1000ms)
        // Critical path: 1 -> 2 -> 4 = 7000ms
        let plan = make_plan(vec![
            (1, "Start", None, vec![]),
            (2, "Long", Some("shell"), vec![1]),
            (3, "Short", Some("shell"), vec![1]),
            (4, "End", None, vec![2, 3]),
        ]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        dag.nodes.get_mut(&2).unwrap().est_duration_ms = 5000;

        let cp = dag.critical_path();
        assert_eq!(cp, vec![1, 2, 4]);
        assert_eq!(dag.estimated_total_ms(), 7000);
    }

    #[test]
    fn test_cycle_detection() {
        // 1 -> 2 -> 1 (cycle)
        let plan = make_plan(vec![(1, "A", None, vec![2]), (2, "B", None, vec![1])]);
        let err = TaskDAG::from_plan(&plan, 1000).unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn test_next_ready_initial() {
        let plan = make_plan(vec![
            (1, "A", None, vec![]),
            (2, "B", None, vec![]),
            (3, "C", None, vec![1, 2]),
        ]);
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        assert_eq!(dag.next_ready(), vec![1, 2]);
    }

    #[test]
    fn test_next_ready_after_completion() {
        let plan = make_plan(vec![
            (1, "A", None, vec![]),
            (2, "B", None, vec![1]),
            (3, "C", None, vec![1]),
        ]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();

        // Initially only step 1 is ready
        assert_eq!(dag.next_ready(), vec![1]);

        // Complete step 1 -> 2 and 3 become ready
        dag.complete_node(1);
        let mut ready = dag.next_ready();
        ready.sort();
        assert_eq!(ready, vec![2, 3]);
    }

    #[test]
    fn test_is_finished() {
        let plan = make_plan(vec![(1, "A", None, vec![]), (2, "B", None, vec![1])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        assert!(!dag.is_finished());

        dag.complete_node(1);
        assert!(!dag.is_finished());

        dag.complete_node(2);
        assert!(dag.is_finished());
    }

    #[test]
    fn test_is_finished_with_failures() {
        let plan = make_plan(vec![(1, "A", None, vec![]), (2, "B", None, vec![1])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        dag.complete_node(1);
        dag.fail_node(2);
        assert!(dag.is_finished());
    }

    #[test]
    fn test_empty_plan() {
        let plan = Plan {
            task: "empty".to_string(),
            steps: vec![],
            status: PlanStatus::Created,
        };
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        assert!(dag.nodes.is_empty());
        assert!(dag.topological_order.is_empty());
        assert!(dag.parallel_groups().is_empty());
        assert!(dag.critical_path().is_empty());
        assert_eq!(dag.estimated_total_ms(), 0);
        assert!(dag.is_finished()); // vacuously true
    }

    #[test]
    fn test_single_step() {
        let plan = make_plan(vec![(1, "Only step", Some("shell"), vec![])]);
        let dag = TaskDAG::from_plan(&plan, 2000).unwrap();
        assert_eq!(dag.topological_order, vec![1]);
        assert_eq!(dag.parallel_groups(), vec![vec![1]]);
        assert_eq!(dag.critical_path(), vec![1]);
        assert_eq!(dag.estimated_total_ms(), 2000);
        assert_eq!(dag.next_ready(), vec![1]);
    }

    #[test]
    fn test_all_independent() {
        // No dependencies - all steps can run in parallel
        let plan = make_plan(vec![
            (1, "A", None, vec![]),
            (2, "B", None, vec![]),
            (3, "C", None, vec![]),
        ]);
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();

        let groups = dag.parallel_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![1, 2, 3]);
    }

    #[test]
    fn test_strategic_planner_analyze() {
        let sp = StrategicPlanner::new();
        assert_eq!(sp.default_est_duration_ms, 5000);

        let plan = make_plan(vec![(1, "A", None, vec![]), (2, "B", None, vec![1])]);
        let dag = sp.analyze(&plan).unwrap();
        assert_eq!(dag.nodes.len(), 2);
        assert_eq!(dag.nodes.get(&1).unwrap().est_duration_ms, 5000);
    }

    #[test]
    fn test_task_node_status_serialization() {
        let statuses = vec![
            TaskNodeStatus::Pending,
            TaskNodeStatus::Running,
            TaskNodeStatus::Completed,
            TaskNodeStatus::Failed,
            TaskNodeStatus::Skipped,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let de: TaskNodeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(de, status);
        }
    }

    #[test]
    fn test_dag_serialization() {
        let plan = make_plan(vec![
            (1, "A", Some("shell"), vec![]),
            (2, "B", None, vec![1]),
        ]);
        let dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        let json = serde_json::to_string(&dag).unwrap();
        let de: TaskDAG = serde_json::from_str(&json).unwrap();
        assert_eq!(de.nodes.len(), 2);
        assert_eq!(de.topological_order, vec![1, 2]);
    }

    #[test]
    fn test_fail_node() {
        let plan = make_plan(vec![(1, "A", None, vec![])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        dag.fail_node(1);
        assert_eq!(dag.nodes.get(&1).unwrap().status, TaskNodeStatus::Failed);
    }

    #[test]
    fn test_complete_node() {
        let plan = make_plan(vec![(1, "A", None, vec![])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        dag.complete_node(1);
        assert_eq!(dag.nodes.get(&1).unwrap().status, TaskNodeStatus::Completed);
    }

    #[test]
    fn test_set_running() {
        let plan = make_plan(vec![(1, "A", None, vec![])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        assert_eq!(dag.nodes.get(&1).unwrap().status, TaskNodeStatus::Pending);
        dag.set_running(1);
        assert_eq!(dag.nodes.get(&1).unwrap().status, TaskNodeStatus::Running);
    }

    #[test]
    fn test_set_running_nonexistent_node() {
        let plan = make_plan(vec![(1, "A", None, vec![])]);
        let mut dag = TaskDAG::from_plan(&plan, 1000).unwrap();
        // Should not panic on missing node
        dag.set_running(999);
        assert_eq!(dag.nodes.get(&1).unwrap().status, TaskNodeStatus::Pending);
    }
}
