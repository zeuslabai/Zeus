//! Zeus Outcome Templates
//!
//! Reusable goal presets with pre-configured skill wiring, tool policies, and
//! system prompts. A template enriches a user's goal before it reaches the
//! planner, giving agents a head-start on context and constraints.
//!
//! # Usage
//!
//! ```no_run
//! use zeus_templates::TemplateRegistry;
//!
//! let registry = TemplateRegistry::load_builtins();
//! let template = registry.get("debug-rust-crate").unwrap();
//! let applied = registry.apply(&template, "Fix the lifetime error in my parser", &[]);
//! println!("{}", applied.enriched_prompt);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use thiserror::Error;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("Template not found: {0}")]
    NotFound(String),

    #[error("Invalid template YAML: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Template already exists: {0}")]
    AlreadyExists(String),
}

pub type Result<T> = std::result::Result<T, TemplateError>;

// ── Core types ────────────────────────────────────────────────────────────────

/// Which tools an agent may use when executing this template.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolPolicy {
    /// Whitelist of allowed tool names. `None` means all tools are allowed.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,

    /// Tools that must never be called, regardless of `allowed_tools`.
    #[serde(default)]
    pub forbidden_tools: Vec<String>,

    /// Maximum number of shell executions (None = unlimited).
    #[serde(default)]
    pub max_shell_commands: Option<usize>,

    /// Tools that require explicit user approval before execution.
    #[serde(default)]
    pub require_approval_for: Vec<String>,
}

/// Planning parameters passed to the Prometheus planner when applying a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningConfig {
    /// Maximum number of plan steps (default: 8).
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    /// Execution mode: "agent" | "llm" | "simulated".
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,

    /// Allow parallel step execution where dependencies permit.
    #[serde(default)]
    pub parallel_execution: bool,

    /// Automatically replan if a step fails.
    #[serde(default = "default_true")]
    pub auto_replan: bool,

    /// Per-step timeout in milliseconds (default: 120 000 = 2 min).
    #[serde(default = "default_step_timeout")]
    pub step_timeout_ms: u64,

    /// Context sources to inject: "memory", "recent_sessions".
    #[serde(default)]
    pub include_context: Vec<String>,
}

impl Default for PlanningConfig {
    fn default() -> Self {
        Self {
            max_steps: default_max_steps(),
            execution_mode: default_execution_mode(),
            parallel_execution: false,
            auto_replan: true,
            step_timeout_ms: default_step_timeout(),
            include_context: vec![],
        }
    }
}

fn default_max_steps() -> usize {
    8
}
fn default_execution_mode() -> String {
    "agent".to_string()
}
fn default_step_timeout() -> u64 {
    120_000
}
fn default_true() -> bool {
    true
}

/// A reusable goal preset that enriches an agent's context before planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeTemplate {
    // ── Identity ─────────────────────────────────────────────────────────────
    /// Unique slug (e.g. "debug-rust-crate"). URL-safe, kebab-case.
    pub id: String,

    /// Human-readable display name.
    pub name: String,

    /// One-line summary shown in template listings.
    pub description: String,

    // ── Organisation ─────────────────────────────────────────────────────────
    /// Broad categories for discovery (e.g. "programming", "writing").
    #[serde(default)]
    pub categories: Vec<String>,

    /// Fine-grained tags for filtering (e.g. "rust", "cargo", "tests").
    #[serde(default)]
    pub tags: Vec<String>,

    // ── Execution config ──────────────────────────────────────────────────────
    /// System prompt injected alongside the user's goal.
    pub system_prompt: String,

    /// Skill IDs that must be loaded before execution.
    #[serde(default)]
    pub required_skills: Vec<String>,

    /// Provider categories required (e.g. ["image_gen", "voice"]).
    /// The onboarding flow can gate template use until these are configured.
    #[serde(default)]
    pub required_providers: Vec<String>,

    /// Tool access policy for the execution.
    #[serde(default)]
    pub tool_policy: ToolPolicy,

    /// Planning parameters.
    #[serde(default)]
    pub planning_config: PlanningConfig,

    // ── Guidance ──────────────────────────────────────────────────────────────
    /// Example user goals for this template.
    #[serde(default)]
    pub examples: Vec<String>,

    /// What the user should have when execution is complete.
    #[serde(default)]
    pub expected_outcome: String,

    /// Criteria used to evaluate success.
    #[serde(default)]
    pub success_criteria: Vec<String>,

    // ── Metadata ──────────────────────────────────────────────────────────────
    /// Template author (e.g. "zeus" for built-ins, or a user name).
    #[serde(default)]
    pub author: Option<String>,

    /// SemVer version string.
    #[serde(default = "default_version")]
    pub version: String,

    /// Whether this is a built-in template (cannot be deleted by users).
    #[serde(default)]
    pub builtin: bool,

    /// How many times this template has been applied.
    #[serde(default)]
    pub usage_count: u64,

    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,

    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// The result of applying a template to a user goal. Returned by
