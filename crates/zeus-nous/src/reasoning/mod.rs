//! Reasoning Engine - Chain-of-thought problem solving
//!
//! Enables Zeus to:
//! - Break down complex problems into steps
//! - Generate and evaluate solution approaches
//! - Create action plans with dependencies
//! - Adapt reasoning based on context

use crate::UserContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use zeus_core::{Message, Result};

/// A chain of thoughts leading to a conclusion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtChain {
    /// Unique identifier
    pub id: String,
    /// The problem being solved
    pub problem: String,
    /// Sequence of reasoning steps
    pub steps: Vec<Step>,
    /// Final conclusion
    pub conclusion: Option<String>,
    /// Whether reasoning was successful
    pub success: bool,
    /// Confidence in the solution
    pub confidence: f32,
    /// Alternative approaches considered
    pub alternatives: Vec<Alternative>,
    /// When reasoning started
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Total thinking time in milliseconds
    pub thinking_time_ms: u64,
}

impl ThoughtChain {
    /// Create a new thought chain for a problem
    pub fn new(problem: &str) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            problem: problem.to_string(),
            steps: Vec::new(),
            conclusion: None,
            success: false,
            confidence: 0.0,
            alternatives: Vec::new(),
            started_at: chrono::Utc::now(),
            thinking_time_ms: 0,
        }
    }

    /// Add a reasoning step
    pub fn add_step(&mut self, step: Step) {
        self.steps.push(step);
    }

    /// Complete the chain with a conclusion
    pub fn conclude(&mut self, conclusion: &str, success: bool, confidence: f32) {
        self.conclusion = Some(conclusion.to_string());
        self.success = success;
        self.confidence = confidence;
        self.thinking_time_ms = (chrono::Utc::now() - self.started_at).num_milliseconds() as u64;
    }

    /// Get the action plan from this reasoning
    pub fn to_action_plan(&self) -> Vec<Action> {
        self.steps
            .iter()
            .filter_map(|step| {
                if let StepType::Action(action) = &step.step_type {
                    Some(action.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// A single reasoning step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step number
    pub number: usize,
    /// Type of step
    pub step_type: StepType,
    /// The thought/reasoning
    pub thought: String,
    /// Outcome of this step
    pub outcome: Option<String>,
    /// Confidence at this step
    pub confidence: f32,
}

impl Step {
    /// Create a new reasoning step
    pub fn new(number: usize, step_type: StepType, thought: &str) -> Self {
        Self {
            number,
            step_type,
            thought: thought.to_string(),
            outcome: None,
            confidence: 0.5,
        }
    }
}

/// Types of reasoning steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepType {
    /// Analyzing the problem
    Analysis,
    /// Breaking down into sub-problems
    Decomposition,
    /// Gathering required information
    InformationGathering,
    /// Evaluating options
    Evaluation,
    /// Making a decision
    Decision,
    /// Planning an action
    Action(Action),
    /// Checking constraints
    ConstraintCheck,
    /// Synthesizing information
    Synthesis,
    /// Verifying a conclusion
    Verification,
}

/// A planned action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Action identifier
    pub id: String,
    /// Description of the action
    pub description: String,
    /// Tool to use (if any)
    pub tool: Option<String>,
    /// Parameters for the tool
    pub parameters: serde_json::Value,
    /// Dependencies (action IDs that must complete first)
    pub dependencies: Vec<String>,
    /// Priority (higher = more important)
    pub priority: u8,
    /// Estimated complexity (1-10)
    pub complexity: u8,
    /// Can this be done in parallel with others?
    pub parallelizable: bool,
}

impl Action {
    /// Create a new action
    pub fn new(description: &str) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            description: description.to_string(),
            tool: None,
            parameters: serde_json::json!({}),
            dependencies: Vec::new(),
            priority: 5,
            complexity: 5,
            parallelizable: true,
        }
    }

    /// Set the tool for this action
    pub fn with_tool(mut self, tool: &str, params: serde_json::Value) -> Self {
        self.tool = Some(tool.to_string());
        self.parameters = params;
        self
    }

    /// Add a dependency
    pub fn depends_on(mut self, action_id: &str) -> Self {
        self.dependencies.push(action_id.to_string());
        self
    }
}

