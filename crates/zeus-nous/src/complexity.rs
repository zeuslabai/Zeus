//! Complexity analysis for intent routing and approval gating
//!
//! Classifies plans into complexity levels and determines whether
//! human approval is required before execution. Used by the Pantheon
//! War Room to decide between direct execution and plan-card approval flow.

use serde::{Deserialize, Serialize};
use zeus_core::ToolSchema;

// ── Types ─────────────────────────────────────────────────

/// How complex a plan is — determines the execution flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComplexityLevel {
    /// 0-1 tool calls, read-only or simple question → execute immediately
    Simple,
    /// 2-5 steps, some writes → execute with streaming progress
    Moderate,
    /// 6+ steps or destructive tools → show plan card, require approval
    Complex,
}

/// Risk level of a plan — drives approval requirements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Read-only operations (read_file, list_dir, web_fetch, search)
    Low,
    /// Write operations to safe locations (write_file, edit_file)
    Medium,
    /// System changes, external APIs, shell commands, deletions
    High,
}

/// A plan card summarizing what will happen before execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCard {
    /// Unique identifier for this plan
    pub plan_id: String,
    /// The original user request
    pub original_request: String,
    /// Classified complexity
    pub complexity: ComplexityLevel,
    /// Risk assessment
    pub risk: RiskLevel,
    /// Whether human approval is required before execution
    pub requires_approval: bool,
    /// Why approval is needed (if applicable)
    pub approval_reason: Option<String>,
    /// Number of steps in the plan
    pub step_count: usize,
    /// Brief descriptions of each step
    pub step_summaries: Vec<StepSummary>,
    /// Tools that will be used
    pub tools_used: Vec<String>,
    /// Estimated token cost (rough)
    pub estimated_tokens: Option<u64>,
}

/// Summary of a single plan step for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSummary {
    pub step_number: usize,
    pub description: String,
    pub tool: Option<String>,
    pub risk: RiskLevel,
}

// ── Analyzer ──────────────────────────────────────────────

/// Classifies plans by complexity and risk, determines approval requirements
pub struct ComplexityAnalyzer {
    /// Tools that are considered high-risk (shell, destructive ops)
    high_risk_tools: Vec<String>,
    /// Tools that are considered medium-risk (writes)
    medium_risk_tools: Vec<String>,
    /// Step count threshold for Complex classification
    complex_threshold: usize,
}

impl Default for ComplexityAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ComplexityAnalyzer {
    pub fn new() -> Self {
        Self {
            high_risk_tools: vec![
                "shell".to_string(),
                "spawn".to_string(),
                "message".to_string(),
            ],
            medium_risk_tools: vec![
                "write_file".to_string(),
                "edit_file".to_string(),
            ],
            complex_threshold: 5,
        }
    }

    /// Classify a plan into a PlanCard with complexity, risk, and approval requirements
    pub fn analyze(
        &self,
        plan_id: &str,
        original_request: &str,
        steps: &[(String, Option<String>)], // (description, tool_name)
        _available_tools: &[ToolSchema],
    ) -> PlanCard {
        let step_count = steps.len();
        let mut tools_used = Vec::new();
        let mut max_risk = RiskLevel::Low;
        let mut has_shell = false;
        let mut has_spawn = false;
        let mut has_message = false;

        let step_summaries: Vec<StepSummary> = steps
            .iter()
            .enumerate()
            .map(|(i, (desc, tool))| {
                let risk = tool
                    .as_ref()
                    .map(|t| self.tool_risk(t))
                    .unwrap_or(RiskLevel::Low);

                if risk as u8 > max_risk as u8 {
                    max_risk = risk;
                }

                if let Some(t) = tool {
                    if !tools_used.contains(t) {
                        tools_used.push(t.clone());
                    }
                    if t == "shell" {
                        has_shell = true;
                    }
                    if t == "spawn" {
                        has_spawn = true;
                    }
                    if t == "message" {
                        has_message = true;
                    }
                }

                StepSummary {
                    step_number: i + 1,
                    description: desc.clone(),
                    tool: tool.clone(),
                    risk,
                }
            })
            .collect();

        // Determine complexity level
        let complexity = if step_count <= 1 && max_risk == RiskLevel::Low {
            ComplexityLevel::Simple
        } else if step_count >= self.complex_threshold || max_risk == RiskLevel::High {
            ComplexityLevel::Complex
        } else {
            ComplexityLevel::Moderate
        };

        // Determine if approval is required
        let (requires_approval, approval_reason) = self.check_approval(
            complexity,
            max_risk,
            has_shell,
            has_spawn,
            has_message,
            step_count,
        );

        // Rough token estimate: ~500 tokens per step (LLM call + tool result)
        let estimated_tokens = Some(step_count as u64 * 500);

        PlanCard {
            plan_id: plan_id.to_string(),
            original_request: original_request.to_string(),
            complexity,
            risk: max_risk,
            requires_approval,
            approval_reason,
            step_count,
            step_summaries,
            tools_used,
            estimated_tokens,
        }
    }

