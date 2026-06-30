//! Cooking Plans - YAML Task Plan Executor
//!
//! Parses and executes YAML-based task plans with dependency management.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use zeus_core::Result;

/// Task plan containing multiple tasks with dependencies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Plan name/title
    pub name: String,
    /// Description of the plan
    #[serde(default)]
    pub description: String,
    /// List of tasks in the plan
    pub tasks: Vec<TaskDef>,
}

/// Individual task definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    /// Unique task identifier
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Tab/session to execute in (optional)
    #[serde(default)]
    pub tab: Option<String>,
    /// Task IDs this task depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Command or action to execute
    #[serde(default)]
    pub action: String,
}

/// Task execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Plan execution status
#[derive(Debug, Clone)]
pub struct ExecutionStatus {
    pub completed: HashSet<String>,
    pub failed: HashSet<String>,
    pub pending: HashSet<String>,
}

/// Executor for task plans
pub struct PlanExecutor {
    plan: TaskPlan,
    results: HashMap<String, TaskResult>,
}

impl PlanExecutor {
    /// Create a new plan executor
    pub fn new(plan: TaskPlan) -> Result<Self> {
        // Validate plan
        Self::validate_plan(&plan)?;

        Ok(Self {
            plan,
            results: HashMap::new(),
        })
    }

    /// Parse a task plan from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let plan: TaskPlan = serde_yaml::from_str(yaml)
            .map_err(|e| zeus_core::Error::Agent(format!("Failed to parse YAML: {}", e)))?;

