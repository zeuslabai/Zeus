//! Plan Mode — file-based planning for complex tasks.
//!
//! When a Titan receives a complex task, it enters plan mode:
//! 1. Creates a plan folder under `~/.zeus/workspace/plans/{slug}/`
//! 2. LLM writes PLAN.md (full step-by-step breakdown, no tool execution)
//! 3. Cooking loop executes the plan step by step
//! 4. STATUS.md is updated after each iteration
//! 5. OUTCOME.md is written on completion
//!
//! Plans are resume-safe: if interrupted, heartbeat calls `find_incomplete_plans()`
//! and resumes from the last completed step.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Status of a plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    /// Plan created, not yet started
    Planning,
    /// LLM is writing the plan
    Writing,
    /// Plan written, executing steps
    Executing,
    /// All steps completed successfully
    Completed,
    /// Plan failed (some steps failed)
    Failed,
    /// Plan was interrupted (can be resumed)
    Interrupted,
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Planning => write!(f, "PLANNING"),
            Self::Writing => write!(f, "WRITING"),
            Self::Executing => write!(f, "IN_PROGRESS"),
            Self::Completed => write!(f, "COMPLETED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Interrupted => write!(f, "INTERRUPTED"),
        }
    }
}

/// Metadata for a plan, serialized as frontmatter in PLAN.md
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanMeta {
    /// Unique plan ID (used as folder name)
    pub slug: String,
    /// Original task/request that triggered the plan
    pub task: String,
    /// Titan that created/owns the plan
    pub titan: String,
    /// When the plan was created
    pub created: DateTime<Utc>,
    /// Current status
    pub status: PlanStatus,
    /// Total steps (populated after plan is written)
    pub total_steps: usize,
    /// Steps completed so far
    pub steps_completed: usize,
    /// Steps failed
    pub steps_failed: usize,
}

/// Manages the plan folder lifecycle for a single plan.
pub struct PlanMode {
    /// Root plans directory (e.g. ~/.zeus/workspace/plans/)
    plans_root: PathBuf,
    /// This plan's folder
    plan_dir: PathBuf,
    /// Plan metadata
    meta: PlanMeta,
}

impl PlanMode {
    /// Create a new plan mode instance and the plan folder on disk.
    pub async fn create(
        workspace_root: &Path,
        task: &str,
        titan_name: &str,
    ) -> Result<Self, String> {
        let plans_root = workspace_root.join("plans");
        let slug = Self::generate_slug(task);
        let plan_dir = plans_root.join(&slug);

        // Create directory
        tokio::fs::create_dir_all(&plan_dir)
            .await
            .map_err(|e| format!("Failed to create plan dir {}: {}", plan_dir.display(), e))?;

        let meta = PlanMeta {
            slug: slug.clone(),
            task: task.to_string(),
            titan: titan_name.to_string(),
            created: Utc::now(),
            status: PlanStatus::Planning,
            total_steps: 0,
            steps_completed: 0,
            steps_failed: 0,
        };

        let pm = Self {
            plans_root,
            plan_dir,
            meta,
        };

        // Write initial STATUS.md
        pm.write_status().await?;

        info!("Plan mode: created {}", pm.plan_dir.display());
        Ok(pm)
    }

    /// Load an existing plan from disk (for resume).
    pub async fn load(workspace_root: &Path, slug: &str) -> Result<Self, String> {
        let plans_root = workspace_root.join("plans");
        let plan_dir = plans_root.join(slug);

        if !plan_dir.exists() {
            return Err(format!("Plan dir not found: {}", plan_dir.display()));
        }

        let status_path = plan_dir.join("STATUS.md");
        if !status_path.exists() {
            return Err(format!("STATUS.md not found in {}", plan_dir.display()));
        }

        let content = tokio::fs::read_to_string(&status_path)
            .await
            .map_err(|e| format!("Failed to read STATUS.md: {}", e))?;

        let meta = Self::parse_status(&content, slug)?;

        Ok(Self {
            plans_root,
            plan_dir,
            meta,
        })
    }

