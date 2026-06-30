//! Team Recommendation Engine
//!
//! Analyzes goal descriptions to recommend optimal team compositions.
//! Uses keyword-based intent classification to determine scope, complexity,
//! and required capabilities, then maps those to agent roles and model tiers.

use serde::{Deserialize, Serialize};

// ============================================================================
// Types
// ============================================================================

/// A recommended team composition for a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecommendation {
    pub team_name: String,
    pub coordinators: Vec<AgentRole>,
    pub workers: Vec<AgentRole>,
    pub rationale: String,
    pub estimated_complexity: Complexity,
    pub estimated_steps: usize,
    pub scope: Scope,
}

/// A recommended agent role within a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRole {
    pub role: String,
    pub capabilities: Vec<String>,
    pub model_tier: ModelTier,
}

/// Project complexity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Low,
    Medium,
    High,
    VeryHigh,
}

impl Complexity {
    pub fn label(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::VeryHigh => "very_high",
        }
    }
}

impl std::fmt::Display for Complexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Project scope / domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Frontend,
    Backend,
    Fullstack,
    Data,
    DevOps,
    Mobile,
    Systems,
    Research,
    General,
}

impl Scope {
    pub fn label(&self) -> &str {
        match self {
            Self::Frontend => "frontend",
            Self::Backend => "backend",
            Self::Fullstack => "fullstack",
            Self::Data => "data",
            Self::DevOps => "devops",
            Self::Mobile => "mobile",
            Self::Systems => "systems",
            Self::Research => "research",
            Self::General => "general",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Model tier for agent assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// Fast, cheap — good for validation, testing, simple tasks
    Haiku,
    /// Balanced — good for most development work
    Sonnet,
    /// Powerful — for architecture, complex reasoning, coordination
    Opus,
}

impl ModelTier {
    pub fn label(&self) -> &str {
        match self {
            Self::Haiku => "haiku",
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
        }
    }
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ============================================================================
// Goal Analysis
// ============================================================================

/// Analyzed properties of a goal, extracted via keyword classification.
#[derive(Debug, Clone)]
pub struct GoalProfile {
    pub scope: Scope,
    pub complexity: Complexity,
    pub capabilities_needed: Vec<String>,
    pub estimated_steps: usize,
}

/// Analyze a goal description to determine scope, complexity, and capabilities.
pub fn analyze_goal(goal: &str) -> GoalProfile {
    let lower = goal.to_lowercase();

    let scope = classify_scope(&lower);
    let complexity = classify_complexity(&lower, scope);
    let capabilities = identify_capabilities(&lower, scope);
    let estimated_steps = estimate_steps(complexity);

    GoalProfile {
        scope,
        complexity,
        capabilities_needed: capabilities,
        estimated_steps,
    }
}

fn classify_scope(goal: &str) -> Scope {
    // Frontend signals
    let frontend_keywords = [
        "react",
        "vue",
        "angular",
        "svelte",
        "nextjs",
        "next.js",
        "html",
        "css",
        "tailwind",
        "ui",
        "frontend",
        "front-end",
        "webpage",
        "web page",
        "website",
        "landing page",
        "dashboard",
        "component",
        "leptos",
        "wasm",
    ];
    // Backend signals
    let backend_keywords = [
        "api",
        "server",
        "database",
        "sql",
        "postgres",
        "mysql",
        "redis",
        "backend",
        "back-end",
        "endpoint",
        "rest",
        "graphql",
        "grpc",
        "microservice",
        "lambda",
        "serverless",
        "auth",
    ];
    // Mobile signals
    let mobile_keywords = [
        "ios",
        "android",
        "mobile",
        "swift",
        "kotlin",
        "flutter",
        "react native",
        "swiftui",
        "jetpack compose",
    ];
    // Data signals
    let data_keywords = [
        "data",
        "ml",
        "machine learning",
        "ai model",
        "training",
        "dataset",
        "analytics",
        "pipeline",
        "etl",
        "pandas",
        "numpy",
        "tensorflow",
        "pytorch",
        "jupyter",
    ];
    // DevOps signals
    let devops_keywords = [
        "deploy",
        "docker",
        "kubernetes",
        "ci/cd",
        "terraform",
        "aws",
        "gcp",
        "azure cloud",
        "infrastructure",
        "monitoring",
        "helm",
        "ansible",
    ];
    // Systems signals
    let systems_keywords = [
        "rust",
        "c++",
        "kernel",
        "driver",
        "embedded",
        "systems",
        "compiler",
        "linker",
        "os",
        "performance",
        "optimization",
        "low-level",
    ];
    // Research signals
    let research_keywords = [
        "research",
        "paper",
        "analyze",
        "study",
        "investigate",
        "compare",
        "benchmark",
        "evaluate",
        "survey",
        "whitepaper",
    ];

    let score =
        |keywords: &[&str]| -> usize { keywords.iter().filter(|kw| goal.contains(*kw)).count() };

    let scores = [
        (Scope::Frontend, score(&frontend_keywords)),
        (Scope::Backend, score(&backend_keywords)),
        (Scope::Mobile, score(&mobile_keywords)),
        (Scope::Data, score(&data_keywords)),
        (Scope::DevOps, score(&devops_keywords)),
        (Scope::Systems, score(&systems_keywords)),
        (Scope::Research, score(&research_keywords)),
    ];

    let (best_scope, best_score) = scores
        .iter()
        .max_by_key(|(_, s)| *s)
        .copied()
        .unwrap_or((Scope::General, 0));

    // Check for fullstack (both frontend and backend signals)
    let fe_score = score(&frontend_keywords);
    let be_score = score(&backend_keywords);
    if fe_score >= 1 && be_score >= 1 {
        return Scope::Fullstack;
    }

    if best_score == 0 {
        Scope::General
    } else {
        best_scope
    }
}

fn classify_complexity(goal: &str, scope: Scope) -> Complexity {
    let word_count = goal.split_whitespace().count();

    // Complexity boosters
    let complex_signals = [
        "authentication",
        "authorization",
        "real-time",
        "realtime",
        "streaming",
        "websocket",
        "multi-tenant",
        "microservice",
        "distributed",
        "scale",
        "production",
        "enterprise",
        "full-featured",
        "complete",
        "comprehensive",
    ];
    let simple_signals = [
        "simple",
        "basic",
        "hello world",
        "tutorial",
        "example",
        "demo",
        "prototype",
        "quick",
        "minimal",
        "tiny",
    ];

    let complex_count = complex_signals
        .iter()
        .filter(|kw| goal.contains(*kw))
        .count();
    let simple_count = simple_signals
        .iter()
        .filter(|kw| goal.contains(*kw))
        .count();

    if simple_count > 0 && complex_count == 0 {
        return Complexity::Low;
    }

    if complex_count >= 3 || (word_count > 50 && complex_count >= 1) {
        return Complexity::VeryHigh;
    }

    if complex_count >= 1 || word_count > 30 || scope == Scope::Fullstack {
        return Complexity::High;
    }

    if word_count > 15 {
        return Complexity::Medium;
    }

    Complexity::Medium
}

fn identify_capabilities(goal: &str, scope: Scope) -> Vec<String> {
    let mut caps = Vec::new();

    // Scope-based capabilities
    match scope {
        Scope::Frontend => {
            caps.push("ui-design".to_string());
            caps.push("frontend-dev".to_string());
        }
        Scope::Backend => {
            caps.push("backend-dev".to_string());
            caps.push("api-design".to_string());
        }
        Scope::Fullstack => {
            caps.push("frontend-dev".to_string());
            caps.push("backend-dev".to_string());
            caps.push("api-design".to_string());
        }
        Scope::Data => {
            caps.push("data-engineering".to_string());
            caps.push("ml-ops".to_string());
        }
        Scope::DevOps => {
            caps.push("infrastructure".to_string());
            caps.push("ci-cd".to_string());
        }
        Scope::Mobile => {
            caps.push("mobile-dev".to_string());
        }
        Scope::Systems => {
            caps.push("systems-programming".to_string());
        }
        Scope::Research => {
            caps.push("research".to_string());
            caps.push("analysis".to_string());
        }
        Scope::General => {
            caps.push("general-dev".to_string());
        }
    }

    // Additional capability signals
    if goal.contains("test") || goal.contains("spec") || goal.contains("coverage") {
        caps.push("testing".to_string());
    }
    if goal.contains("deploy") || goal.contains("docker") || goal.contains("ci") {
        caps.push("devops".to_string());
    }
    if goal.contains("database") || goal.contains("sql") || goal.contains("migration") {
        caps.push("database".to_string());
    }
    if goal.contains("auth") || goal.contains("security") || goal.contains("oauth") {
        caps.push("security".to_string());
    }
    if goal.contains("documentation") || goal.contains("readme") || goal.contains("docs") {
        caps.push("documentation".to_string());
    }

    caps.dedup();
    caps
}

fn estimate_steps(complexity: Complexity) -> usize {
    match complexity {
        Complexity::Low => 3,
        Complexity::Medium => 6,
        Complexity::High => 10,
        Complexity::VeryHigh => 15,
    }
}

// ============================================================================
// Team Recommender
// ============================================================================

/// Recommend a team composition based on a goal description.
pub fn recommend_team(goal: &str) -> TeamRecommendation {
    let profile = analyze_goal(goal);
    recommend_from_profile(&profile)
}

/// Recommend a team composition from an already-analyzed goal profile.
pub fn recommend_from_profile(profile: &GoalProfile) -> TeamRecommendation {
    let (coordinators, workers) = build_team(profile);

    TeamRecommendation {
        team_name: format!("{}-team", profile.scope.label()),
        coordinators,
        workers,
        rationale: build_rationale(profile),
        estimated_complexity: profile.complexity,
        estimated_steps: profile.estimated_steps,
        scope: profile.scope,
    }
}

fn build_team(profile: &GoalProfile) -> (Vec<AgentRole>, Vec<AgentRole>) {
    match profile.complexity {
        Complexity::Low => (
            vec![AgentRole {
                role: "lead-developer".to_string(),
                capabilities: profile.capabilities_needed.clone(),
                model_tier: ModelTier::Sonnet,
            }],
            vec![],
        ),
        Complexity::Medium => (
            vec![AgentRole {
                role: "project-lead".to_string(),
                capabilities: vec!["planning".to_string(), "code-review".to_string()],
                model_tier: ModelTier::Opus,
            }],
            vec![
                AgentRole {
                    role: "developer".to_string(),
                    capabilities: profile.capabilities_needed.clone(),
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "qa".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: ModelTier::Haiku,
                },
            ],
        ),
        Complexity::High => {
            let mut workers = vec![
                AgentRole {
                    role: "senior-developer".to_string(),
                    capabilities: profile.capabilities_needed.clone(),
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "developer".to_string(),
                    capabilities: vec!["implementation".to_string(), "testing".to_string()],
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "qa-engineer".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: ModelTier::Haiku,
                },
            ];

            // Add specialized roles based on capabilities
            if profile
                .capabilities_needed
                .contains(&"database".to_string())
            {
                workers.push(AgentRole {
                    role: "data-engineer".to_string(),
                    capabilities: vec!["database".to_string(), "migration".to_string()],
                    model_tier: ModelTier::Sonnet,
                });
            }

            (
                vec![AgentRole {
                    role: "project-lead".to_string(),
                    capabilities: vec![
                        "planning".to_string(),
                        "architecture".to_string(),
                        "code-review".to_string(),
                    ],
                    model_tier: ModelTier::Opus,
                }],
                workers,
            )
        }
        Complexity::VeryHigh => {
            let mut workers = vec![
                AgentRole {
                    role: "architect".to_string(),
                    capabilities: vec!["architecture".to_string(), "system-design".to_string()],
                    model_tier: ModelTier::Opus,
                },
                AgentRole {
                    role: "senior-developer-1".to_string(),
                    capabilities: profile.capabilities_needed.clone(),
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "senior-developer-2".to_string(),
                    capabilities: vec!["implementation".to_string(), "testing".to_string()],
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "developer".to_string(),
                    capabilities: vec!["implementation".to_string()],
                    model_tier: ModelTier::Sonnet,
                },
                AgentRole {
                    role: "qa-lead".to_string(),
                    capabilities: vec![
                        "testing".to_string(),
                        "integration-testing".to_string(),
                        "validation".to_string(),
                    ],
                    model_tier: ModelTier::Sonnet,
                },
            ];

            if profile.capabilities_needed.contains(&"devops".to_string()) {
                workers.push(AgentRole {
                    role: "devops-engineer".to_string(),
                    capabilities: vec!["infrastructure".to_string(), "ci-cd".to_string()],
                    model_tier: ModelTier::Sonnet,
                });
            }

            (
                vec![AgentRole {
                    role: "project-director".to_string(),
                    capabilities: vec![
                        "planning".to_string(),
                        "architecture".to_string(),
                        "code-review".to_string(),
                        "risk-management".to_string(),
                    ],
                    model_tier: ModelTier::Opus,
                }],
                workers,
            )
        }
    }
}

fn build_rationale(profile: &GoalProfile) -> String {
    let agent_count = match profile.complexity {
        Complexity::Low => 1,
        Complexity::Medium => 3,
        Complexity::High => 4,
        Complexity::VeryHigh => 7,
    };

    format!(
        "{complexity} complexity {scope} project requiring {agents} agent(s). \
         Key capabilities: {caps}.",
        complexity = profile.complexity,
        scope = profile.scope,
        agents = agent_count,
        caps = profile.capabilities_needed.join(", "),
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_simple_frontend() {
        let profile = analyze_goal("Build a simple React landing page");
        assert_eq!(profile.scope, Scope::Frontend);
        assert_eq!(profile.complexity, Complexity::Low);
        assert!(
            profile
                .capabilities_needed
                .contains(&"frontend-dev".to_string())
        );
    }

    #[test]
    fn test_analyze_fullstack() {
        let profile =
            analyze_goal("Build a React frontend with a REST API backend and PostgreSQL database");
        assert_eq!(profile.scope, Scope::Fullstack);
        assert!(
            profile
                .capabilities_needed
                .contains(&"frontend-dev".to_string())
        );
        assert!(
            profile
                .capabilities_needed
                .contains(&"backend-dev".to_string())
        );
    }

    #[test]
    fn test_analyze_complex_system() {
        let profile = analyze_goal(
            "Build a distributed microservice architecture with authentication, \
             real-time streaming, multi-tenant support, and comprehensive testing",
        );
        assert_eq!(profile.complexity, Complexity::VeryHigh);
    }

    #[test]
    fn test_recommend_low_complexity() {
        let rec = recommend_team("Build a simple hello world app");
        assert_eq!(rec.estimated_complexity, Complexity::Low);
        assert_eq!(rec.coordinators.len(), 1);
        assert_eq!(rec.workers.len(), 0);
    }

    #[test]
    fn test_recommend_medium_complexity() {
        let rec = recommend_team("Build a todo app with React and local storage");
        assert_eq!(rec.coordinators.len(), 1);
        assert!(rec.workers.len() >= 1);
    }

    #[test]
    fn test_recommend_high_complexity() {
        let rec = recommend_team(
            "Build a real-time chat application with React frontend, Node.js API, \
             PostgreSQL database, and WebSocket streaming",
        );
        assert!(matches!(
            rec.estimated_complexity,
            Complexity::High | Complexity::VeryHigh
        ));
        assert!(rec.coordinators.len() >= 1);
        assert!(rec.workers.len() >= 2);
    }

    #[test]
    fn test_scope_detection() {
        assert_eq!(
            classify_scope("build an ios app with swiftui"),
            Scope::Mobile
        );
        assert_eq!(
            classify_scope("deploy docker containers to kubernetes"),
            Scope::DevOps
        );
        assert_eq!(
            classify_scope("train a machine learning model on the dataset"),
            Scope::Data
        );
        assert_eq!(
            classify_scope("write a rust compiler optimization"),
            Scope::Systems
        );
    }

    #[test]
    fn test_model_tier_assignment() {
        let rec = recommend_team("Build a comprehensive enterprise microservice platform");
        // Director should be Opus
        assert_eq!(rec.coordinators[0].model_tier, ModelTier::Opus);
        // Should have multiple workers
        assert!(rec.workers.len() >= 3);
    }

    #[test]
    fn test_capability_detection() {
        let caps = identify_capabilities(
            "build an api with authentication and database migrations",
            Scope::Backend,
        );
        assert!(caps.contains(&"security".to_string()));
        assert!(caps.contains(&"database".to_string()));
    }
}