/// An alternative approach that was considered
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alternative {
    /// Description of the alternative
    pub description: String,
    /// Why it was not chosen
    pub rejection_reason: String,
    /// Estimated success probability
    pub estimated_success: f32,
}

/// The Reasoning Engine
pub struct ReasoningEngine {
    user_context: Arc<RwLock<UserContext>>,
    /// Optional LLM client — when present, `think()` uses LLM-backed reasoning.
    /// Falls back to keyword heuristics when absent or when the LLM call fails.
    llm: Option<Arc<zeus_llm::LlmClient>>,
}

impl ReasoningEngine {
    /// Create a new reasoning engine (keyword-heuristic fallback only).
    pub fn new(user_context: Arc<RwLock<UserContext>>) -> Self {
        Self {
            user_context,
            llm: None,
        }
    }

    /// Create a reasoning engine backed by an LLM.
    pub fn with_llm(user_context: Arc<RwLock<UserContext>>, llm: Arc<zeus_llm::LlmClient>) -> Self {
        Self {
            user_context,
            llm: Some(llm),
        }
    }

    /// Think through a problem.
    ///
    /// When an LLM is configured, asks it to reason through the problem and
    /// injects that reasoning into the thought chain. Falls back to the
    /// keyword-heuristic path when no LLM is configured or the call fails.
    pub async fn think(&self, problem: &str, ctx: &UserContext) -> Result<ThoughtChain> {
        let mut chain = ThoughtChain::new(problem);

        // Step 1: Analyze the problem
        chain.add_step(Step::new(
            1,
            StepType::Analysis,
            &format!("Understanding the problem: {}", problem),
        ));

        // Step 2: Decompose into sub-problems
        let sub_problems = self.decompose(problem, ctx);
        if !sub_problems.is_empty() {
            chain.add_step(Step::new(
                2,
                StepType::Decomposition,
                &format!("Breaking down into {} sub-problems", sub_problems.len()),
            ));
        }

        // Step 3: Gather information needs
        let info_needs = self.identify_information_needs(problem, ctx);
        if !info_needs.is_empty() {
            chain.add_step(Step::new(
                3,
                StepType::InformationGathering,
                &format!("Need to gather: {}", info_needs.join(", ")),
            ));
        }

        // Step 4: Check constraints
        let constraints = self.check_constraints(problem, ctx);
        let constraints_str = if constraints.is_empty() {
            "None identified".to_string()
        } else {
            constraints.join(", ")
        };
        chain.add_step(Step::new(
            4,
            StepType::ConstraintCheck,
            &format!("Constraints: {}", constraints_str),
        ));

        // Step 5: Generate alternatives
        let alternatives = self.generate_alternatives(problem, ctx);
        chain.alternatives = alternatives;

        // Step 6: Plan actions
        let actions = self.plan_actions(problem, &sub_problems, ctx);
        for (i, action) in actions.into_iter().enumerate() {
            chain.add_step(Step::new(
                5 + i,
                StepType::Action(action.clone()),
                &action.description,
            ));
        }

        // Step 7: Synthesize and conclude — use LLM when available
        if let Some(conclusion) = self.llm_synthesize(problem, &chain).await {
            chain.conclude(&conclusion, true, 0.85);
        } else {
            let conclusion = self.synthesize(&chain, ctx);
            chain.conclude(&conclusion, true, 0.8);
        }

        tracing::info!(
            problem = problem,
            steps = chain.steps.len(),
            thinking_time_ms = chain.thinking_time_ms,
            "Completed reasoning"
        );

        Ok(chain)
    }