/// `POST /v1/templates/:id/apply` before the user confirms execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedTemplate {
    /// The template that was applied.
    pub template_id: String,

    /// Original user goal string.
    pub user_goal: String,

    /// Enriched prompt combining template system_prompt + user goal +
    /// expected_outcome context. Pass this to the Prometheus planner.
    pub enriched_prompt: String,

    /// Required providers that are NOT yet configured.
    /// Non-empty means execution will likely fail until these are wired up.
    pub missing_providers: Vec<String>,

    /// Required skills that are NOT currently installed.
    pub missing_skills: Vec<String>,

    /// Summary of the tool policy that will be enforced.
    pub tool_policy: ToolPolicy,

    /// Planning config that will be used.
    pub planning_config: PlanningConfig,
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Built-in template YAML files bundled into the binary.
const BUILTIN_YAMLS: &[(&str, &str)] = &[
    ("write-blog-post", include_str!("../templates/write-blog-post.yaml")),
    ("debug-rust-crate", include_str!("../templates/debug-rust-crate.yaml")),
    ("code-review", include_str!("../templates/code-review.yaml")),
    ("research-topic", include_str!("../templates/research-topic.yaml")),
    ("refactor-codebase", include_str!("../templates/refactor-codebase.yaml")),
    ("create-unit-tests", include_str!("../templates/create-unit-tests.yaml")),
    ("deploy-service", include_str!("../templates/deploy-service.yaml")),
    ("generate-image", include_str!("../templates/generate-image.yaml")),
];

/// In-memory template registry. Thread-safe via `Arc<RwLock<…>>`.
///
/// Built-ins are loaded at startup. Users can add, update, and delete their
/// own templates (persisted to `~/.zeus/templates/`). Built-in templates
/// cannot be deleted.
#[derive(Clone)]
pub struct TemplateRegistry {
    inner: Arc<RwLock<HashMap<String, OutcomeTemplate>>>,
    /// Directory for user-created templates. None = no persistence.
    store_path: Option<PathBuf>,
}

impl TemplateRegistry {
    /// Create a registry pre-loaded with all built-in templates.
    pub fn load_builtins() -> Self {
        let mut map = HashMap::new();
        for (id, yaml) in BUILTIN_YAMLS {
            match serde_yaml::from_str::<OutcomeTemplate>(yaml) {
                Ok(mut t) => {
                    t.builtin = true;
                    map.insert(id.to_string(), t);
                }
                Err(e) => {
                    tracing::error!("Failed to parse built-in template '{}': {}", id, e);
                }
            }
        }
        tracing::info!("Loaded {} built-in templates", map.len());
        Self {
            inner: Arc::new(RwLock::new(map)),
            store_path: None,
        }
    }