    /// Generate a slug from a task description.
    /// Format: `YYYY-MM-DD-first-few-words`
    fn generate_slug(task: &str) -> String {
        let date = Utc::now().format("%Y-%m-%d");
        let words: String = task
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 2)
            .take(5)
            .collect::<Vec<_>>()
            .join("-");
        let words = if words.is_empty() { "task".to_string() } else { words };
        // Truncate to keep filesystem-friendly
        let words: String = words.chars().take(50).collect();
        format!("{}-{}", date, words)
    }

    /// Write the LLM-generated plan to PLAN.md.
    pub async fn write_plan(&mut self, plan_content: &str) -> Result<(), String> {
        self.meta.status = PlanStatus::Executing;

        // Count steps (lines starting with checkbox pattern)
        self.meta.total_steps = plan_content
            .lines()
            .filter(|l| {
                let trimmed = l.trim();
                trimmed.starts_with("- [ ]")
                    || trimmed.starts_with("- [x]")
                    || trimmed.starts_with("- [X]")
            })
            .count();

        // If no checkbox steps found, count numbered list items
        if self.meta.total_steps == 0 {
            self.meta.total_steps = plan_content
                .lines()
                .filter(|l| {
                    let trimmed = l.trim();
                    trimmed.len() > 2
                        && trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                        && trimmed.contains('.')
                })
                .count();
        }

        let full_content = format!(
            "# Plan: {}\n\
             \n\
             - **Titan:** {}\n\
             - **Created:** {}\n\
             - **Status:** {}\n\
             - **Steps:** {}\n\
             \n\
             ---\n\
             \n\
             {}",
            self.meta.task,
            self.meta.titan,
            self.meta.created.to_rfc3339(),
            self.meta.status,
            self.meta.total_steps,
            plan_content,
        );

        let plan_path = self.plan_dir.join("PLAN.md");
        tokio::fs::write(&plan_path, &full_content)
            .await
            .map_err(|e| format!("Failed to write PLAN.md: {}", e))?;

        self.write_status().await?;

        info!(
            "Plan mode: wrote PLAN.md ({} steps) at {}",
            self.meta.total_steps,
            plan_path.display()
        );
        Ok(())
    }

    /// Read the plan content from PLAN.md.
    pub async fn read_plan(&self) -> Result<String, String> {
        let plan_path = self.plan_dir.join("PLAN.md");
        tokio::fs::read_to_string(&plan_path)
            .await
            .map_err(|e| format!("Failed to read PLAN.md: {}", e))
    }

    /// Update step progress after a cooking iteration.
    pub async fn update_progress(
        &mut self,
        steps_completed: usize,
        steps_failed: usize,
    ) -> Result<(), String> {
        self.meta.steps_completed = steps_completed;
        self.meta.steps_failed = steps_failed;

        if self.meta.steps_completed + self.meta.steps_failed >= self.meta.total_steps
            && self.meta.total_steps > 0
        {
            self.meta.status = if self.meta.steps_failed > 0 {
                PlanStatus::Failed
            } else {
                PlanStatus::Completed
            };
        }

        self.write_status().await
    }

    /// Mark the plan as interrupted (for heartbeat resume).
    pub async fn mark_interrupted(&mut self) -> Result<(), String> {
        self.meta.status = PlanStatus::Interrupted;
        self.write_status().await
    }

    /// Mark the plan as completed and write OUTCOME.md.
    pub async fn complete(&mut self, summary: &str) -> Result<(), String> {
        self.meta.status = PlanStatus::Completed;
        self.write_status().await?;
        self.write_outcome(summary).await
    }

    /// Mark the plan as failed and write OUTCOME.md.
    pub async fn fail(&mut self, reason: &str) -> Result<(), String> {
        self.meta.status = PlanStatus::Failed;
        self.write_status().await?;
        self.write_outcome(&format!("FAILED: {}", reason)).await
    }

    /// Write STATUS.md with current metadata.
    async fn write_status(&self) -> Result<(), String> {
        let content = format!(
            "slug: {}\n\
             task: {}\n\
             titan: {}\n\
             created: {}\n\
             status: {}\n\
             total_steps: {}\n\
             steps_completed: {}\n\
             steps_failed: {}\n",
            self.meta.slug,
            self.meta.task,
            self.meta.titan,
            self.meta.created.to_rfc3339(),
            self.meta.status,
            self.meta.total_steps,
            self.meta.steps_completed,
            self.meta.steps_failed,
        );

        let status_path = self.plan_dir.join("STATUS.md");
        tokio::fs::write(&status_path, &content)
            .await
            .map_err(|e| format!("Failed to write STATUS.md: {}", e))?;
        Ok(())
    }

    /// Write OUTCOME.md with the final summary.
    async fn write_outcome(&self, summary: &str) -> Result<(), String> {
        let content = format!(
            "# Outcome: {}\n\
             \n\
             - **Titan:** {}\n\
             - **Status:** {}\n\
             - **Steps:** {}/{} completed, {} failed\n\
             - **Started:** {}\n\
             - **Finished:** {}\n\
             \n\
             ---\n\
             \n\
             {}",
            self.meta.task,
            self.meta.titan,
            self.meta.status,
            self.meta.steps_completed,
            self.meta.total_steps,
            self.meta.steps_failed,
            self.meta.created.to_rfc3339(),
            Utc::now().to_rfc3339(),
            summary,
        );

        let outcome_path = self.plan_dir.join("OUTCOME.md");
        tokio::fs::write(&outcome_path, &content)
            .await
            .map_err(|e| format!("Failed to write OUTCOME.md: {}", e))?;

        info!("Plan mode: wrote OUTCOME.md at {}", outcome_path.display());
        Ok(())
    }

    /// Parse STATUS.md content into PlanMeta.
    fn parse_status(content: &str, slug: &str) -> Result<PlanMeta, String> {
        let get = |key: &str| -> Option<String> {
            content.lines().find_map(|l| {
                l.strip_prefix(key)
                    .map(|v| v.trim().to_string())
            })
        };

        let task = get("task: ").unwrap_or_default();
        let titan = get("titan: ").unwrap_or_default();
        let created = get("created: ")
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let status = match get("status: ").as_deref() {
            Some("PLANNING") => PlanStatus::Planning,
            Some("WRITING") => PlanStatus::Writing,
            Some("IN_PROGRESS") => PlanStatus::Executing,
            Some("COMPLETED") => PlanStatus::Completed,
            Some("FAILED") => PlanStatus::Failed,
            Some("INTERRUPTED") => PlanStatus::Interrupted,
            _ => PlanStatus::Interrupted,
        };
        let total_steps = get("total_steps: ")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let steps_completed = get("steps_completed: ")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let steps_failed = get("steps_failed: ")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        Ok(PlanMeta {
            slug: slug.to_string(),
            task,
            titan,
            created,
            status,
            total_steps,
            steps_completed,
            steps_failed,
        })
    }

    /// Find all incomplete plans (for heartbeat resume).
    /// Returns slugs of plans that are Executing or Interrupted.
    pub async fn find_incomplete(workspace_root: &Path) -> Vec<String> {
        let plans_dir = workspace_root.join("plans");
        if !plans_dir.exists() {
            return Vec::new();
        }

        let mut incomplete = Vec::new();
        let mut entries = match tokio::fs::read_dir(&plans_dir).await {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read plans dir: {}", e);
                return Vec::new();
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }

            let slug = entry.file_name().to_string_lossy().to_string();
            let status_path = entry.path().join("STATUS.md");

            if let Ok(content) = tokio::fs::read_to_string(&status_path).await {
                if let Ok(meta) = Self::parse_status(&content, &slug) {
                    match meta.status {
                        // Interrupted plans are always resumable — they were
                        // explicitly interrupted (crash, cancel, new message).
                        PlanStatus::Interrupted => {
                            debug!("Found interrupted plan: {}", slug);
                            incomplete.push(slug);
                        }
                        // Executing plans are only resumable if stale (>5 min).
                        // Fresh Executing plans are actively being cooked by the
                        // gateway — heartbeat should NOT interfere. This prevents
                        // the infinite resume loop where conversational messages
                        // create short-lived plans that heartbeat immediately resumes.
                        PlanStatus::Executing => {
                            let age = chrono::Utc::now().signed_duration_since(meta.created);
                            if age.num_seconds() > 300 {
                                debug!("Found stale executing plan: {} ({}s old)", slug, age.num_seconds());
                                incomplete.push(slug);
                            } else {
                                debug!("Skipping fresh executing plan: {} ({}s old, <300s)", slug, age.num_seconds());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        incomplete
    }

    /// Get the plan directory path.
    pub fn plan_dir(&self) -> &Path {
        &self.plan_dir
    }

    /// Get plan metadata.
    pub fn meta(&self) -> &PlanMeta {
        &self.meta
    }

    /// Build the system prompt injection for plan-guided cooking.
    pub fn plan_context_prompt(&self, plan_content: &str) -> String {
        format!(
            "\n\n[PLAN MODE — ACTIVE]\n\
             You are executing a plan step by step. Follow the plan below precisely.\n\
             After completing each step, note which step you just finished.\n\
             Do NOT skip steps or change the order unless a step fails and requires adaptation.\n\
             \n\
             Current progress: {}/{} steps completed ({} failed)\n\
             \n\
             {}\n\
             \n\
             [END PLAN]",
            self.meta.steps_completed,
            self.meta.total_steps,
            self.meta.steps_failed,
            plan_content,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();
        (dir, workspace)
    }

    #[tokio::test]
    async fn test_generate_slug() {
        let slug = PlanMode::generate_slug("Set up NGINX with VNET jails");
        assert!(slug.contains("set-up-nginx-with-vnet"));
        assert!(slug.starts_with("20")); // starts with year
    }

    #[tokio::test]
    async fn test_generate_slug_short_input() {
        let slug = PlanMode::generate_slug("fix bug");
        assert!(slug.contains("fix-bug"));
    }

    #[tokio::test]
    async fn test_generate_slug_empty() {
        let slug = PlanMode::generate_slug("");
        assert!(slug.contains("task"));
    }

    #[tokio::test]
    async fn test_generate_slug_special_chars() {
        let slug = PlanMode::generate_slug("Deploy v2.0 @staging! #urgent");
        assert!(!slug.contains('@'));
        assert!(!slug.contains('#'));
        assert!(!slug.contains('!'));
    }

    #[tokio::test]
    async fn test_create_plan_folder() {
        let (_dir, workspace) = setup().await;
        let pm = PlanMode::create(&workspace, "Set up NGINX", "Zeus107").await.unwrap();

        assert!(pm.plan_dir().exists());
        assert!(pm.plan_dir().join("STATUS.md").exists());
        assert_eq!(pm.meta().status, PlanStatus::Planning);
        assert_eq!(pm.meta().titan, "Zeus107");
        assert_eq!(pm.meta().task, "Set up NGINX");
    }

    #[tokio::test]
    async fn test_write_plan() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Deploy app", "Zeus100").await.unwrap();

        let plan = "## Steps\n\
                     - [ ] Build release binary\n\
                     - [ ] Run tests\n\
                     - [ ] Deploy to staging\n";

        pm.write_plan(plan).await.unwrap();

        assert_eq!(pm.meta().total_steps, 3);
        assert_eq!(pm.meta().status, PlanStatus::Executing);
        assert!(pm.plan_dir().join("PLAN.md").exists());

        let content = pm.read_plan().await.unwrap();
        assert!(content.contains("Build release binary"));
        assert!(content.contains("Deploy app"));
    }

    #[tokio::test]
    async fn test_write_plan_numbered_steps() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Migrate DB", "Zeus112").await.unwrap();

        let plan = "## Steps\n\
                     1. Backup database\n\
                     2. Run migration\n\
                     3. Verify data\n\
                     4. Update config\n";

        pm.write_plan(plan).await.unwrap();
        assert_eq!(pm.meta().total_steps, 4);
    }

    #[tokio::test]
    async fn test_update_progress() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Build feature", "Zeus106").await.unwrap();

        let plan = "- [ ] Step 1\n- [ ] Step 2\n- [ ] Step 3\n";
        pm.write_plan(plan).await.unwrap();

        pm.update_progress(2, 0).await.unwrap();
        assert_eq!(pm.meta().steps_completed, 2);
        assert_eq!(pm.meta().status, PlanStatus::Executing);

        pm.update_progress(3, 0).await.unwrap();
        assert_eq!(pm.meta().status, PlanStatus::Completed);
    }

    #[tokio::test]
    async fn test_update_progress_with_failures() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Risky task", "Zeus107").await.unwrap();

        let plan = "- [ ] Step 1\n- [ ] Step 2\n- [ ] Step 3\n";
        pm.write_plan(plan).await.unwrap();

        pm.update_progress(2, 1).await.unwrap();
        assert_eq!(pm.meta().status, PlanStatus::Failed);
    }

    #[tokio::test]
    async fn test_complete_writes_outcome() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Ship feature", "Zeus100").await.unwrap();

        pm.complete("All steps passed. Feature deployed.").await.unwrap();

        assert_eq!(pm.meta().status, PlanStatus::Completed);
        assert!(pm.plan_dir().join("OUTCOME.md").exists());

        let outcome = tokio::fs::read_to_string(pm.plan_dir().join("OUTCOME.md"))
            .await
            .unwrap();
        assert!(outcome.contains("All steps passed"));
        assert!(outcome.contains("COMPLETED"));
    }

    #[tokio::test]
    async fn test_fail_writes_outcome() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Deploy DB", "Zeus112").await.unwrap();

        pm.fail("Migration script crashed").await.unwrap();

        assert_eq!(pm.meta().status, PlanStatus::Failed);
        let outcome = tokio::fs::read_to_string(pm.plan_dir().join("OUTCOME.md"))
            .await
            .unwrap();
        assert!(outcome.contains("FAILED"));
        assert!(outcome.contains("Migration script crashed"));
    }

    #[tokio::test]
    async fn test_mark_interrupted() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Long task", "Zeus100").await.unwrap();
        let plan = "- [ ] Step 1\n- [ ] Step 2\n";
        pm.write_plan(plan).await.unwrap();

        pm.mark_interrupted().await.unwrap();
        assert_eq!(pm.meta().status, PlanStatus::Interrupted);
    }

    #[tokio::test]
    async fn test_load_existing_plan() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Resumable task", "Zeus107").await.unwrap();
        let plan = "- [ ] Step 1\n- [ ] Step 2\n- [ ] Step 3\n";
        pm.write_plan(plan).await.unwrap();
        pm.update_progress(1, 0).await.unwrap();

        let slug = pm.meta().slug.clone();

        // Load from disk
        let loaded = PlanMode::load(&workspace, &slug).await.unwrap();
        assert_eq!(loaded.meta().task, "Resumable task");
        assert_eq!(loaded.meta().titan, "Zeus107");
        assert_eq!(loaded.meta().total_steps, 3);
        assert_eq!(loaded.meta().steps_completed, 1);
        assert_eq!(loaded.meta().status, PlanStatus::Executing);
    }

    #[tokio::test]
    async fn test_find_incomplete_plans() {
        let (_dir, workspace) = setup().await;

        // Create a plan in progress
        let mut pm1 = PlanMode::create(&workspace, "Task one", "Zeus100").await.unwrap();
        pm1.write_plan("- [ ] Step 1\n").await.unwrap();

        // Create a completed plan
        let mut pm2 = PlanMode::create(&workspace, "Task two", "Zeus112").await.unwrap();
        pm2.complete("Done").await.unwrap();

        // Create an interrupted plan
        let mut pm3 = PlanMode::create(&workspace, "Task three", "Zeus107").await.unwrap();
        pm3.write_plan("- [ ] Step 1\n").await.unwrap();
        pm3.mark_interrupted().await.unwrap();

        let incomplete = PlanMode::find_incomplete(&workspace).await;
        assert_eq!(incomplete.len(), 2); // pm1 (Executing) + pm3 (Interrupted)
    }

    #[tokio::test]
    async fn test_find_incomplete_no_plans_dir() {
        let dir = TempDir::new().unwrap();
        let incomplete = PlanMode::find_incomplete(dir.path()).await;
        assert!(incomplete.is_empty());
    }

    #[tokio::test]
    async fn test_plan_context_prompt() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Test task", "Zeus100").await.unwrap();
        pm.write_plan("- [ ] Step 1\n- [ ] Step 2\n").await.unwrap();
        pm.update_progress(1, 0).await.unwrap();

        let plan_content = pm.read_plan().await.unwrap();
        let prompt = pm.plan_context_prompt(&plan_content);

        assert!(prompt.contains("[PLAN MODE — ACTIVE]"));
        assert!(prompt.contains("1/2 steps completed"));
        assert!(prompt.contains("Step 1"));
    }

    #[tokio::test]
    async fn test_load_nonexistent_plan() {
        let dir = TempDir::new().unwrap();
        let result = PlanMode::load(dir.path(), "nonexistent-plan").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plan_status_display() {
        assert_eq!(PlanStatus::Planning.to_string(), "PLANNING");
        assert_eq!(PlanStatus::Executing.to_string(), "IN_PROGRESS");
        assert_eq!(PlanStatus::Completed.to_string(), "COMPLETED");
        assert_eq!(PlanStatus::Failed.to_string(), "FAILED");
        assert_eq!(PlanStatus::Interrupted.to_string(), "INTERRUPTED");
    }

    #[tokio::test]
    async fn test_parse_status_roundtrip() {
        let (_dir, workspace) = setup().await;
        let mut pm = PlanMode::create(&workspace, "Roundtrip test", "Zeus100").await.unwrap();
        pm.write_plan("- [ ] A\n- [ ] B\n- [ ] C\n").await.unwrap();
        pm.update_progress(2, 0).await.unwrap();

        let content = tokio::fs::read_to_string(pm.plan_dir().join("STATUS.md"))
            .await
            .unwrap();
        let parsed = PlanMode::parse_status(&content, &pm.meta().slug).unwrap();

        assert_eq!(parsed.task, "Roundtrip test");
        assert_eq!(parsed.titan, "Zeus100");
        assert_eq!(parsed.total_steps, 3);
        assert_eq!(parsed.steps_completed, 2);
        assert_eq!(parsed.status, PlanStatus::Executing);
    }
}