    /// Classify a raw user message without a plan (for intent routing)
    pub fn classify_message(&self, message: &str) -> ComplexityLevel {
        let lower = message.to_lowercase();

        // Simple patterns — questions, lookups, single actions
        let simple_patterns = [
            "what is", "what's", "how do", "how does", "tell me", "explain",
            "show me", "list", "find", "search", "read", "check", "status",
            "who is", "when", "where", "why", "define",
        ];
        if simple_patterns.iter().any(|p| lower.starts_with(p)) {
            return ComplexityLevel::Simple;
        }

        // Complex patterns — multi-step, creation, deployment
        let complex_patterns = [
            "build me", "create a", "deploy", "launch", "set up",
            "migrate", "refactor", "redesign", "automate", "schedule",
            "campaign", "pipeline", "workflow", "integrate",
        ];
        if complex_patterns.iter().any(|p| lower.contains(p)) {
            return ComplexityLevel::Complex;
        }

        // Default to moderate
        ComplexityLevel::Moderate
    }

    /// Check if a complex message is too vague to plan without clarification.
    /// Returns true if the message lacks enough specifics to decompose into steps.
    pub fn needs_clarification(&self, message: &str) -> bool {
        let lower = message.to_lowercase();
        let word_count = message.split_whitespace().count();

        // Very short complex messages are almost always too vague
        // "build me a website" (4 words) — vague
        // "build me a landing page for FC Thunder with roster and schedule pages in Next.js" — specific enough
        if word_count < 8 {
            return true;
        }

        // Check for vague markers without specifics
        let vague_patterns = [
            "something", "stuff", "things", "whatever", "anything",
            "somehow", "some kind of", "a thing",
        ];
        if vague_patterns.iter().any(|p| lower.contains(p)) {
            return true;
        }

        // If it's complex but has detail indicators, it's specific enough
        let detail_indicators = [
            "using", "with", "in", "for", "called", "named",
            "that", "which", "including", "features", "pages",
            "should", "must", "needs to",
        ];
        let detail_count = detail_indicators.iter().filter(|p| lower.contains(**p)).count();

        // Fewer than 2 detail indicators in a complex request = likely vague
        detail_count < 2
    }

    /// Get risk level for a specific tool
    fn tool_risk(&self, tool_name: &str) -> RiskLevel {
        if self.high_risk_tools.iter().any(|t| t == tool_name) {
            RiskLevel::High
        } else if self.medium_risk_tools.iter().any(|t| t == tool_name) {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }

    /// Determine if the plan requires human approval and why
    fn check_approval(
        &self,
        complexity: ComplexityLevel,
        risk: RiskLevel,
        has_shell: bool,
        has_spawn: bool,
        has_message: bool,
        step_count: usize,
    ) -> (bool, Option<String>) {
        // High risk always needs approval
        if risk == RiskLevel::High {
            let mut reasons = Vec::new();
            if has_shell {
                reasons.push("shell commands");
            }
            if has_spawn {
                reasons.push("sub-agent spawning");
            }
            if has_message {
                reasons.push("external messaging");
            }
            let reason = if reasons.is_empty() {
                "high-risk tools".to_string()
            } else {
                format!("plan uses {}", reasons.join(", "))
            };
            return (true, Some(reason));
        }

        // Complex plans with many steps need approval
        if complexity == ComplexityLevel::Complex && step_count >= 6 {
            return (
                true,
                Some(format!("complex plan with {} steps", step_count)),
            );
        }

        (false, None)
    }
}

// ── Display helpers ───────────────────────────────────────

impl PlanCard {
    /// Format the plan card as a chat message for the War Room
    pub fn to_chat_message(&self) -> String {
        let complexity_icon = match self.complexity {
            ComplexityLevel::Simple => "🟢",
            ComplexityLevel::Moderate => "🟡",
            ComplexityLevel::Complex => "🔴",
        };

        let risk_label = match self.risk {
            RiskLevel::Low => "low risk",
            RiskLevel::Medium => "medium risk",
            RiskLevel::High => "high risk",
        };

        let mut msg = format!(
            "{} Plan: {} steps · {} · {}\n",
            complexity_icon, self.step_count, risk_label,
            self.complexity_label()
        );

        for step in &self.step_summaries {
            let tool_label = step
                .tool
                .as_ref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            msg.push_str(&format!(
                "  {}. {}{}\n",
                step.step_number, step.description, tool_label
            ));
        }

        if let Some(tokens) = self.estimated_tokens {
            msg.push_str(&format!("  ~{} tokens estimated\n", tokens));
        }

        if let (true, Some(reason)) = (self.requires_approval, &self.approval_reason) {
            msg.push_str(&format!(
                "\n⚠️ Approval required: {}\nUse /approve {} or /reject {}",
                reason, self.plan_id, self.plan_id
            ));
        }

        msg
    }