    /// Create a registry with built-ins + user templates from disk.
    pub fn load(store_path: PathBuf) -> Self {
        let mut registry = Self::load_builtins();
        registry.store_path = Some(store_path.clone());

        if store_path.is_dir() {
            match std::fs::read_dir(&store_path) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                            continue;
                        }
                        match std::fs::read_to_string(&path) {
                            Ok(yaml) => match serde_yaml::from_str::<OutcomeTemplate>(&yaml) {
                                Ok(t) => {
                                    if let Ok(mut map) = registry.inner.write() {
                                        tracing::debug!("Loaded user template: {}", t.id);
                                        map.insert(t.id.clone(), t);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Skipping malformed template {}: {}",
                                        path.display(),
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                tracing::warn!(
                                    "Could not read template file {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Could not read templates dir {}: {}", store_path.display(), e);
                }
            }
        }
        registry
    }

    // ── Read operations ───────────────────────────────────────────────────────

    /// Get a single template by ID.
    pub fn get(&self, id: &str) -> Option<OutcomeTemplate> {
        self.inner.read().ok()?.get(id).cloned()
    }

    /// List all templates, optionally filtered by category.
    pub fn list(&self, category: Option<&str>) -> Vec<OutcomeTemplate> {
        let map = match self.inner.read() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut templates: Vec<OutcomeTemplate> = map
            .values()
            .filter(|t| {
                category
                    .map(|c| t.categories.iter().any(|cat| cat == c))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        templates.sort_by(|a, b| a.name.cmp(&b.name));
        templates
    }

    /// Search templates by keyword (name, description, tags).
    pub fn search(&self, query: &str) -> Vec<OutcomeTemplate> {
        let q = query.to_lowercase();
        let map = match self.inner.read() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut results: Vec<OutcomeTemplate> = map
            .values()
            .filter(|t| {
                t.name.to_lowercase().contains(&q)
                    || t.description.to_lowercase().contains(&q)
                    || t.tags.iter().any(|tag| tag.to_lowercase().contains(&q))
                    || t.categories.iter().any(|cat| cat.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// List all unique category names across all templates.
    pub fn categories(&self) -> Vec<String> {
        let map = match self.inner.read() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut cats: Vec<String> = map
            .values()
            .flat_map(|t| t.categories.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        cats
    }

    // ── Write operations ──────────────────────────────────────────────────────

    /// Create a new user-defined template. Returns an error if the ID already
    /// exists, or if the template is missing required fields.
    pub fn create(&self, mut template: OutcomeTemplate) -> Result<()> {
        {
            let map = self
                .inner
                .read()
                .map_err(|_| TemplateError::ParseError("registry lock poisoned".into()))?;
            if map.contains_key(&template.id) {
                return Err(TemplateError::AlreadyExists(template.id.clone()));
            }
        }
        let now = Utc::now();
        template.created_at = now;
        template.updated_at = now;
        template.builtin = false;

        self.persist(&template)?;

        let mut map = self
            .inner
            .write()
            .map_err(|_| TemplateError::ParseError("registry lock poisoned".into()))?;
        map.insert(template.id.clone(), template);
        Ok(())
    }

    /// Update an existing user-defined template. Built-ins cannot be updated.
    pub fn update(&self, id: &str, mut template: OutcomeTemplate) -> Result<()> {
        let is_builtin = self
            .inner
            .read()
            .ok()
            .and_then(|m| m.get(id).map(|t| t.builtin))
            .unwrap_or(false);

        if is_builtin {
            return Err(TemplateError::ParseError(format!(
                "Cannot modify built-in template '{}'",
                id
            )));
        }

        template.id = id.to_string();
        template.updated_at = Utc::now();
        template.builtin = false;

        self.persist(&template)?;

        let mut map = self
            .inner
            .write()
            .map_err(|_| TemplateError::ParseError("registry lock poisoned".into()))?;
        if !map.contains_key(id) {
            return Err(TemplateError::NotFound(id.to_string()));
        }
        map.insert(id.to_string(), template);
        Ok(())
    }

    /// Delete a user-defined template. Built-ins cannot be deleted.
    pub fn delete(&self, id: &str) -> Result<()> {
        let is_builtin = self
            .inner
            .read()
            .ok()
            .and_then(|m| m.get(id).map(|t| t.builtin))
            .unwrap_or(false);

        if is_builtin {
            return Err(TemplateError::ParseError(format!(
                "Cannot delete built-in template '{}'",
                id
            )));
        }

        // Remove from disk
        if let Some(ref path) = self.store_path {
            let file = path.join(format!("{}.yaml", id));
            if file.exists() {
                std::fs::remove_file(&file)?;
            }
        }

        let mut map = self
            .inner
            .write()
            .map_err(|_| TemplateError::ParseError("registry lock poisoned".into()))?;
        if map.remove(id).is_none() {
            return Err(TemplateError::NotFound(id.to_string()));
        }
        Ok(())
    }

    // ── Apply ─────────────────────────────────────────────────────────────────

    /// Apply a template to a user goal.
    ///
    /// Returns an `AppliedTemplate` with the enriched prompt and a list of
    /// any missing providers/skills that need to be configured before the
    /// goal can be executed successfully.
    ///
    /// The `configured_providers` parameter is the set of provider categories
    /// already configured by the user (e.g. `["llm", "image_gen"]`).
    pub fn apply(
        &self,
        template: &OutcomeTemplate,
        user_goal: &str,
        configured_providers: &[String],
    ) -> AppliedTemplate {
        // Build enriched prompt
        let enriched_prompt = build_enriched_prompt(template, user_goal);

        // Check which required providers are missing
        let missing_providers: Vec<String> = template
            .required_providers
            .iter()
            .filter(|p| !configured_providers.contains(p))
            .cloned()
            .collect();

        // Increment usage count (best-effort, ignore lock errors)
        if let Ok(mut map) = self.inner.write()
            && let Some(t) = map.get_mut(&template.id)
        {
            t.usage_count += 1;
        }

        AppliedTemplate {
            template_id: template.id.clone(),
            user_goal: user_goal.to_string(),
            enriched_prompt,
            missing_providers,
            missing_skills: template.required_skills.clone(),
            tool_policy: template.tool_policy.clone(),
            planning_config: template.planning_config.clone(),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn persist(&self, template: &OutcomeTemplate) -> Result<()> {
        if let Some(ref path) = self.store_path {
            std::fs::create_dir_all(path)?;
            let file = path.join(format!("{}.yaml", template.id));
            let yaml = serde_yaml::to_string(template)
                .map_err(|e| TemplateError::ParseError(e.to_string()))?;
            std::fs::write(file, yaml)?;
        }
        Ok(())
    }
}

/// Build the enriched prompt that agents receive when a template is applied.
fn build_enriched_prompt(template: &OutcomeTemplate, user_goal: &str) -> String {
    let mut parts = vec![template.system_prompt.trim().to_string()];

    if !template.expected_outcome.is_empty() {
        parts.push(format!(
            "\n## Expected Outcome\n{}",
            template.expected_outcome
        ));
    }

    if !template.success_criteria.is_empty() {
        let criteria = template
            .success_criteria
            .iter()
            .map(|c| format!("- {}", c))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("\n## Success Criteria\n{}", criteria));
    }

    parts.push(format!("\n## User Goal\n{}", user_goal));

    parts.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtins() {
        let registry = TemplateRegistry::load_builtins();
        let templates = registry.list(None);
        assert_eq!(templates.len(), 8, "expected 8 built-in templates");
    }

    #[test]
    fn test_get_builtin() {
        let registry = TemplateRegistry::load_builtins();
        let t = registry.get("debug-rust-crate").expect("should exist");
        assert_eq!(t.id, "debug-rust-crate");
        assert!(t.builtin);
        assert!(!t.system_prompt.is_empty());
    }

    #[test]
    fn test_list_by_category() {
        let registry = TemplateRegistry::load_builtins();
        let rust = registry.list(Some("rust"));
        assert!(!rust.is_empty());
        for t in &rust {
            assert!(t.categories.contains(&"rust".to_string()));
        }
    }

    #[test]
    fn test_search() {
        let registry = TemplateRegistry::load_builtins();
        let results = registry.search("blog");
        assert!(!results.is_empty());
        assert!(results.iter().any(|t| t.id == "write-blog-post"));
    }

    #[test]
    fn test_categories() {
        let registry = TemplateRegistry::load_builtins();
        let cats = registry.categories();
        assert!(!cats.is_empty());
        assert!(cats.contains(&"programming".to_string()));
        assert!(cats.contains(&"writing".to_string()));
    }

    #[test]
    fn test_apply_no_missing_providers() {
        let registry = TemplateRegistry::load_builtins();
        let t = registry.get("debug-rust-crate").unwrap();
        let applied = registry.apply(&t, "Fix the lifetime error in my parser", &[]);
        assert!(applied.missing_providers.is_empty());
        assert!(applied.enriched_prompt.contains("Fix the lifetime error"));
        assert!(applied.enriched_prompt.contains("Rust"));
    }

    #[test]
    fn test_apply_missing_provider() {
        let registry = TemplateRegistry::load_builtins();
        let t = registry.get("generate-image").unwrap();
        // image_gen not configured
        let applied = registry.apply(&t, "A mountain at sunset", &[]);
        assert!(applied.missing_providers.contains(&"image_gen".to_string()));
    }

    #[test]
    fn test_apply_provider_configured() {
        let registry = TemplateRegistry::load_builtins();
        let t = registry.get("generate-image").unwrap();
        let applied = registry.apply(&t, "A mountain at sunset", &["image_gen".to_string()]);
        assert!(applied.missing_providers.is_empty());
    }

    #[test]
    fn test_delete_builtin_rejected() {
        let registry = TemplateRegistry::load_builtins();
        let err = registry.delete("debug-rust-crate").unwrap_err();
        assert!(matches!(err, TemplateError::ParseError(_)));
    }

    #[test]
    fn test_create_and_delete_user_template() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TemplateRegistry::load(dir.path().to_path_buf());

        let t = OutcomeTemplate {
            id: "test-template".to_string(),
            name: "Test".to_string(),
            description: "A test template".to_string(),
            categories: vec!["test".to_string()],
            tags: vec![],
            system_prompt: "Do the test.".to_string(),
            required_skills: vec![],
            required_providers: vec![],
            tool_policy: ToolPolicy::default(),
            planning_config: PlanningConfig::default(),
            examples: vec![],
            expected_outcome: String::new(),
            success_criteria: vec![],
            author: None,
            version: "1.0.0".to_string(),
            builtin: false,
            usage_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        registry.create(t).unwrap();
        assert!(registry.get("test-template").is_some());

        registry.delete("test-template").unwrap();
        assert!(registry.get("test-template").is_none());
    }

    #[test]
    fn test_usage_count_increments() {
        let registry = TemplateRegistry::load_builtins();
        let t = registry.get("code-review").unwrap();
        assert_eq!(t.usage_count, 0);

        registry.apply(&t, "Review my auth module", &[]);
        let t2 = registry.get("code-review").unwrap();
        assert_eq!(t2.usage_count, 1);
    }
}