    /// Ask the LLM to synthesize a conclusion from the partial thought chain.
    /// Returns `None` when no LLM is configured or the call fails (graceful fallback).
    async fn llm_synthesize(&self, problem: &str, chain: &ThoughtChain) -> Option<String> {
        let llm = self.llm.as_ref()?;

        let action_count = chain
            .steps
            .iter()
            .filter(|s| matches!(s.step_type, StepType::Action(_)))
            .count();

        let prompt = format!(
            "You are a reasoning assistant. Briefly synthesize a conclusion for this task.\n\
             Task: {problem}\n\
             Steps identified: {steps}\n\
             Actions planned: {actions}\n\n\
             Respond with a single concise sentence (≤25 words) summarizing the approach.",
            problem = problem,
            steps = chain.steps.len(),
            actions = action_count,
        );

        let msgs = vec![Message::user(&prompt)];
        match llm.complete(&msgs, &[], None).await {
            Ok(resp) => {
                let text = resp.content.trim().to_string();
                if text.is_empty() { None } else { Some(text) }
            }
            Err(e) => {
                tracing::debug!(error = %e, "LLM reasoning synthesis failed, using keyword fallback");
                None
            }
        }
    }

    /// Decompose a problem into sub-problems
    fn decompose(&self, problem: &str, _ctx: &UserContext) -> Vec<String> {
        let mut sub_problems = Vec::new();
        let problem_lower = problem.to_lowercase();

        // Look for compound tasks
        if problem_lower.contains(" and ") {
            let parts: Vec<&str> = problem.split(" and ").collect();
            for part in parts {
                sub_problems.push(part.trim().to_string());
            }
        }

        // Look for sequential indicators
        if problem_lower.contains(" then ") {
            let parts: Vec<&str> = problem.split(" then ").collect();
            for part in parts {
                sub_problems.push(part.trim().to_string());
            }
        }

        // Common decomposition patterns
        if problem_lower.contains("plan") {
            sub_problems.push("Define objectives".to_string());
            sub_problems.push("Identify resources".to_string());
            sub_problems.push("Create timeline".to_string());
            sub_problems.push("Assign responsibilities".to_string());
        } else if problem_lower.contains("organize") {
            sub_problems.push("Inventory current items".to_string());
            sub_problems.push("Define categories".to_string());
            sub_problems.push("Sort and arrange".to_string());
        } else if problem_lower.contains("research") {
            sub_problems.push("Define research questions".to_string());
            sub_problems.push("Identify sources".to_string());
            sub_problems.push("Gather information".to_string());
            sub_problems.push("Synthesize findings".to_string());
        }

        sub_problems
    }

    /// Identify what information is needed
    fn identify_information_needs(&self, problem: &str, ctx: &UserContext) -> Vec<String> {
        let mut needs = Vec::new();
        let problem_lower = problem.to_lowercase();

        // Check for missing context
        if problem_lower.contains("meeting") && ctx.entities.is_empty() {
            needs.push("participant information".to_string());
        }

        if problem_lower.contains("schedule") && ctx.timezone.is_none() {
            needs.push("timezone preference".to_string());
        }

        if (problem_lower.contains("email") || problem_lower.contains("send"))
            && !ctx.preferences.iter().any(|p| p.key == "email_signature")
        {
            needs.push("email preferences".to_string());
        }

        // Generic needs based on action type
        if problem_lower.contains("compare") {
            needs.push("items to compare".to_string());
            needs.push("comparison criteria".to_string());
        }

        if problem_lower.contains("summarize") || problem_lower.contains("summary") {
            needs.push("source material".to_string());
            needs.push("summary length preference".to_string());
        }

        needs
    }

    /// Check for constraints
    fn check_constraints(&self, problem: &str, ctx: &UserContext) -> Vec<String> {
        let mut constraints = Vec::new();
        let problem_lower = problem.to_lowercase();

        // Time constraints
        if problem_lower.contains("by ") || problem_lower.contains("before ") {
            constraints.push("Time deadline specified".to_string());
        }

        // Resource constraints
        if problem_lower.contains("budget") || problem_lower.contains("cost") {
            constraints.push("Budget considerations".to_string());
        }

        // Permission constraints
        if problem_lower.contains("confidential") || problem_lower.contains("private") {
            constraints.push("Privacy/confidentiality required".to_string());
        }

        // Check user-level constraints from preferences
        for pref in &ctx.preferences {
            if pref.key.starts_with("constraint_") {
                constraints.push(pref.value.clone());
            }
        }

        constraints
    }