    fn complexity_label(&self) -> &'static str {
        match self.complexity {
            ComplexityLevel::Simple => "simple",
            ComplexityLevel::Moderate => "moderate",
            ComplexityLevel::Complex => "complex",
        }
    }
}

impl std::fmt::Display for ComplexityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple => write!(f, "simple"),
            Self::Moderate => write!(f, "moderate"),
            Self::Complex => write!(f, "complex"),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_classification() {
        let analyzer = ComplexityAnalyzer::new();
        let steps = vec![("Read the config file".to_string(), Some("read_file".to_string()))];
        let card = analyzer.analyze("p1", "read my config", &steps, &[]);
        assert_eq!(card.complexity, ComplexityLevel::Simple);
        assert_eq!(card.risk, RiskLevel::Low);
        assert!(!card.requires_approval);
    }

    #[test]
    fn test_moderate_classification() {
        let analyzer = ComplexityAnalyzer::new();
        let steps = vec![
            ("Read current file".to_string(), Some("read_file".to_string())),
            ("Edit the function".to_string(), Some("edit_file".to_string())),
            ("Verify the change".to_string(), Some("read_file".to_string())),
        ];
        let card = analyzer.analyze("p2", "fix the bug", &steps, &[]);
        assert_eq!(card.complexity, ComplexityLevel::Moderate);
        assert_eq!(card.risk, RiskLevel::Medium);
        assert!(!card.requires_approval);
    }

    #[test]
    fn test_complex_with_shell() {
        let analyzer = ComplexityAnalyzer::new();
        let steps = vec![
            ("Read project".to_string(), Some("read_file".to_string())),
            ("Install deps".to_string(), Some("shell".to_string())),
            ("Build project".to_string(), Some("shell".to_string())),
            ("Deploy".to_string(), Some("shell".to_string())),
        ];
        let card = analyzer.analyze("p3", "deploy the app", &steps, &[]);
        assert_eq!(card.complexity, ComplexityLevel::Complex);
        assert_eq!(card.risk, RiskLevel::High);
        assert!(card.requires_approval);
        assert!(card.approval_reason.unwrap().contains("shell commands"));
    }

    #[test]
    fn test_many_steps_complex() {
        let analyzer = ComplexityAnalyzer::new();
        let steps: Vec<_> = (0..7)
            .map(|i| (format!("Step {}", i + 1), Some("read_file".to_string())))
            .collect();
        let card = analyzer.analyze("p4", "analyze everything", &steps, &[]);
        assert_eq!(card.complexity, ComplexityLevel::Complex);
        // Low risk but many steps → still requires approval
        assert!(card.requires_approval);
    }

    #[test]
    fn test_message_classification() {
        let analyzer = ComplexityAnalyzer::new();

        assert_eq!(
            analyzer.classify_message("what is the weather today"),
            ComplexityLevel::Simple
        );
        assert_eq!(
            analyzer.classify_message("build me a website for my restaurant"),
            ComplexityLevel::Complex
        );
        assert_eq!(
            analyzer.classify_message("fix this typo in the README"),
            ComplexityLevel::Moderate
        );
    }

    #[test]
    fn test_plan_card_display() {
        let analyzer = ComplexityAnalyzer::new();
        let steps = vec![
            ("Scaffold project".to_string(), Some("shell".to_string())),
            ("Generate homepage".to_string(), Some("write_file".to_string())),
            ("Deploy to server".to_string(), Some("shell".to_string())),
        ];
        let card = analyzer.analyze("spawn-abc", "build me a website", &steps, &[]);
        let msg = card.to_chat_message();
        assert!(msg.contains("3 steps"));
        assert!(msg.contains("/approve spawn-abc"));
    }

    #[test]
    fn test_needs_clarification_short_vague() {
        let analyzer = ComplexityAnalyzer::new();
        // Very short complex requests — vague
        assert!(analyzer.needs_clarification("build me a website"));
        assert!(analyzer.needs_clarification("create a thing"));
        assert!(analyzer.needs_clarification("deploy the app"));
    }

    #[test]
    fn test_needs_clarification_vague_markers() {
        let analyzer = ComplexityAnalyzer::new();
        assert!(analyzer.needs_clarification("build me something cool for my project or whatever works"));
        assert!(analyzer.needs_clarification("create some kind of landing page with stuff on it"));
    }

    #[test]
    fn test_no_clarification_specific_request() {
        let analyzer = ComplexityAnalyzer::new();
        // Specific enough — has detail indicators
        assert!(!analyzer.needs_clarification(
            "build me a landing page for FC Thunder using Next.js with a roster and schedule page"
        ));
        assert!(!analyzer.needs_clarification(
            "create a REST API for user management that includes authentication and role-based access control"
        ));
        assert!(!analyzer.needs_clarification(
            "deploy the Zeus gateway to FreeBSD .224 using the deploy script with the latest binary"
        ));
    }
}
