//! Planner - LLM-backed task decomposition into executable steps

use crate::goals::Goal;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use zeus_core::{Message, Result, ToolSchema};
use zeus_llm::LlmClient;
use zeus_templates::TemplateRegistry;

/// Task planner that uses LLM to decompose tasks into steps
pub struct Planner {
    max_steps: usize,
    /// Optional template registry for auto-enriching tasks before planning.
    /// When set, `create_plan` searches for a matching outcome template by
    /// keyword/tag and, if a confident match is found, prepends the template's
    /// system prompt + success criteria to the task before LLM decomposition.
    templates: Option<TemplateRegistry>,
}

impl Planner {
    /// Create a new planner
    pub fn new() -> Self {
        Self { max_steps: 10, templates: None }
    }

    /// Attach a template registry for automatic goal enrichment.
    /// When a task passes through `create_plan`, the planner will try to
    /// match it against the loaded templates and enrich the prompt with
    /// the matched template's system prompt + success criteria.
    pub fn with_templates(mut self, registry: TemplateRegistry) -> Self {
        self.templates = Some(registry);
        self
    }

    /// Attempt to enrich a raw task string using the template registry.
    /// Returns `Some(enriched)` when a confident match is found, `None` otherwise.
    ///
    /// Match criterion: the top keyword-search hit must share at least one
    /// tag or category whose word appears in the task (case-insensitive).
    /// This avoids incidentally enriching tasks that only loosely hit a
    /// template name.
    fn try_enrich(&self, task: &str) -> Option<String> {
        let registry = self.templates.as_ref()?;
        let task_lc = task.to_lowercase();

        // Prefer the highest-signal hit by trying longer tokens first.
        let tokens: Vec<&str> = task_lc
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 4)
            .collect();