    /// Generate alternative approaches
    fn generate_alternatives(&self, problem: &str, _ctx: &UserContext) -> Vec<Alternative> {
        let mut alternatives = Vec::new();
        let problem_lower = problem.to_lowercase();

        if problem_lower.contains("schedule") || problem_lower.contains("meeting") {
            alternatives.push(Alternative {
                description: "Use async communication instead of meeting".to_string(),
                rejection_reason: "User specifically requested a meeting".to_string(),
                estimated_success: 0.6,
            });
        }

        if problem_lower.contains("email") {
            alternatives.push(Alternative {
                description: "Use instant messaging for faster response".to_string(),
                rejection_reason: "Email may be more appropriate for formal communication"
                    .to_string(),
                estimated_success: 0.5,
            });
        }

        if problem_lower.contains("create") || problem_lower.contains("write") {
            alternatives.push(Alternative {
                description: "Use a template instead of creating from scratch".to_string(),
                rejection_reason: "Custom creation may be required".to_string(),
                estimated_success: 0.7,
            });
        }

        alternatives
    }

    /// Plan specific actions
    fn plan_actions(
        &self,
        problem: &str,
        sub_problems: &[String],
        _ctx: &UserContext,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let problem_lower = problem.to_lowercase();

        if sub_problems.is_empty() {
            // Single action for simple problems
            let mut action = Action::new(&format!("Execute: {}", problem));
            action.priority = 5;

            // Try to identify appropriate tool
            if problem_lower.contains("file") || problem_lower.contains("document") {
                action = action.with_tool("write_file", serde_json::json!({}));
            } else if problem_lower.contains("search") || problem_lower.contains("find") {
                action = action.with_tool("search", serde_json::json!({}));
            } else if problem_lower.contains("email") || problem_lower.contains("send") {
                action = action.with_tool("send_email", serde_json::json!({}));
            } else if problem_lower.contains("calendar") || problem_lower.contains("schedule") {
                action = action.with_tool("calendar_create_event", serde_json::json!({}));
            }

            actions.push(action);
        } else {
            // Action for each sub-problem
            let mut prev_id: Option<String> = None;

            for (i, sub) in sub_problems.iter().enumerate() {
                let mut action = Action::new(sub);
                action.priority = 10u8.saturating_sub(i as u8).max(1);

                // Add dependency on previous action
                if let Some(prev) = &prev_id {
                    action = action.depends_on(prev);
                    action.parallelizable = false;
                }

                prev_id = Some(action.id.clone());
                actions.push(action);
            }
        }

        actions
    }

    /// Synthesize a conclusion from the reasoning
    fn synthesize(&self, chain: &ThoughtChain, _ctx: &UserContext) -> String {
        let action_count = chain
            .steps
            .iter()
            .filter(|s| matches!(s.step_type, StepType::Action(_)))
            .count();

        if action_count == 0 {
            format!(
                "Analyzed '{}' but no specific actions identified",
                chain.problem
            )
        } else if action_count == 1 {
            format!("Ready to execute single action for: {}", chain.problem)
        } else {
            format!(
                "Created {} action plan with {} steps for: {}",
                if chain.alternatives.is_empty() {
                    "optimal"
                } else {
                    "best"
                },
                action_count,
                chain.problem
            )
        }
    }