        Self::new(plan)
    }

    /// Validate plan structure and dependencies
    fn validate_plan(plan: &TaskPlan) -> Result<()> {
        if plan.tasks.is_empty() {
            return Err(zeus_core::Error::Agent("Plan has no tasks".to_string()));
        }

        let task_ids: HashSet<_> = plan.tasks.iter().map(|t| &t.id).collect();

        // Check for duplicate task IDs
        if task_ids.len() != plan.tasks.len() {
            return Err(zeus_core::Error::Agent(
                "Duplicate task IDs found".to_string(),
            ));
        }

        // Validate dependencies exist
        for task in &plan.tasks {
            for dep in &task.depends_on {
                if !task_ids.contains(&dep) {
                    return Err(zeus_core::Error::Agent(format!(
                        "Task '{}' depends on non-existent task '{}'",
                        task.id, dep
                    )));
                }
            }
        }

        // Check for circular dependencies
        Self::check_cycles(plan)?;

        Ok(())
    }

    /// Check for circular dependencies using DFS
    fn check_cycles(plan: &TaskPlan) -> Result<()> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for task in &plan.tasks {
            if !visited.contains(&task.id)
                && Self::has_cycle(task, plan, &mut visited, &mut rec_stack)?
            {
                return Err(zeus_core::Error::Agent(
                    "Circular dependency detected".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn has_cycle(
        task: &TaskDef,
        plan: &TaskPlan,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> Result<bool> {
        visited.insert(task.id.clone());
        rec_stack.insert(task.id.clone());

        for dep_id in &task.depends_on {
            if !visited.contains(dep_id) {
                let dep_task =
                    plan.tasks.iter().find(|t| &t.id == dep_id).ok_or_else(|| {
                        zeus_core::Error::Agent("Dependency not found".to_string())
                    })?;

                if Self::has_cycle(dep_task, plan, visited, rec_stack)? {
                    return Ok(true);
                }
            } else if rec_stack.contains(dep_id) {
                return Ok(true);
            }
        }

        rec_stack.remove(&task.id);
        Ok(false)
    }

    /// Get tasks ready to execute (dependencies satisfied)
    pub fn get_ready_tasks(&self) -> Vec<&TaskDef> {
        let completed: HashSet<_> = self.results.keys().cloned().collect();

        self.plan
            .tasks
            .iter()
            .filter(|task| {
                // Not already executed
                !completed.contains(&task.id)
                    // All dependencies are completed
                    && task.depends_on.iter().all(|dep| completed.contains(dep))
            })
            .collect()
    }

    /// Get execution status
    pub fn get_status(&self) -> ExecutionStatus {
        let completed: HashSet<_> = self
            .results
            .iter()
            .filter(|(_, r)| r.success)
            .map(|(id, _)| id.clone())
            .collect();

        let failed: HashSet<_> = self
            .results
            .iter()
            .filter(|(_, r)| !r.success)
            .map(|(id, _)| id.clone())
            .collect();

        let all_ids: HashSet<_> = self.plan.tasks.iter().map(|t| t.id.clone()).collect();

        let pending: HashSet<_> = all_ids
            .difference(&completed)
            .filter(|id| !failed.contains(*id))
            .cloned()
            .collect();

        ExecutionStatus {
            completed,
            failed,
            pending,
        }
    }

    /// Record task result
    pub fn record_result(&mut self, result: TaskResult) {
        self.results.insert(result.task_id.clone(), result);
    }

    /// Get task by ID
    pub fn get_task(&self, task_id: &str) -> Option<&TaskDef> {
        self.plan.tasks.iter().find(|t| t.id == task_id)
    }

    /// Check if plan execution is complete
    pub fn is_complete(&self) -> bool {
        self.results.len() == self.plan.tasks.len()
    }

    /// Get execution order (topological sort)
    pub fn get_execution_order(&self) -> Result<Vec<String>> {
        let mut order = Vec::new();
        let mut visited = HashSet::new();

        for task in &self.plan.tasks {
            self.visit_task(task, &mut visited, &mut order)?;
        }

        Ok(order)
    }

    fn visit_task(
        &self,
        task: &TaskDef,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(&task.id) {
            return Ok(());
        }

        // Visit dependencies first
        for dep_id in &task.depends_on {
            let dep_task = self
                .get_task(dep_id)
                .ok_or_else(|| zeus_core::Error::Agent("Dependency not found".to_string()))?;
            self.visit_task(dep_task, visited, order)?;
        }

        visited.insert(task.id.clone());
        order.push(task.id.clone());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_plan() -> TaskPlan {
        TaskPlan {
            name: "Test Plan".to_string(),
            description: "A test plan".to_string(),
            tasks: vec![
                TaskDef {
                    id: "task1".to_string(),
                    description: "First task".to_string(),
                    tab: None,
                    depends_on: vec![],
                    action: "echo hello".to_string(),
                },
                TaskDef {
                    id: "task2".to_string(),
                    description: "Second task".to_string(),
                    tab: Some("main".to_string()),
                    depends_on: vec!["task1".to_string()],
                    action: "echo world".to_string(),
                },
            ],
        }
    }

    #[test]
    fn test_plan_creation() {
        let plan = create_test_plan();
        let executor = PlanExecutor::new(plan);
        assert!(executor.is_ok());
    }

    #[test]
    fn test_invalid_plan_empty_tasks() {
        let plan = TaskPlan {
            name: "Empty".to_string(),
            description: "".to_string(),
            tasks: vec![],
        };
        let result = PlanExecutor::new(plan);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_plan_missing_dependency() {
        let plan = TaskPlan {
            name: "Bad deps".to_string(),
            description: "".to_string(),
            tasks: vec![TaskDef {
                id: "task1".to_string(),
                description: "Task".to_string(),
                tab: None,
                depends_on: vec!["nonexistent".to_string()],
                action: "echo test".to_string(),
            }],
        };
        let result = PlanExecutor::new(plan);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_ready_tasks() {
        let plan = create_test_plan();
        let executor = PlanExecutor::new(plan).unwrap();

        let ready = executor.get_ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "task1");
    }

    #[test]
    fn test_record_result() {
        let plan = create_test_plan();
        let mut executor = PlanExecutor::new(plan).unwrap();

        executor.record_result(TaskResult {
            task_id: "task1".to_string(),
            success: true,
            output: "done".to_string(),
            error: None,
        });

        let ready = executor.get_ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "task2");
    }

    #[test]
    fn test_execution_status() {
        let plan = create_test_plan();
        let mut executor = PlanExecutor::new(plan).unwrap();

        executor.record_result(TaskResult {
            task_id: "task1".to_string(),
            success: true,
            output: "done".to_string(),
            error: None,
        });

        let status = executor.get_status();
        assert_eq!(status.completed.len(), 1);
        assert_eq!(status.pending.len(), 1);
        assert_eq!(status.failed.len(), 0);
    }

    #[test]
    fn test_yaml_parsing() {
        let yaml = r#"
name: Test Plan
description: A simple test
tasks:
  - id: task1
    description: First task
    action: echo hello
  - id: task2
    description: Second task
    depends_on:
      - task1
    action: echo world
"#;
        let executor = PlanExecutor::from_yaml(yaml);
        assert!(executor.is_ok());
    }

    #[test]
    fn test_execution_order() {
        let plan = create_test_plan();
        let executor = PlanExecutor::new(plan).unwrap();

        let order = executor.get_execution_order().unwrap();
        assert_eq!(order, vec!["task1", "task2"]);
    }
}