        for token in &tokens {
            let hits = registry.search(token);
            if let Some(t) = hits.into_iter().next() {
                // Confidence check: at least one tag/category must literally
                // appear in the task string.
                let grounded = t
                    .tags
                    .iter()
                    .chain(t.categories.iter())
                    .any(|term| task_lc.contains(&term.to_lowercase()));
                if grounded {
                    let applied = registry.apply(&t, task, &[]);
                    info!(
                        template_id = %t.id,
                        missing_providers = ?applied.missing_providers,
                        "Planner enriched task via template"
                    );
                    return Some(applied.enriched_prompt);
                }
            }
        }
        None
    }

    /// Create a plan for a task using LLM decomposition
    pub async fn create_plan(
        &self,
        task: &str,
        llm: &LlmClient,
        tools: &[ToolSchema],
    ) -> Result<Plan> {
        let tool_list = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");

        let system = format!(
            "You are a task planner. Decompose user tasks into concrete steps.\n\
             Each step should be a single action. You have these tools:\n{}\n\n\
             Respond ONLY with valid JSON matching this schema:\n\
             {{\n  \"steps\": [\n    {{\n      \"description\": \"what to do\",\n      \
             \"tool\": \"tool_name or null\",\n      \
             \"arguments\": {{}} or null,\n      \
             \"depends_on\": []\n    }}\n  ]\n}}\n\n\
             Rules:\n\
             - Maximum {} steps\n\
             - depends_on contains 0-indexed step numbers this step requires\n\
             - tool is null if the step is reasoning/analysis, otherwise the tool name\n\
             - arguments is the JSON object to pass to the tool (null if no tool or if args depend on previous step output)\n\
             - Keep steps concrete and actionable\n\
             - If the task is simple, use 1-2 steps",
            tool_list, self.max_steps
        );

        // Try template enrichment: if a matching outcome template is found,
        // use its enriched prompt so the LLM sees the template's system prompt
        // + success criteria alongside the user's task. Falls through on miss.
        let enriched = self.try_enrich(task);
        let task_for_llm: &str = enriched.as_deref().unwrap_or(task);

        let messages = vec![Message::user(format!("Plan this task: {}", task_for_llm))];

        let response = llm.complete(&messages, &[], Some(&system)).await?;

        // Keep the original task in the Plan struct so higher layers still see
        // what the user asked for — the enrichment is an LLM-side hint only.
        self.parse_plan_response(task, &response.content)
    }

    /// Parse the LLM's JSON response into a Plan
    fn parse_plan_response(&self, task: &str, response: &str) -> Result<Plan> {
        // Extract JSON from the response (handles markdown code blocks)
        let json_str = extract_json(response);

        match serde_json::from_str::<PlanResponse>(&json_str) {
            Ok(parsed) => {
                let steps: Vec<Step> = parsed
                    .steps
                    .into_iter()
                    .take(self.max_steps)
                    .enumerate()
                    .map(|(i, s)| Step {
                        id: i + 1,
                        description: s.description,
                        tool: s.tool,
                        arguments: s.arguments,
                        dependencies: s.depends_on.into_iter().map(|d| d + 1).collect(),
                        status: StepStatus::Pending,
                        output: None,
                    })
                    .collect();

                debug!("Created plan with {} steps for: {}", steps.len(), task);

                Ok(Plan {
                    task: task.to_string(),
                    steps,
                    status: PlanStatus::Created,
                })
            }
            Err(e) => {
                warn!(
                    "Failed to parse plan JSON ({}), creating single-step plan",
                    e
                );
                // Fallback: treat the whole task as a single step
                Ok(Plan {
                    task: task.to_string(),
                    steps: vec![Step {
                        id: 1,
                        description: task.to_string(),
                        tool: None,
                        arguments: None,
                        dependencies: vec![],
                        status: StepStatus::Pending,
                        output: None,
                    }],
                    status: PlanStatus::Created,
                })
            }
        }
    }

    /// Create a plan for a specific goal, using the goal's description and
    /// success criteria to guide the LLM's decomposition.
    pub async fn create_plan_for_goal(
        &self,
        goal: &Goal,
        llm: &LlmClient,
        tools: &[ToolSchema],
    ) -> Result<Plan> {
        let criteria = if goal.success_criteria.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nSuccess criteria:\n{}",
                goal.success_criteria
                    .iter()
                    .map(|c| format!("- {}", c))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let context = if goal.context.is_empty() {
            String::new()
        } else {
            format!("\n\nAdditional context: {}", goal.context)
        };

        let enriched_task = format!(
            "[Goal: {}] {}{}{}",
            goal.id, goal.description, criteria, context
        );

        self.create_plan(&enriched_task, llm, tools).await
    }

    /// Set maximum steps
    pub fn with_max_steps(mut self, max: usize) -> Self {
        self.max_steps = max;
        self
    }

    /// Adaptive replanning: re-plan remaining steps after a failure.
    ///
    /// Takes the original plan, the completed step results (success and failure),
    /// and asks the LLM to produce a new plan for achieving the original task
    /// given what has already happened.
    pub async fn replan_remaining(
        &self,
        original_plan: &Plan,
        step_results: &[crate::executor::StepResult],
        llm: &LlmClient,
        tools: &[ToolSchema],
    ) -> Result<Plan> {
        let tool_list = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");

        // Build summary of what happened so far
        let history = step_results
            .iter()
            .map(|r| {
                let status = if r.success { "SUCCESS" } else { "FAILED" };
                let error_info = r
                    .error
                    .as_ref()
                    .map(|e| format!(" ({})", e))
                    .unwrap_or_default();
                let output_preview: String = r.output.chars().take(200).collect();
                format!(
                    "Step {}: {} {}{}\n  Output: {}",
                    r.step_id,
                    status,
                    error_info,
                    "",
                    if output_preview.is_empty() {
                        "(none)".to_string()
                    } else {
                        output_preview
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = format!(
            "You are a task planner performing adaptive replanning.\n\
             The original task was: {}\n\n\
             Here is what happened so far:\n{}\n\n\
             Some steps failed. Create a NEW plan for the REMAINING work needed to \
             complete the original task, taking into account what succeeded and what failed.\n\n\
             Available tools:\n{}\n\n\
             Respond ONLY with valid JSON matching this schema:\n\
             {{\n  \"steps\": [\n    {{\n      \"description\": \"what to do\",\n      \
             \"tool\": \"tool_name or null\",\n      \
             \"arguments\": {{}} or null,\n      \
             \"depends_on\": []\n    }}\n  ]\n}}\n\n\
             Rules:\n\
             - Maximum {} steps\n\
             - depends_on contains 0-indexed step numbers within THIS new plan\n\
             - Do NOT repeat steps that already succeeded\n\
             - If the failure was due to a fixable issue, plan a corrective step first\n\
             - Keep steps concrete and actionable",
            original_plan.task, history, tool_list, self.max_steps
        );

        let messages = vec![Message::user(format!(
            "Replan the remaining work for: {}",
            original_plan.task
        ))];

        let response = llm.complete(&messages, &[], Some(&system)).await?;

        let mut new_plan = self.parse_plan_response(&original_plan.task, &response.content)?;
        // Mark as replanned by prefixing task
        new_plan.task = format!("[Replan] {}", original_plan.task);
        Ok(new_plan)
    }
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract JSON from a response that may contain markdown code blocks
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();
    // Try to extract from ```json ... ``` blocks
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        // Skip optional language tag on the same line
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    // Try finding { ... } directly
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
    {
        return trimmed[start..=end].to_string();
    }
    trimmed.to_string()
}

/// Intermediate type for parsing LLM response
#[derive(Deserialize)]
struct PlanResponse {
    steps: Vec<PlanStep>,
}

#[derive(Deserialize)]
struct PlanStep {
    description: String,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
    #[serde(default)]
    depends_on: Vec<usize>,
}

/// A plan for executing a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Original task description
    pub task: String,
    /// Steps to execute
    pub steps: Vec<Step>,
    /// Overall plan status
    pub status: PlanStatus,
}

/// A step in a plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step ID (1-indexed)
    pub id: usize,
    /// What this step does
    pub description: String,
    /// Tool to use (if any)
    pub tool: Option<String>,
    /// Suggested arguments for the tool (LLM-generated, may be refined at execution)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    /// IDs of steps this depends on
    pub dependencies: Vec<usize>,
    /// Step status
    pub status: StepStatus,
    /// Output from execution (populated by executor)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Status of a plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Created,
    InProgress,
    Completed,
    Failed,
}

/// Status of a step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let input = "Here's the plan:\n```json\n{\"steps\": [{\"description\": \"do it\"}]}\n```";
        let json = extract_json(input);
        assert!(json.contains("steps"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["steps"].is_array());
    }

    #[test]
    fn test_extract_json_raw() {
        let input =
            "{\"steps\": [{\"description\": \"do it\", \"tool\": null, \"depends_on\": []}]}";
        let json = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["steps"].is_array());
    }

    #[test]
    fn test_parse_plan_fallback() {
        let planner = Planner::new();
        let result = planner.parse_plan_response("test task", "not valid json");
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, "test task");
    }

    #[test]
    fn test_parse_plan_valid() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Read the file", "tool": "read_file", "depends_on": []},
            {"description": "Analyze content", "tool": null, "depends_on": [0]},
            {"description": "Write summary", "tool": "write_file", "depends_on": [1]}
        ]}"#;
        let result = planner.parse_plan_response("summarize file", json);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].tool.as_deref(), Some("read_file"));
        assert!(plan.steps[1].tool.is_none());
        assert_eq!(plan.steps[2].dependencies, vec![2]); // 1-indexed
    }

    #[test]
    fn test_max_steps_limit() {
        let planner = Planner::new().with_max_steps(2);
        let json = r#"{"steps": [
            {"description": "Step 1"},
            {"description": "Step 2"},
            {"description": "Step 3"}
        ]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn test_planner_defaults() {
        let planner = Planner::new();
        assert_eq!(planner.max_steps, 10);
    }

    #[test]
    fn test_planner_default_trait() {
        let planner = Planner::default();
        assert_eq!(planner.max_steps, 10);
    }

    #[test]
    fn test_with_max_steps() {
        let planner = Planner::new().with_max_steps(5);
        assert_eq!(planner.max_steps, 5);
    }

    #[test]
    fn test_parse_plan_dependencies_remapped_to_1_indexed() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Step A", "depends_on": []},
            {"description": "Step B", "depends_on": [0]}
        ]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.steps[0].id, 1);
        assert_eq!(plan.steps[0].dependencies, Vec::<usize>::new());
        assert_eq!(plan.steps[1].id, 2);
        assert_eq!(plan.steps[1].dependencies, vec![1]); // 0 + 1 = 1
    }

    #[test]
    fn test_parse_plan_status_is_created() {
        let planner = Planner::new();
        let json = r#"{"steps": [{"description": "do it"}]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.status, PlanStatus::Created);
    }

    #[test]
    fn test_parse_plan_task_name_preserved() {
        let planner = Planner::new();
        let json = r#"{"steps": [{"description": "do it"}]}"#;
        let plan = planner
            .parse_plan_response("my important task", json)
            .unwrap();
        assert_eq!(plan.task, "my important task");
    }

    #[test]
    fn test_extract_json_from_generic_code_block() {
        let input = "```\n{\"steps\": [{\"description\": \"test\"}]}\n```";
        let json = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["steps"].is_array());
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let input = "Here is the plan: {\"steps\": []} and that's it.";
        let json = extract_json(input);
        assert_eq!(json, "{\"steps\": []}");
    }

    #[test]
    fn test_extract_json_plain_text_no_json() {
        let input = "This has no JSON at all";
        let json = extract_json(input);
        assert_eq!(json, "This has no JSON at all");
    }

    #[test]
    fn test_extract_json_empty_string() {
        let json = extract_json("");
        assert_eq!(json, "");
    }

    #[test]
    fn test_extract_json_whitespace_only() {
        let json = extract_json("   \n  \t  ");
        assert_eq!(json, "");
    }

    #[test]
    fn test_parse_plan_empty_steps() {
        let planner = Planner::new();
        let json = r#"{"steps": []}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn test_parse_plan_step_with_tool() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Run tests", "tool": "shell", "depends_on": []}
        ]}"#;
        let plan = planner
            .parse_plan_response("run tests", json)
            .expect("Failed to parse plan response with tool");
        assert_eq!(plan.steps[0].tool.as_deref(), Some("shell"));
        assert_eq!(plan.steps[0].status, StepStatus::Pending);
    }

    #[test]
    fn test_parse_plan_step_without_tool() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Think about approach"}
        ]}"#;
        let plan = planner.parse_plan_response("plan", json).unwrap();
        assert!(plan.steps[0].tool.is_none());
    }

    #[test]
    fn test_plan_status_serialization() {
        let statuses = vec![
            PlanStatus::Created,
            PlanStatus::InProgress,
            PlanStatus::Completed,
            PlanStatus::Failed,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deser: PlanStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deser, status);
        }
    }

    #[test]
    fn test_step_status_serialization() {
        let statuses = vec![
            StepStatus::Pending,
            StepStatus::InProgress,
            StepStatus::Completed,
            StepStatus::Failed,
            StepStatus::Skipped,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deser: StepStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deser, status);
        }
    }

    #[test]
    fn test_plan_serialization_roundtrip() {
        let plan = Plan {
            task: "deploy app".to_string(),
            steps: vec![
                Step {
                    id: 1,
                    description: "Build".to_string(),
                    tool: Some("shell".to_string()),
                    dependencies: vec![],
                    status: StepStatus::Completed,
                    arguments: None,
                    output: None,
                },
                Step {
                    id: 2,
                    description: "Deploy".to_string(),
                    tool: Some("shell".to_string()),
                    arguments: None,
                    dependencies: vec![1],
                    status: StepStatus::Pending,
                    output: None,
                },
            ],
            status: PlanStatus::InProgress,
        };
        let json = serde_json::to_string(&plan).unwrap();
        let deser: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.task, "deploy app");
        assert_eq!(deser.steps.len(), 2);
        assert_eq!(deser.steps[0].status, StepStatus::Completed);
        assert_eq!(deser.steps[1].dependencies, vec![1]);
    }

    #[test]
    fn test_max_steps_one() {
        let planner = Planner::new().with_max_steps(1);
        let json = r#"{"steps": [
            {"description": "Step 1"},
            {"description": "Step 2"}
        ]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, "Step 1");
    }

    #[test]
    fn test_parse_plan_with_many_dependencies() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Step A", "depends_on": []},
            {"description": "Step B", "depends_on": []},
            {"description": "Step C", "depends_on": []},
            {"description": "Step D", "depends_on": [0, 1, 2]}
        ]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.steps.len(), 4);
        // depends_on [0,1,2] remapped to 1-indexed: [1,2,3]
        assert_eq!(plan.steps[3].dependencies, vec![1, 2, 3]);
    }

    #[test]
    fn test_parse_plan_description_field() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Read the configuration file and parse it", "tool": "read_file", "depends_on": []}
        ]}"#;
        let plan = planner.parse_plan_response("parse config", json).unwrap();
        assert_eq!(
            plan.steps[0].description,
            "Read the configuration file and parse it"
        );
    }

    #[test]
    fn test_parse_plan_single_step() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "Do the one thing", "tool": "shell", "depends_on": []}
        ]}"#;
        let plan = planner.parse_plan_response("single task", json).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].id, 1);
        assert_eq!(plan.steps[0].tool.as_deref(), Some("shell"));
        assert!(plan.steps[0].dependencies.is_empty());
        assert_eq!(plan.steps[0].status, StepStatus::Pending);
    }

    #[test]
    fn test_extract_json_nested_code_blocks() {
        // JSON inside a nested markdown context with extra text
        let input = "Sure, here is the plan:\n\n```json\n{\"steps\": [{\"description\": \"nested test\"}]}\n```\n\nHope that helps!";
        let json = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["steps"].is_array());
        assert_eq!(parsed["steps"][0]["description"], "nested test");
    }

    #[test]
    fn test_extract_json_multiple_json_blocks() {
        // Two code blocks; should extract the first valid JSON
        let input = "```json\n{\"steps\": [{\"description\": \"first\"}]}\n```\nAnd also:\n```json\n{\"steps\": [{\"description\": \"second\"}]}\n```";
        let json = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["steps"][0]["description"], "first");
    }

    #[test]
    fn test_parse_plan_preserves_tool_params() {
        let planner = Planner::new();
        let json = r#"{"steps": [
            {"description": "List files", "tool": "list_dir", "depends_on": []},
            {"description": "Read config", "tool": "read_file", "depends_on": [0]},
            {"description": "Analyze", "tool": null, "depends_on": [1]}
        ]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.steps[0].tool.as_deref(), Some("list_dir"));
        assert_eq!(plan.steps[1].tool.as_deref(), Some("read_file"));
        assert!(plan.steps[2].tool.is_none());
    }

    #[test]
    fn test_max_steps_truncation() {
        let planner = Planner::new().with_max_steps(10);
        // Create 11 steps
        let steps: Vec<String> = (0..11)
            .map(|i| format!(r#"{{"description": "Step {}"}}"#, i))
            .collect();
        let json = format!(r#"{{"steps": [{}]}}"#, steps.join(","));
        let plan = planner.parse_plan_response("task", &json).unwrap();
        assert_eq!(plan.steps.len(), 10);
        // Verify the 11th step was dropped
        assert_eq!(plan.steps[9].description, "Step 9");
    }

    #[test]
    fn test_plan_default_status() {
        let planner = Planner::new();
        let json = r#"{"steps": [{"description": "a"}, {"description": "b"}]}"#;
        let plan = planner.parse_plan_response("task", json).unwrap();
        assert_eq!(plan.status, PlanStatus::Created);
        // Also verify all steps are Pending
        for step in &plan.steps {
            assert_eq!(step.status, StepStatus::Pending);
        }
    }
}