    /// Get the user context
    pub fn user_context(&self) -> &Arc<RwLock<UserContext>> {
        &self.user_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thought_chain_creation() {
        let chain = ThoughtChain::new("Test problem");
        assert!(!chain.success);
        assert!(chain.steps.is_empty());
    }

    #[test]
    fn test_action_creation() {
        let action = Action::new("Send email")
            .with_tool("send_email", serde_json::json!({"to": "test@example.com"}));

        assert_eq!(action.tool, Some("send_email".to_string()));
    }

    #[test]
    fn test_action_dependencies() {
        let action1 = Action::new("First");
        let action2 = Action::new("Second").depends_on(&action1.id);

        assert!(action2.dependencies.contains(&action1.id));
    }

    #[tokio::test]
    async fn test_reasoning_engine() {
        let ctx = Arc::new(RwLock::new(UserContext::default()));
        let engine = ReasoningEngine::new(ctx.clone());

        let user_ctx = ctx.read().await;
        let chain = engine
            .think("Plan a team meeting", &user_ctx)
            .await
            .expect("async operation should succeed");

        assert!(!chain.steps.is_empty());
        assert!(chain.conclusion.is_some());
    }

    #[test]
    fn test_thought_chain_empty() {
        let chain = ThoughtChain::new("Empty problem");
        assert!(chain.steps.is_empty());
        assert!(chain.conclusion.is_none());
        assert!(!chain.success);
        assert_eq!(chain.confidence, 0.0);
        assert!(chain.alternatives.is_empty());
        assert_eq!(chain.thinking_time_ms, 0);
    }

    #[test]
    fn test_thought_chain_multiple_steps() {
        let mut chain = ThoughtChain::new("Multi-step problem");
        chain.add_step(Step::new(1, StepType::Analysis, "Analyzing"));
        chain.add_step(Step::new(2, StepType::Decomposition, "Breaking down"));
        chain.add_step(Step::new(3, StepType::Synthesis, "Combining results"));
        chain.add_step(Step::new(4, StepType::Verification, "Verifying"));

        assert_eq!(chain.steps.len(), 4);
        assert_eq!(chain.steps[0].number, 1);
        assert_eq!(chain.steps[3].number, 4);
    }

    #[test]
    fn test_thought_chain_conclusion_override() {
        let mut chain = ThoughtChain::new("Problem");
        chain.conclude("First conclusion", true, 0.7);
        assert_eq!(chain.conclusion.as_deref(), Some("First conclusion"));
        assert!(chain.success);
        assert_eq!(chain.confidence, 0.7);

        // Conclude again to override
        chain.conclude("Second conclusion", false, 0.3);
        assert_eq!(chain.conclusion.as_deref(), Some("Second conclusion"));
        assert!(!chain.success);
        assert_eq!(chain.confidence, 0.3);
    }

    #[test]
    fn test_action_without_tool() {
        let action = Action::new("Manual review step");
        assert!(action.tool.is_none());
        assert_eq!(action.parameters, serde_json::json!({}));
        assert!(action.dependencies.is_empty());
    }

    #[test]
    fn test_action_multiple_dependencies() {
        let a1 = Action::new("Step 1");
        let a2 = Action::new("Step 2");
        let a3 = Action::new("Step 3");

        let dependent = Action::new("Final step")
            .depends_on(&a1.id)
            .depends_on(&a2.id)
            .depends_on(&a3.id);

        assert_eq!(dependent.dependencies.len(), 3);
        assert!(dependent.dependencies.contains(&a1.id));
        assert!(dependent.dependencies.contains(&a2.id));
        assert!(dependent.dependencies.contains(&a3.id));
    }

    #[test]
    fn test_action_serialization() {
        let action =
            Action::new("Test action").with_tool("shell", serde_json::json!({"cmd": "ls"}));

        let json = serde_json::to_string(&action).expect("should serialize to JSON");
        let deserialized: Action = serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deserialized.description, "Test action");
        assert_eq!(deserialized.tool, Some("shell".to_string()));
        assert_eq!(deserialized.parameters, serde_json::json!({"cmd": "ls"}));
    }

    #[test]
    fn test_thought_chain_serialization() {
        let mut chain = ThoughtChain::new("Serialize me");
        chain.add_step(Step::new(1, StepType::Analysis, "Step 1"));
        chain.conclude("Done", true, 0.9);

        let json = serde_json::to_string(&chain).expect("should serialize to JSON");
        let deserialized: ThoughtChain =
            serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deserialized.problem, "Serialize me");
        assert_eq!(deserialized.steps.len(), 1);
        assert_eq!(deserialized.conclusion.as_deref(), Some("Done"));
        assert!(deserialized.success);
        assert_eq!(deserialized.confidence, 0.9);
    }

    #[test]
    fn test_action_default_fields() {
        let action = Action::new("Default test");
        assert_eq!(action.priority, 5);
        assert_eq!(action.complexity, 5);
        assert!(action.parallelizable);
        assert!(action.tool.is_none());
        assert!(action.dependencies.is_empty());
        assert!(!action.id.is_empty());
        assert_eq!(action.description, "Default test");
    }
}
