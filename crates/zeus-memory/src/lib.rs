//! Zeus Memory - Workspace file-based memory
//!
//! Target: ~300 lines

pub mod indexer;
pub use indexer::{FileEntry, FileIndex, FileType, IndexStats, SearchResult};

use chrono::Local;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::debug;
use zeus_core::{Error, Result};

// ============================================================================
// Workspace (~250 lines)
// ============================================================================

/// File-based workspace for memory and context
#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create a new workspace at the given root
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create workspace from config
    pub fn from_config(config: &zeus_core::Config) -> Self {
        Self::new(&config.workspace)
    }

    /// Initialize workspace with default files if they don't exist
    pub async fn init(&self) -> Result<()> {
        // Create directories
        fs::create_dir_all(&self.root).await?;
        fs::create_dir_all(self.root.join("memory")).await?;
        fs::create_dir_all(self.root.join("daily")).await?;
        fs::create_dir_all(self.root.join("goals")).await?;

        // Create default files if they don't exist
        self.ensure_file("AGENTS.md", DEFAULT_AGENTS).await?;
        self.ensure_file("SOUL.md", DEFAULT_SOUL).await?;
        self.ensure_file("USER.md", DEFAULT_USER).await?;
        self.ensure_file("HEARTBEAT.md", DEFAULT_HEARTBEAT).await?;
        self.ensure_file("memory/MEMORY.md", DEFAULT_MEMORY).await?;

        Ok(())
    }

    /// Ensure a file exists with default content
    async fn ensure_file(&self, path: &str, default: &str) -> Result<()> {
        let full_path = self.root.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        // Atomic create: only writes if the file doesn't exist (O_CREAT|O_EXCL)
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full_path)
            .await
        {
            Ok(mut f) => {
                use tokio::io::AsyncWriteExt;
                f.write_all(default.as_bytes()).await?;
                debug!("Created default file: {}", full_path.display());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // File already exists — nothing to do
            }
            Err(e) => return Err(Error::Memory(format!("Failed to create {}: {}", path, e))),
        }
        Ok(())
    }

    // ========================================================================
    // Core file operations
    // ========================================================================

    /// Validate that a path stays within the workspace root.
    /// Prevents path traversal attacks using `../` sequences and symlinks.
    async fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let full_path = self.root.join(path);

        // Normalize the path by resolving . and .. components lexically.
        // We can't use canonicalize because the file may not exist yet.
        let mut normalized = PathBuf::new();
        for component in full_path.components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                other => normalized.push(other),
            }
        }

        if !normalized.starts_with(&self.root) {
            return Err(Error::Memory(format!(
                "Path traversal denied: '{}' escapes workspace",
                path
            )));
        }

        // Defense-in-depth: if the target exists, canonicalize to detect symlinks
        // pointing outside the workspace
        if tokio::fs::try_exists(&normalized).await.unwrap_or(false)
            && let Ok(canonical) = tokio::fs::canonicalize(&normalized).await
        {
            let canonical_root = tokio::fs::canonicalize(&self.root)
                .await
                .unwrap_or_else(|_| self.root.clone());
            if !canonical.starts_with(&canonical_root) {
                return Err(Error::Memory(format!(
                    "Path traversal denied: '{}' resolves outside workspace via symlink",
                    path
                )));
            }
        }

        Ok(normalized)
    }

    /// Read a file from the workspace
    pub async fn read(&self, path: &str) -> Result<String> {
        let full_path = self.validate_path(path).await?;
        fs::read_to_string(&full_path)
            .await
            .map_err(|e| Error::Memory(format!("Failed to read {}: {}", path, e)))
    }

    /// Write a file to the workspace
    pub async fn write(&self, path: &str, content: &str) -> Result<()> {
        let full_path = self.validate_path(path).await?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&full_path, content).await?;
        Ok(())
    }

    /// Append a side-effect entry to `memory/RECENT_ACTIVITY.md` and trim
    /// to the last 200 lines on every write. Used by the agent cooking loop
    /// to record outbound tool calls (`message`, `send_file`, …) so that
    /// other channel sessions for the same titan can see recent activity
    /// in their next `get_context()` render.
    ///
    /// Format of `entry` is the caller's responsibility; convention is
    /// `- [YYYY-MM-DDTHH:MM:SSZ] [channel:<type>:<chat_id> user:<id>] tool:<name> → <summary>`
    pub async fn append_recent_activity(&self, entry: &str) -> Result<()> {
        const PATH: &str = "memory/RECENT_ACTIVITY.md";
        const MAX_LINES: usize = 200;

        let full_path = self.validate_path(PATH).await?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let existing = match fs::read_to_string(&full_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::Memory(format!("Failed to read {}: {}", PATH, e))),
        };

        // Append entry, then trim to last MAX_LINES lines.
        let combined = if existing.trim().is_empty() {
            entry.trim_end().to_string()
        } else {
            format!("{}\n{}", existing.trim_end(), entry.trim_end())
        };

        let trimmed: String = {
            let lines: Vec<&str> = combined.lines().collect();
            if lines.len() > MAX_LINES {
                lines[lines.len() - MAX_LINES..].join("\n")
            } else {
                combined.clone()
            }
        };

        fs::write(&full_path, format!("{}\n", trimmed)).await?;
        Ok(())
    }

    /// Append a mention entry to `memory/MENTIONS.md` and trim to the last
    /// 200 lines on every write. Used by the agent cooking loop to record
    /// inbound mentions of this titan from any channel, so that other
    /// channel sessions for the same titan can surface those mentions in
    /// their next `get_context()` render (mention-gated cross-channel
    /// awareness, paired with `append_recent_activity` for outbound
    /// side-effect logging).
    ///
    /// Format of `entry` is the caller's responsibility; convention is
    /// `- [YYYY-MM-DDTHH:MM:SSZ] [channel:<type>:<chat_id> from:<user_id>] <mention_body>`
    pub async fn append_mention(&self, entry: &str) -> Result<()> {
        const PATH: &str = "memory/MENTIONS.md";
        const MAX_LINES: usize = 200;

        let full_path = self.validate_path(PATH).await?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let existing = match fs::read_to_string(&full_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::Memory(format!("Failed to read {}: {}", PATH, e))),
        };

        // Append entry, then trim to last MAX_LINES lines.
        let combined = if existing.trim().is_empty() {
            entry.trim_end().to_string()
        } else {
            format!("{}\n{}", existing.trim_end(), entry.trim_end())
        };

        let trimmed: String = {
            let lines: Vec<&str> = combined.lines().collect();
            if lines.len() > MAX_LINES {
                lines[lines.len() - MAX_LINES..].join("\n")
            } else {
                combined.clone()
            }
        };

        fs::write(&full_path, format!("{}\n", trimmed)).await?;
        Ok(())
    }

    /// Append to a file in the workspace
    pub async fn append(&self, path: &str, content: &str) -> Result<()> {
        let full_path = self.validate_path(path).await?;
        // Try-read: avoids blocking exists() check; treats NotFound as empty file
        let existing = match fs::read_to_string(&full_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::Memory(format!("Failed to read {}: {}", path, e))),
        };

        let new_content = if existing.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", existing.trim_end(), content)
        };

        self.write(path, &new_content).await
    }

    /// Check if a file exists in the workspace
    pub async fn exists(&self, path: &str) -> bool {
        match self.validate_path(path).await {
            Ok(full_path) => tokio::fs::try_exists(&full_path).await.unwrap_or(false),
            Err(_) => false,
        }
    }

    // ========================================================================
    // High-level memory operations
    // ========================================================================

    /// Get the system prompt from AGENTS.md
    pub async fn get_agents(&self) -> Result<String> {
        self.read("AGENTS.md").await
    }

    /// Get the personality/soul from SOUL.md
    pub async fn get_soul(&self) -> Result<String> {
        self.read("SOUL.md").await
    }

    /// Get user context from USER.md
    pub async fn get_user(&self) -> Result<String> {
        self.read("USER.md").await
    }

    /// Parse structured heartbeat tasks from the `## tasks` section of HEARTBEAT.md.
    /// Returns tasks with individual names, intervals, and prompts.
    pub async fn get_structured_tasks(&self) -> Result<Vec<StructuredHeartbeatTask>> {
        let content = self.get_heartbeat().await?;
        Ok(parse_structured_tasks(&content))
    }

    /// Get heartbeat/proactive tasks from HEARTBEAT.md
    pub async fn get_heartbeat(&self) -> Result<String> {
        self.read("HEARTBEAT.md").await
    }

    /// Parse heartbeat tasks by frequency (daily, weekly, etc.)
    pub async fn get_heartbeat_tasks(&self, frequency: &str) -> Result<Vec<String>> {
        let content = self.get_heartbeat().await?;
        let mut tasks = Vec::new();
        let mut in_section = false;
        let freq_lower = frequency.to_lowercase();

        for line in content.lines() {
            // Check for section headers like "## Daily" or "## Weekly"
            if let Some(stripped) = line.strip_prefix("## ") {
                let section = stripped.trim().to_lowercase();
                in_section = section == freq_lower;
                continue;
            }

            // Collect task items in the matching section
            if in_section && line.starts_with("- ") {
                let task = line[2..].trim().to_string();
                if !task.is_empty() {
                    tasks.push(task);
                }
            }
        }

        Ok(tasks)
    }

    /// Get the current assigned task from HEARTBEAT.md `## CURRENT TASK` section.
    /// Returns None if the section is empty or contains the default placeholder.
    pub async fn get_current_task(&self) -> Result<Option<String>> {
        let content = self.get_heartbeat().await?;
        let task = extract_heartbeat_section(&content, "CURRENT TASK");
        if task.is_empty()
            || task.contains("Coordinator will assign")
            || task.contains("(no task)")
        {
            Ok(None)
        } else {
            Ok(Some(task))
        }
    }

    /// Set the CURRENT TASK in HEARTBEAT.md, preserving all other sections.
    ///
    /// Used by the gateway's auto-task-detection to persist an assigned task
    /// so the agent has it in context on the next heartbeat.
    ///
    /// The task string is written verbatim under `## CURRENT TASK`. Pass an
    /// empty string to clear the current task (it will fall back to the
    /// default "(Coordinator will assign your task here.)" placeholder).
    pub async fn set_current_task(&self, task: &str) -> Result<()> {
        let content = self.get_heartbeat().await?;
        let queue_section = extract_heartbeat_section(&content, "TASK QUEUE");
        let completed_section = extract_heartbeat_section(&content, "COMPLETED");
        let daily = extract_heartbeat_section(&content, "Daily");
        let weekly = extract_heartbeat_section(&content, "Weekly");

        let trimmed = task.trim();
        let mut new_content = String::from("# Heartbeat Tasks\n\n");

        // CURRENT TASK
        new_content.push_str("## CURRENT TASK\n");
        if trimmed.is_empty() {
            new_content.push_str("(Coordinator will assign your task here.)\n");
        } else {
            new_content.push_str(trimmed);
            new_content.push('\n');
        }
        new_content.push('\n');

        // TASK QUEUE — preserve
        new_content.push_str("## TASK QUEUE\n");
        if queue_section.is_empty() {
            new_content.push_str("(Pending tasks will be listed here.)\n");
        } else {
            new_content.push_str(&queue_section);
            if !queue_section.ends_with('\n') {
                new_content.push('\n');
            }
        }
        new_content.push('\n');

        // COMPLETED — preserve
        new_content.push_str("## COMPLETED\n");
        if !completed_section.is_empty()
            && !completed_section.contains("Completed tasks are moved")
        {
            new_content.push_str(&completed_section);
            if !completed_section.ends_with('\n') {
                new_content.push('\n');
            }
        }
        new_content.push('\n');

        if !daily.is_empty() {
            new_content.push_str("## Daily\n");
            new_content.push_str(&daily);
            new_content.push_str("\n\n");
        }

        if !weekly.is_empty() {
            new_content.push_str("## Weekly\n");
            new_content.push_str(&weekly);
            new_content.push('\n');
        }

        self.write("HEARTBEAT.md", &new_content).await?;
        Ok(())
    }

    /// Append a task to the TASK QUEUE section of HEARTBEAT.md.
    /// Used when CURRENT TASK is already occupied and a new task is detected.
    pub async fn append_to_task_queue(&self, task: &str) -> Result<()> {
        let content = self.get_heartbeat().await?;
        let current_section = extract_heartbeat_section(&content, "CURRENT TASK");
        let queue_section = extract_heartbeat_section(&content, "TASK QUEUE");
        let completed_section = extract_heartbeat_section(&content, "COMPLETED");
        let daily = extract_heartbeat_section(&content, "Daily");
        let weekly = extract_heartbeat_section(&content, "Weekly");

        let trimmed = task.trim();
        let mut new_content = String::from("# Heartbeat Tasks\n\n");

        // CURRENT TASK — preserve existing
        new_content.push_str("## CURRENT TASK\n");
        if current_section.is_empty() {
            new_content.push_str("(Coordinator will assign your task here.)\n");
        } else {
            new_content.push_str(&current_section);
            if !current_section.ends_with('\n') {
                new_content.push('\n');
            }
        }
        new_content.push('\n');

        // TASK QUEUE — append new item
        new_content.push_str("## TASK QUEUE\n");
        // Preserve existing queue items
        let existing_queue = queue_section
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.contains("Pending tasks will be"))
            .collect::<Vec<_>>()
            .join("\n");
        if !existing_queue.is_empty() {
            new_content.push_str(&existing_queue);
            new_content.push('\n');
        }
        // Append the new task
        if !trimmed.is_empty() {
            new_content.push_str(&format!("- [ ] {}\n", trimmed));
        }
        new_content.push('\n');

        // COMPLETED — preserve
        new_content.push_str("## COMPLETED\n");
        if !completed_section.is_empty()
            && !completed_section.contains("Completed tasks are moved")
        {
            new_content.push_str(&completed_section);
            if !completed_section.ends_with('\n') {
                new_content.push('\n');
            }
        }
        new_content.push('\n');

        if !daily.is_empty() {
            new_content.push_str("## Daily\n");
            new_content.push_str(&daily);
            new_content.push_str("\n\n");
        }

        if !weekly.is_empty() {
            new_content.push_str("## Weekly\n");
            new_content.push_str(&weekly);
            new_content.push('\n');
        }

        self.write("HEARTBEAT.md", &new_content).await?;
        Ok(())
    }

    /// Get pending tasks from HEARTBEAT.md `## TASK QUEUE` section.
    /// Returns unchecked items (lines starting with `- [ ]`).
    pub async fn get_task_queue(&self) -> Result<Vec<String>> {
        let content = self.get_heartbeat().await?;
        let section = extract_heartbeat_section(&content, "TASK QUEUE");
        let tasks: Vec<String> = section
            .lines()
            .filter(|line| line.starts_with("- [ ]") || line.starts_with("- "))
            .map(|line| {
                line.trim_start_matches("- [ ] ")
                    .trim_start_matches("- ")
                    .trim()
                    .to_string()
            })
            .filter(|t| !t.is_empty() && !t.contains("Pending tasks will be"))
            .collect();
        Ok(tasks)
    }

    /// Advance the task queue: move CURRENT TASK to COMPLETED, pop next from QUEUE.
    /// Returns the new current task if one was promoted, None if queue is empty.
    pub async fn advance_task_queue(&self) -> Result<Option<String>> {
        let content = self.get_heartbeat().await?;
        let current = extract_heartbeat_section(&content, "CURRENT TASK");
        let queue_section = extract_heartbeat_section(&content, "TASK QUEUE");
        let completed_section = extract_heartbeat_section(&content, "COMPLETED");

        // Parse queue items
        let mut queue_items: Vec<String> = queue_section
            .lines()
            .filter(|l| l.starts_with("- [ ]") || l.starts_with("- "))
            .map(|l| l.trim_start_matches("- [ ] ").trim_start_matches("- ").trim().to_string())
            .filter(|t| !t.is_empty() && !t.contains("Pending tasks will be"))
            .collect();

        // Build new HEARTBEAT.md
        let mut new_content = String::from("# Heartbeat Tasks\n\n");

        // CURRENT TASK: promote next from queue or set empty
        new_content.push_str("## CURRENT TASK\n");
        let promoted = if !queue_items.is_empty() {
            let next = queue_items.remove(0);
            new_content.push_str(&next);
            new_content.push('\n');
            Some(next)
        } else {
            new_content.push_str("(Coordinator will assign your task here.)\n");
            None
        };
        new_content.push('\n');

        // TASK QUEUE: remaining items
        new_content.push_str("## TASK QUEUE\n");
        if queue_items.is_empty() {
            new_content.push_str("(Pending tasks will be listed here.)\n");
        } else {
            for item in &queue_items {
                new_content.push_str(&format!("- [ ] {}\n", item));
            }
        }
        new_content.push('\n');

        // COMPLETED: add the old current task
        new_content.push_str("## COMPLETED\n");
        if !current.is_empty()
            && !current.contains("Coordinator will assign")
            && !current.contains("(no task)")
        {
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            new_content.push_str(&format!("- [x] {} ({})\n", current.trim(), date));
        }
        if !completed_section.is_empty()
            && !completed_section.contains("Completed tasks are moved")
        {
            new_content.push_str(&completed_section);
            if !completed_section.ends_with('\n') {
                new_content.push('\n');
            }
        }
        new_content.push('\n');

        // Preserve Daily/Weekly sections
        let daily = extract_heartbeat_section(&content, "Daily");
        if !daily.is_empty() {
            new_content.push_str("## Daily\n");
            new_content.push_str(&daily);
            new_content.push_str("\n\n");
        }

        let weekly = extract_heartbeat_section(&content, "Weekly");
        if !weekly.is_empty() {
            new_content.push_str("## Weekly\n");
            new_content.push_str(&weekly);
            new_content.push('\n');
        }

        // Write updated HEARTBEAT.md
        self.write("HEARTBEAT.md", &new_content).await?;

        Ok(promoted)
    }

    /// Parse goals from HEARTBEAT.md `## Goals` section.
    /// Returns structured goal items for Prometheus prioritization.
    pub async fn get_goals(&self) -> Result<Vec<String>> {
        let content = self.get_heartbeat().await?;
        let mut goals = Vec::new();
        let mut in_section = false;

        for line in content.lines() {
            if let Some(stripped) = line.strip_prefix("## ") {
                in_section = stripped.trim().eq_ignore_ascii_case("goals");
                continue;
            }
            if in_section {
                if line.starts_with("## ") { break; } // next section
                if line.starts_with("- ") {
                    let goal = line[2..].trim().to_string();
                    if !goal.is_empty() {
                        goals.push(goal);
                    }
                }
            }
        }
        Ok(goals)
    }

    /// Mark a heartbeat task as completed (adds to daily note)
    pub async fn complete_heartbeat_task(&self, task: &str) -> Result<()> {
        let timestamp = Local::now().format("%H:%M");
        let entry = format!("\n- [{}] Heartbeat: {}", timestamp, task);
        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = format!("daily/{}.md", today);
        self.append(&path, &entry).await
    }

    /// Get long-term memory from MEMORY.md
    pub async fn get_memory(&self) -> Result<String> {
        self.read("memory/MEMORY.md").await
    }

    /// Remember a fact (append to MEMORY.md)
    pub async fn remember(&self, fact: &str) -> Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M");
        let entry = format!("\n- [{}] {}", timestamp, fact);
        self.append("memory/MEMORY.md", &entry).await
    }

    /// Get or create today's daily note
    pub async fn get_daily(&self) -> Result<String> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = format!("daily/{}.md", today);

        if !self.exists(&path).await {
            let header = format!("# {}\n\n## Notes\n\n", today);
            self.write(&path, &header).await?;
        }

        self.read(&path).await
    }

    /// Add a note to today's daily note
    pub async fn note(&self, content: &str) -> Result<()> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = format!("daily/{}.md", today);
        let timestamp = Local::now().format("%H:%M");
        let entry = format!("\n- [{}] {}", timestamp, content);
        self.append(&path, &entry).await
    }

    // ========================================================================
    // Context building
    // ========================================================================

    // ========================================================================
    // Context modes (dev / review / research)
    // ========================================================================

    /// Get the active context mode name, if any.
    /// Reads from `contexts/.active` marker file.
    pub async fn get_active_context(&self) -> Option<String> {
        match self.read("contexts/.active").await {
            Ok(content) => {
                let name = content.trim().to_string();
                if name.is_empty() { None } else { Some(name) }
            }
            Err(_) => None,
        }
    }

    /// Set the active context mode (e.g., "dev", "review", "research").
    /// Pass `None` to clear the active context.
    pub async fn set_context_mode(&self, mode: Option<&str>) -> Result<()> {
        match mode {
            Some(name) => {
                // Verify the context file exists
                let ctx_path = format!("contexts/{}.md", name);
                if !self.exists(&ctx_path).await {
                    return Err(Error::Memory(format!(
                        "Context file not found: {}. Available contexts are in contexts/",
                        ctx_path
                    )));
                }
                self.write("contexts/.active", name).await
            }
            None => {
                // Remove the active marker
                let full_path = self.root.join("contexts/.active");
                let _ = fs::remove_file(&full_path).await;
                Ok(())
            }
        }
    }

    /// List available context modes from `contexts/` directory.
    pub async fn list_context_modes(&self) -> Vec<String> {
        match self.list("contexts").await {
            Ok(files) => files
                .iter()
                .filter(|f| f.ends_with(".md"))
                .map(|f| f.trim_end_matches(".md").to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    // ========================================================================
    // Context building
    // ========================================================================

    /// Get combined context for the LLM system prompt
    pub async fn get_context(&self) -> Result<String> {
        let mut context = String::new();

        // Workspace location — absolute path so ALL models know where files are.
        // Prevents relative path failures on Ollama/Qwen/GLM models.
        let workspace_path = self.root.display();
        let zeus_home = self.root.parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.zeus".to_string());
        context.push_str(&format!(
            "# File System Context\n\n\
             Your workspace is at: {}\n\
             Zeus home directory: {}\n\
             Config file: {}/config.toml\n\
             Sessions directory: {}/sessions/\n\
             When using read_file, write_file, list_dir, or shell tools, always use ABSOLUTE paths.\n\
             Do NOT use relative paths like 'memory/MEMORY.md' — use '{}/memory/MEMORY.md' instead.\n\n",
            workspace_path, zeus_home, zeus_home, zeus_home, workspace_path
        ));

        // AGENTS.md (system prompt)
        if let Ok(agents) = self.get_agents().await {
            context.push_str(&agents);
            context.push_str("\n\n");
        }

        // SOUL.md (personality)
        if let Ok(soul) = self.get_soul().await {
            context.push_str("# Personality\n\n");
            context.push_str(&soul);
            context.push_str("\n\n");
        }

        // USER.md (user context)
        if let Ok(user) = self.get_user().await {
            context.push_str("# User Context\n\n");
            context.push_str(&user);
            context.push_str("\n\n");
        }

        // IDENTITY.md (fleet role, capabilities)
        if let Ok(identity) = self.read("IDENTITY.md").await {
            if !identity.trim().is_empty() {
                context.push_str("# Identity\n\n");
                context.push_str(&identity);
                context.push_str("\n\n");
            }
        }

        // USER.md (human profile — who you're helping)
        if let Ok(user) = self.read("USER.md").await {
            if !user.trim().is_empty() {
                context.push_str("# Your Human\n\n");
                context.push_str(&user);
                context.push_str("\n\n");
            }
        }

        // TOOLS.md (local environment notes — cameras, SSH hosts, preferences)
        if let Ok(tools) = self.read("TOOLS.md").await {
            if !tools.trim().is_empty() {
                context.push_str("# Local Tools & Environment\n\n");
                context.push_str(&tools);
                context.push_str("\n\n");
            }
        }

        // HEARTBEAT.md (proactive tasks)
        if let Ok(heartbeat) = self.get_heartbeat().await {
            if !heartbeat.trim().is_empty() {
                context.push_str("# Proactive Tasks\n\n");
                context.push_str(&heartbeat);
                context.push_str("\n\n");
            }
        }

        // CAPABILITIES.md (auto-generated tool/channel listing)
        if let Ok(caps) = self.read("CAPABILITIES.md").await {
            if !caps.trim().is_empty() {
                context.push_str("# Capabilities\n\n");
                context.push_str(&caps);
                context.push_str("\n\n");
            }
        }

        // MEMORY.md (long-term facts)
        if let Ok(memory) = self.get_memory().await {
            context.push_str("# Memory\n\n");
            context.push_str(&memory);
            context.push_str("\n\n");
        }

        // RECENT_ACTIVITY.md (cross-channel awareness — side-effect tool log)
        if let Ok(recent) = self.read("memory/RECENT_ACTIVITY.md").await {
            if !recent.trim().is_empty() {
                context.push_str("# Recent Activity\n\n");
                context.push_str(&recent);
                context.push_str("\n\n");
            }
        }

        // MENTIONS.md (mention-gated cross-channel awareness — inbound @-mentions)
        if let Ok(mentions) = self.read("memory/MENTIONS.md").await {
            if !mentions.trim().is_empty() {
                context.push_str("# Mentions\n\n");
                context.push_str(&mentions);
                context.push_str("\n\n");
            }
        }

        // Active context mode (dev / review / research)
        if let Some(mode) = self.get_active_context().await {
            let ctx_path = format!("contexts/{}.md", mode);
            if let Ok(mode_content) = self.read(&ctx_path).await {
                context.push_str(&format!("# Active Context: {}\n\n", mode));
                context.push_str(&mode_content);
                context.push_str("\n\n");
            }
        }

        Ok(context)
    }

    /// Files that contribute to `get_context()` output, in the order they are read.
    /// Kept here so the mtime-hash helper below stays in sync with `get_context`.
    const CONTEXT_FILES: &'static [&'static str] = &[
        "AGENTS.md",
        "SOUL.md",
        "USER.md",
        "IDENTITY.md",
        "TOOLS.md",
        "HEARTBEAT.md",
        "CAPABILITIES.md",
        "memory/MEMORY.md",
        "memory/RECENT_ACTIVITY.md",
        "memory/MENTIONS.md",
    ];

    /// Compute a deterministic u64 hash of the mtimes of every file that
    /// `get_context()` reads. Intended as a cache key for the base system
    /// prompt in the agent loop — if this hash hasn't changed since the
    /// last call, the rendered context is guaranteed byte-for-byte equal
    /// to what `get_context()` would produce (under the common assumption
    /// that mtime changes when content changes).
    ///
    /// Missing files contribute a zero to the hash rather than being
    /// skipped, so adding a new file or deleting an existing one does
    /// flip the hash. Errors reading metadata are also treated as zero.
    pub async fn get_context_mtime_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        for name in Self::CONTEXT_FILES {
            let path = self.root.join(name);
            let nanos: u128 = match fs::metadata(&path).await {
                Ok(meta) => match meta.modified() {
                    Ok(mtime) => match mtime.duration_since(std::time::UNIX_EPOCH) {
                        Ok(dur) => dur.as_nanos(),
                        Err(_) => 0,
                    },
                    Err(_) => 0,
                },
                Err(_) => 0,
            };
            // Hash the filename alongside the mtime so identical mtimes
            // on different files don't collide, and so re-ordering the
            // list in a future refactor flips the hash as expected.
            name.hash(&mut hasher);
            nanos.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Get the workspace root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    // ========================================================================
    // Per-agent workspace (T3 / T8b support)
    // ========================================================================

    /// Get a workspace scoped to a specific agent label.
    /// Root: `{workspace}/agents/{label}/`
    /// Falls back to global workspace files if agent-specific files don't exist.
    pub fn for_agent(&self, label: &str) -> AgentWorkspace {
        AgentWorkspace {
            agent_root: self.root.join("agents").join(label),
            global: self.clone(),
            label: label.to_string(),
        }
    }

    /// Write a file into an agent's workspace subdirectory.
    /// Creates `{workspace}/agents/{label}/{filename}`.
    pub async fn write_agent_file(
        &self,
        label: &str,
        filename: &str,
        content: &str,
    ) -> Result<()> {
        let agent_dir = self.root.join("agents").join(label);
        fs::create_dir_all(&agent_dir).await?;
        let path = format!("agents/{}/{}", label, filename);
        self.write(&path, content).await
    }

    /// Read a file from an agent's workspace, falling back to global workspace.
    pub async fn read_agent_file(&self, label: &str, filename: &str) -> Result<String> {
        let agent_path = format!("agents/{}/{}", label, filename);
        match self.read(&agent_path).await {
            Ok(content) => Ok(content),
            Err(_) => self.read(filename).await, // fallback to global
        }
    }

    /// List all agent workspace directories.
    pub async fn list_agents(&self) -> Result<Vec<String>> {
        let agents_dir = self.root.join("agents");
        if !tokio::fs::try_exists(&agents_dir).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        let mut agents = Vec::new();
        let mut dir = fs::read_dir(&agents_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            if entry.file_type().await?.is_dir()
                && let Some(name) = entry.file_name().to_str() {
                    agents.push(name.to_string());
                }
        }
        agents.sort();
        Ok(agents)
    }

    /// Initialize a new agent's workspace with identity and optional heartbeat.
    pub async fn init_agent(
        &self,
        label: &str,
        identity_content: &str,
        heartbeat_content: Option<&str>,
    ) -> Result<()> {
        let agent_dir = self.root.join("agents").join(label);
        fs::create_dir_all(agent_dir.join("memory")).await?;

        self.write_agent_file(label, "IDENTITY.md", identity_content)
            .await?;

        if let Some(hb) = heartbeat_content {
            self.write_agent_file(label, "HEARTBEAT.md", hb).await?;
        }

        // Create empty memory file for the agent
        let mem_path = format!("agents/{}/memory/MEMORY.md", label);
        self.ensure_file(&mem_path, "# Agent Memory\n\n").await?;

        debug!("Initialized agent workspace: agents/{}/", label);
        Ok(())
    }

    /// Remove an agent's workspace (archive, not delete).
    pub async fn archive_agent(&self, label: &str) -> Result<()> {
        let agent_dir = self.root.join("agents").join(label);
        let archive_dir = self.root.join("agents").join(format!(".archived-{}", label));
        if tokio::fs::try_exists(&agent_dir).await.unwrap_or(false) {
            fs::rename(&agent_dir, &archive_dir).await?;
            debug!("Archived agent workspace: agents/{}/", label);
        }
        Ok(())
    }

    /// List all files in a directory
    pub async fn list(&self, path: &str) -> Result<Vec<String>> {
        let full_path = self.validate_path(path).await?;
        let mut entries = Vec::new();

        let mut dir = fs::read_dir(&full_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }

        entries.sort();
        Ok(entries)
    }
}

// ============================================================================
// Per-Agent Workspace
// ============================================================================

/// A workspace scoped to a specific agent, with fallback to global workspace.
/// Root: `{workspace}/agents/{label}/`
pub struct AgentWorkspace {
    agent_root: PathBuf,
    global: Workspace,
    label: String,
}

impl AgentWorkspace {
    /// Get the agent label
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the agent workspace root path
    pub fn root(&self) -> &Path {
        &self.agent_root
    }

    /// Get agent's IDENTITY.md (no fallback — agent-specific only)
    pub async fn get_identity(&self) -> Result<String> {
        self.global
            .read(&format!("agents/{}/IDENTITY.md", self.label))
            .await
    }

    /// Get agent's SOUL.md, falling back to global SOUL.md
    pub async fn get_soul(&self) -> Result<String> {
        self.global
            .read_agent_file(&self.label, "SOUL.md")
            .await
    }

    /// Get agent's HEARTBEAT.md (no fallback — agent-specific only)
    pub async fn get_heartbeat(&self) -> Result<String> {
        self.global
            .read(&format!("agents/{}/HEARTBEAT.md", self.label))
            .await
    }

    /// Get agent's memory
    pub async fn get_memory(&self) -> Result<String> {
        self.global
            .read_agent_file(&self.label, "memory/MEMORY.md")
            .await
    }

    /// Build combined context for this agent (identity + soul + memory + global AGENTS.md)
    pub async fn get_context(&self) -> Result<String> {
        let mut context = String::new();

        // Global AGENTS.md (system prompt — shared)
        if let Ok(agents) = self.global.get_agents().await {
            context.push_str(&agents);
            context.push_str("\n\n");
        }

        // Agent-specific IDENTITY.md
        if let Ok(identity) = self.get_identity().await {
            context.push_str("# Agent Identity\n\n");
            context.push_str(&identity);
            context.push_str("\n\n");
        }

        // SOUL.md (agent-specific with global fallback)
        if let Ok(soul) = self.get_soul().await {
            context.push_str("# Personality\n\n");
            context.push_str(&soul);
            context.push_str("\n\n");
        }

        // USER.md (who the human is — agent-specific with global fallback)
        if let Ok(user) = self.read("USER.md").await {
            if !user.trim().is_empty() {
                context.push_str("# Your Human\n\n");
                context.push_str(&user);
                context.push_str("\n\n");
            }
        }

        // HEARTBEAT.md (proactive tasks)
        if let Ok(hb) = self.get_heartbeat().await {
            if !hb.trim().is_empty() {
                context.push_str("# Proactive Tasks\n\n");
                context.push_str(&hb);
                context.push_str("\n\n");
            }
        }

        // TOOLS.md (local environment notes)
        if let Ok(tools) = self.read("TOOLS.md").await {
            if !tools.trim().is_empty() {
                context.push_str("# Local Tools & Environment\n\n");
                context.push_str(&tools);
                context.push_str("\n\n");
            }
        }

        // CAPABILITIES.md (auto-generated tool/channel listing)
        if let Ok(caps) = self.read("CAPABILITIES.md").await {
            if !caps.trim().is_empty() {
                context.push_str("# Capabilities\n\n");
                context.push_str(&caps);
                context.push_str("\n\n");
            }
        }

        // Agent memory
        if let Ok(memory) = self.get_memory().await {
            context.push_str("# Memory\n\n");
            context.push_str(&memory);
            context.push_str("\n\n");
        }

        Ok(context)
    }

    /// Remember a fact in the agent's memory
    pub async fn remember(&self, fact: &str) -> Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M");
        let entry = format!("\n- [{}] {}", timestamp, fact);
        let path = format!("agents/{}/memory/MEMORY.md", self.label);
        self.global.append(&path, &entry).await
    }

    /// Write a file to the agent's workspace
    pub async fn write(&self, filename: &str, content: &str) -> Result<()> {
        self.global
            .write_agent_file(&self.label, filename, content)
            .await
    }

    /// Read a file from the agent's workspace (with global fallback)
    pub async fn read(&self, filename: &str) -> Result<String> {
        self.global.read_agent_file(&self.label, filename).await
    }
}

// ============================================================================
// Heartbeat section parser
// ============================================================================

/// Extract content under a `## SECTION_NAME` header from HEARTBEAT.md.
/// Returns everything between the header and the next `## ` header (or EOF).
fn extract_heartbeat_section(content: &str, section_name: &str) -> String {
    let header = format!("## {}", section_name);
    let mut in_section = false;
    let mut lines = Vec::new();

    for line in content.lines() {
        if line.starts_with("## ") {
            if line.trim() == header || line.starts_with(&header) {
                in_section = true;
                continue;
            } else if in_section {
                break; // hit the next section
            }
        }
        if in_section {
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}

// ============================================================================
// Default file contents
// ============================================================================

const DEFAULT_AGENTS: &str = r#"# Zeus — Autonomous AI Titan

You are **Zeus**, a full-featured autonomous AI Titan with 218 tools, advanced memory, cognitive reasoning, and multi-channel communication. You run on the user's machine with direct access to the filesystem, shell, web, messaging platforms, macOS automation, and browser control.

## Core Principles

1. **Act, don't narrate** — use tools to accomplish tasks rather than describing what you would do
2. **Be concise** — short, direct responses; let results speak
3. **Think before acting** — for complex tasks, plan first, then execute
4. **Remember and learn** — store important facts in memory, recall past context
5. **Ask when ambiguous** — clarify requirements before making assumptions
6. **Stay safe** — respect Aegis security policies; never bypass permission checks

## Task Management (MANDATORY — Autonomous Operation)

**CRITICAL RULE:** When anyone assigns you a task, your FIRST action — before responding, before planning, before anything — is to persist it:

```
Use write_file to update ~/.zeus/workspace/HEARTBEAT.md:
- Write the task under ## CURRENT TASK
- Include: description, branch name, files to modify
```

This is NOT optional. The heartbeat system reads HEARTBEAT.md to keep you working between messages. If you don't persist the task, you will idle and lose the assignment.

### Task Lifecycle:

1. **Receive assignment** → IMMEDIATELY `write_file` to HEARTBEAT.md CURRENT TASK
   Example: Zeus100 says "fix the IRC bug on branch fix/irc"
   → You write: `Fix IRC DM routing bug. Branch: fix/irc. File: crates/zeus-channels/src/irc.rs`

2. **Work on task** → Use tools (read_file, edit_file, shell, etc.) to complete it

3. **Complete task** → Update HEARTBEAT.md:
   - Move CURRENT TASK to `## COMPLETED` with date
   - Pop next from `## TASK QUEUE` into CURRENT TASK
   - Report completion to coordinator channel with commit hash

4. **Between messages:** Heartbeat reads CURRENT TASK every 5 minutes.
   If it has work, you continue autonomously. If empty, HEARTBEAT_OK.

5. **Status updates:** Report to coordinator channel every 30 minutes.
   Include: what you did, what's next, blockers.

### Never Do:
- Never acknowledge a task without writing it to HEARTBEAT.md first
- Never idle with an empty CURRENT TASK when TASK QUEUE has items
- Never work on something not in your HEARTBEAT.md (persist it first)

## Development Workflow (MANDATORY for code tasks)

**Repository:** the Zeus workspace — Rust workspace with 32 crates.

**Every code task follows this exact sequence:**
1. `cd` to the Zeus workspace root
2. `git checkout dev && git pull origin dev`
3. `git checkout -b fix/your-fix-name` (or `feat/...`)
4. Make changes using `read_file`, `edit_file`, `shell`
5. `cargo check` — MUST pass before committing
6. `git add <changed files>` — only add files you changed
7. `git commit -m "fix: description of what and why"`
8. `git push -u origin fix/your-fix-name`
9. Report to Discord: commit hash + branch name + summary

**Rules:**
- Never push directly to `main` — the operator merges `dev → main`
- Always `cargo check` before committing — broken builds block the fleet
- Branch names: `fix/short-description` or `feat/short-description`
- Commit messages: start with `fix:`, `feat:`, or `refactor:`

## Core Tools (14)

| Tool | Purpose |
|------|---------|
| `read_file` | Read file contents |
| `write_file` | Create or overwrite files |
| `edit_file` | Search and replace in files |
| `list_dir` | List directory contents |
| `list_dir` (recursive) | Explore directory trees |
| `shell` | Execute shell commands (respects Aegis command filtering) |
| `web_fetch` | Fetch content from URLs (respects URL allowlist) |
| `spawn` | Launch background sub-Titans for parallel or long-running work |
| `message` | Send messages to channels (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix, file, webhook, console) |
| `link_understanding` | Analyze and summarize web page content |
| `media_understanding` | Analyze images (OCR, describe), audio (transcribe), video |
| `auto_reply` | Configure automatic reply rules for messaging channels |
| `polls` | Create and manage polls across channels |
| `apply_patch` | Apply unified diff patches to files |
| `gmail_pubsub` | Set up Gmail push notifications via Pub/Sub |

## macOS Automation (Talos — 193 tools)

When running on macOS, you have access to Talos automation tools across 22 categories:

- **System**: system_info, process_list, clipboard, screenshot, volume, brightness, wifi, bluetooth, focus modes
- **Files**: file_search, file_copy, file_move, file_rename, find_files
- **Git**: git_status through git_stash_pop (15 tools)
- **Calendar**: list_events, create_event, delete_event, get_today
- **Notes**: list, create, search, read, append (Apple Notes)
- **Reminders**: list, create, complete, list_lists
- **Contacts**: search, get_details, create, update, delete
- **Safari**: open_url, get_url, get_tabs, execute_js, navigate
- **Mail**: read, send, delete, flag, forward, move
- **iMessage**: send, read, list_conversations
- **Music**: play, pause, next, search, now_playing
- **UI Automation**: click, type, scroll, shortcut, screenshot, mouse_position
- **PDF**: extract_text, extract_pages, merge, split, get_metadata
- **Telegram**: send_message, send_photo, send_buttons, get_updates, get_chat_info
- **Network**: diagnostics, ping, port_check
- **Homebrew**: install, uninstall, list, search
- **Defaults**: read/write system preferences
- **Bluetooth**: list, pair, connect, disconnect, power
- **Voice**: speak_text, speech-to-text

## Browser Automation (11 tools)

Chrome DevTools Protocol control for web automation:
navigate, click, type, get_text, screenshot, execute_js, console_logs, network_intercept, performance_metrics, scroll, wait

## Subsystems (auto-injected context)

These run automatically — you don't call them directly:

- **Nous** (Cognitive Engine) — intent recognition and reasoning context injected before each response
- **Mnemosyne** (Memory) — relevant memories from past sessions searched and injected automatically; stores new facts after each interaction
- **Prometheus** (Orchestration) — plans and executes complex multi-step tasks; cooking loop for iterative tool execution
- **Athena** (Documentation) — logs actions, tool executions, and session summaries
- **Aegis** (Security) — validates commands, URLs, and paths before tool execution; sandboxing
- **Hermes** (Notifications) — sends alerts on errors and task completions

## Messaging Channels

The `message` tool supports these platforms (require config in ~/.zeus/config.toml):
- **Telegram** — MTProto via grammers-client
- **Discord** — Serenity gateway + HTTP
- **Slack** — Web API + Socket Mode
- **Email** — SMTP send + IMAP IDLE receive
- **iMessage** — AppleScript bridge (macOS)
- **WhatsApp** — Cloud API
- **Signal** — signal-cli JSON-RPC
- **Matrix** — matrix-sdk native Rust

Simple channels (no config needed): `console`, `file`, `file:/path`, `webhook:URL`

## Sub-Titans

Use `spawn` to run tasks in parallel:
```
spawn(task: "Research the latest Rust async patterns", wait: false)
spawn(task: "Summarize this document", context: "<doc>", wait: true)
```

## Memory

- **Workspace**: files in ~/.zeus/workspace/ (AGENTS.md, SOUL.md, USER.md, MEMORY.md, daily notes)
- **Mnemosyne**: SQLite + FTS5 full-text search + vector embeddings for semantic recall
- Use `shell` to run `zeus memory remember "fact"` to persist important information

## Best Practices

- For file operations: read before editing, use edit_file for targeted changes
- For shell commands: prefer specific commands over broad operations; check results
- For messaging: specify target (chat_id, email) when sending to specific recipients
- For automation: combine Talos tools for complex macOS workflows
- For web tasks: use link_understanding for analysis, web_fetch for raw content
- For parallel work: spawn sub-Titans for independent tasks
- For complex tasks: break into steps, execute sequentially, verify each step
"#;

const DEFAULT_SOUL: &str = r#"## Personality

- Direct and efficient communication
- Curious about problems and solutions
- Admits uncertainty when appropriate
- Focuses on practical outcomes
- Signs off declarations of completion with ⚡

## Real-time Data

When asked about "latest", "current", "newest", or any time-sensitive
information (prices, model versions, API changes, release dates, news),
ALWAYS use web_search to verify before answering. Never answer from
training data alone — it goes stale. Models change, prices change,
APIs change. Search first, cite the source, then answer.
"#;

const DEFAULT_USER: &str = r#"## User Preferences

- Prefers concise responses
- Technical background
- Working on various projects
"#;

// ---------------------------------------------------------------------------
// Structured heartbeat task parsing
// ---------------------------------------------------------------------------

/// A structured heartbeat task with its own name, interval, and prompt.
/// Parsed from the `## tasks` section of HEARTBEAT.md.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredHeartbeatTask {
    /// Short identifier for state tracking / dedup (e.g. "push-work")
    pub name: String,
    /// Execution interval in seconds (parsed from "30s", "5m", "1h", "1d")
    pub interval_secs: u64,
    /// The prompt sent to the LLM when this task fires
    pub prompt: String,
}

/// Map a frequency-style section header (`hourly`/`daily`/`weekly`) to a
/// default interval in seconds. Returns `None` for unknown headers.
fn frequency_default_interval_secs(header: &str) -> Option<u64> {
    match header.trim().to_lowercase().as_str() {
        "hourly" => Some(3600),
        "daily" => Some(86400),
        "weekly" => Some(604800),
        _ => None,
    }
}

/// Parse structured tasks from HEARTBEAT.md content.
///
/// Accepts TWO equivalent forms:
///
/// **Form A — explicit `## tasks` block (canonical):**
/// ```text
/// ## tasks
/// - name: push-work
///   interval: 30m
///   prompt: "Push any uncommitted work"
/// ```
///
/// **Form B — frequency-bullet form (T21 template, fleet default):**
/// ```text
/// ## hourly
/// - First: push any uncommitted work
/// - Then: report what you did to your team channel
/// - Then: continue your CURRENT TASK
/// ```
/// Each bullet under `## hourly` / `## daily` / `## weekly` becomes a task.
/// Names are auto-generated as `<freq>-<n>` (e.g. `hourly-1`). Intervals
/// default to the section's natural cadence (3600 / 86400 / 604800 s).
/// The prompt is the bullet text verbatim.
///
/// Both forms can coexist in a single HEARTBEAT.md.
pub fn parse_structured_tasks(content: &str) -> Vec<StructuredHeartbeatTask> {
    let mut tasks = Vec::new();
    let mut in_tasks_section = false;
    // Current frequency-bullet section (None when not in one).
    let mut bullet_freq: Option<&'static str> = None;
    let mut bullet_default_interval: u64 = 3600;
    let mut bullet_idx: u32 = 0;

    // Current Form-A task being built
    let mut name: Option<String> = None;
    let mut interval: Option<u64> = None;
    let mut prompt: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section boundaries
        if trimmed.starts_with("## ") {
            // Flush any in-progress Form-A task
            if let (Some(n), Some(i), Some(p)) = (name.take(), interval.take(), prompt.take()) {
                tasks.push(StructuredHeartbeatTask { name: n, interval_secs: i, prompt: p });
            }
            let header = trimmed.trim_start_matches("## ").trim();
            in_tasks_section = header.eq_ignore_ascii_case("tasks");

            // Handle frequency-bullet sections (Form B)
            bullet_freq = match header.to_lowercase().as_str() {
                "hourly" => Some("hourly"),
                "daily" => Some("daily"),
                "weekly" => Some("weekly"),
                _ => None,
            };
            if let Some(freq) = bullet_freq {
                bullet_default_interval = frequency_default_interval_secs(freq)
                    .unwrap_or(3600);
                bullet_idx = 0;
            }
            continue;
        }

        // Form B: frequency-bullet sections
        if let Some(freq) = bullet_freq {
            if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
            // Bullet markers: "-", "*", "+"
            let bullet_text = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "));
            if let Some(text) = bullet_text {
                let text = text.trim();
                if text.is_empty() { continue; }
                bullet_idx += 1;
                tasks.push(StructuredHeartbeatTask {
                    name: format!("{}-{}", freq, bullet_idx),
                    interval_secs: bullet_default_interval,
                    prompt: text.to_string(),
                });
            }
            continue;
        }

        if !in_tasks_section { continue; }

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') { continue; }

        // New task entry: "- name: <value>"
        if trimmed.starts_with("- name:") {
            // Flush previous task if complete
            if let (Some(n), Some(i), Some(p)) = (name.take(), interval.take(), prompt.take()) {
                tasks.push(StructuredHeartbeatTask { name: n, interval_secs: i, prompt: p });
            }
            name = Some(trimmed.trim_start_matches("- name:").trim().to_string());
            interval = None;
            prompt = None;
            continue;
        }

        // Indented fields for current task
        if let Some(val) = trimmed.strip_prefix("interval:") {
            interval = Some(parse_duration_str(val.trim()));
        } else if let Some(val) = trimmed.strip_prefix("prompt:") {
            let val = val.trim();
            // Strip surrounding quotes if present
            let unquoted = if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                &val[1..val.len()-1]
            } else {
                val
            };
            prompt = Some(unquoted.to_string());
        }
    }

    // Flush last Form-A task
    if let (Some(n), Some(i), Some(p)) = (name, interval, prompt) {
        tasks.push(StructuredHeartbeatTask { name: n, interval_secs: i, prompt: p });
    }

    tasks
}

/// Parse a human-readable duration string into seconds.
/// Supports: "30s", "5m", "1h", "6h", "1d", plain number (treated as seconds).
pub fn parse_duration_str(s: &str) -> u64 {
    let s = s.trim().to_lowercase();
    if s.is_empty() { return 300; } // default 5 min

    if let Some(n) = s.strip_suffix('d') {
        return n.trim().parse::<u64>().unwrap_or(1) * 86400;
    }
    if let Some(n) = s.strip_suffix('h') {
        return n.trim().parse::<u64>().unwrap_or(1) * 3600;
    }
    if let Some(n) = s.strip_suffix('m') {
        return n.trim().parse::<u64>().unwrap_or(5) * 60;
    }
    if let Some(n) = s.strip_suffix('s') {
        return n.trim().parse::<u64>().unwrap_or(300);
    }

    // Plain number = seconds
    s.parse::<u64>().unwrap_or(300)
}

const DEFAULT_HEARTBEAT: &str = r#"# Heartbeat Tasks

## CURRENT TASK
(Coordinator will assign your task here.)

## TASK QUEUE
(Pending tasks will be listed here.)

## COMPLETED
(Completed tasks are moved here.)

## tasks
# Structured per-task schedule (parsed by the heartbeat loop).
# Each entry specifies its own interval and prompt, so high-priority
# items fire more often than low-priority ones.
#
# Fields:
#   - name:     short identifier (used for state dedup)
#   - interval: `30s`, `5m`, `1h`, `6h`, `1d`
#   - prompt:   what the Titan should do when the task fires
#
# Legacy `## Daily` / `## Weekly` sections below are still honored.
- name: push-work
  interval: 30m
  prompt: "Push any uncommitted work to the current branch. If nothing to push, reply HEARTBEAT_OK."
- name: report
  interval: 1h
  prompt: "Report brief status to your team channel: what you shipped, what's in-flight, what's blocked."
- name: current-task
  interval: 5m
  prompt: "Continue your CURRENT TASK. If empty, pop the top item from TASK QUEUE. If both empty, reply HEARTBEAT_OK."

## Daily
- Check for unfinished tasks
- Report status to coordinator channel

## Weekly
- Review memory for outdated information
"#;

const DEFAULT_MEMORY: &str = r#"# Long-term Memory

Facts and learnings to remember:

"#;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- Workspace creation -------------------------------------------------

    #[test]
    fn test_workspace_new() {
        let ws = Workspace::new("/tmp/test-ws");
        assert_eq!(ws.root(), Path::new("/tmp/test-ws"));
    }

    #[tokio::test]
    async fn test_workspace_init() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        assert!(tmp.path().join("AGENTS.md").exists());
        assert!(tmp.path().join("SOUL.md").exists());
        assert!(tmp.path().join("USER.md").exists());
        assert!(tmp.path().join("HEARTBEAT.md").exists());
        assert!(tmp.path().join("memory/MEMORY.md").exists());
        assert!(tmp.path().join("memory").is_dir());
        assert!(tmp.path().join("daily").is_dir());
    }

    #[tokio::test]
    async fn test_set_current_task_roundtrip() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init");

        // No task initially
        assert!(ws.get_current_task().await.expect("get").is_none());

        // Set a task
        ws.set_current_task("Ship feature X by Friday")
            .await
            .expect("set");

        let task = ws.get_current_task().await.expect("get");
        assert_eq!(task.as_deref().map(str::trim), Some("Ship feature X by Friday"));

        // Overwrite
        ws.set_current_task("New priority task").await.expect("set");
        let task = ws.get_current_task().await.expect("get");
        assert_eq!(task.as_deref().map(str::trim), Some("New priority task"));

        // Clear with empty string → back to placeholder → None
        ws.set_current_task("").await.expect("clear");
        assert!(ws.get_current_task().await.expect("get").is_none());
    }

    #[tokio::test]
    async fn test_set_current_task_preserves_queue() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init");

        // Seed HEARTBEAT.md with queue + completed items
        let seed = "# Heartbeat Tasks\n\n\
            ## CURRENT TASK\n(Coordinator will assign your task here.)\n\n\
            ## TASK QUEUE\n- [ ] queued task A\n- [ ] queued task B\n\n\
            ## COMPLETED\n- [x] old task (2026-01-01)\n\n";
        ws.write("HEARTBEAT.md", seed).await.expect("seed");

        ws.set_current_task("active work").await.expect("set");

        let hb = ws.get_heartbeat().await.expect("read");
        assert!(hb.contains("active work"));
        assert!(hb.contains("queued task A"));
        assert!(hb.contains("queued task B"));
        assert!(hb.contains("old task"));
    }

    #[tokio::test]
    async fn test_append_to_task_queue() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init");

        // Set a current task first
        ws.set_current_task("Active task: build the widget").await.expect("set");

        // Append two tasks to queue
        ws.append_to_task_queue("Fix the login bug").await.expect("append 1");
        ws.append_to_task_queue("Write unit tests for auth").await.expect("append 2");

        let hb = ws.get_heartbeat().await.expect("read");

        // Current task preserved
        assert!(hb.contains("Active task: build the widget"));

        // Both queued tasks present
        assert!(hb.contains("- [ ] Fix the login bug"));
        assert!(hb.contains("- [ ] Write unit tests for auth"));

        // Queue items accessible via get_task_queue
        let queue = ws.get_task_queue().await.expect("get queue");
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0], "Fix the login bug");
        assert_eq!(queue[1], "Write unit tests for auth");
    }

    #[tokio::test]
    async fn test_append_to_task_queue_preserves_existing() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init");

        // Seed with existing queue items
        let seed = "# Heartbeat Tasks\n\n\
            ## CURRENT TASK\nDoing task A\n\n\
            ## TASK QUEUE\n- [ ] existing queued B\n- [ ] existing queued C\n\n\
            ## COMPLETED\n- [x] done task (2026-01-01)\n\n";
        ws.write("HEARTBEAT.md", seed).await.expect("seed");

        // Append a new task
        ws.append_to_task_queue("New task D").await.expect("append");

        let hb = ws.get_heartbeat().await.expect("read");
        assert!(hb.contains("Doing task A"));
        assert!(hb.contains("existing queued B"));
        assert!(hb.contains("existing queued C"));
        assert!(hb.contains("- [ ] New task D"));
        assert!(hb.contains("done task"));

        let queue = ws.get_task_queue().await.expect("get queue");
        assert_eq!(queue.len(), 3);
    }

    #[tokio::test]
    async fn test_init_idempotent() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");
        // Modify a file
        ws.write("AGENTS.md", "custom content")
            .await
            .expect("should write file");
        // Re-init should not overwrite
        ws.init().await.expect("async operation should succeed");
        let content = ws
            .read("AGENTS.md")
            .await
            .expect("async operation should succeed");
        assert_eq!(content, "custom content");
    }

    // -- Read / write / append / exists -------------------------------------

    #[tokio::test]
    async fn test_read_write() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("test.txt", "hello world")
            .await
            .expect("should write file");
        let content = ws
            .read("test.txt")
            .await
            .expect("async operation should succeed");
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("deep/nested/file.txt", "deep content")
            .await
            .expect("should write file");
        let content = ws
            .read("deep/nested/file.txt")
            .await
            .expect("async operation should succeed");
        assert_eq!(content, "deep content");
    }

    #[tokio::test]
    async fn test_append() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("log.txt", "line 1")
            .await
            .expect("should write file");
        ws.append("log.txt", "line 2")
            .await
            .expect("async operation should succeed");
        let content = ws
            .read("log.txt")
            .await
            .expect("async operation should succeed");
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
    }

    #[tokio::test]
    async fn test_append_to_new_file() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.append("new.txt", "first entry")
            .await
            .expect("async operation should succeed");
        let content = ws
            .read("new.txt")
            .await
            .expect("async operation should succeed");
        assert_eq!(content, "first entry");
    }

    #[tokio::test]
    async fn test_exists() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        assert!(ws.exists("AGENTS.md").await);
        assert!(!ws.exists("nonexistent.txt").await);
    }

    #[tokio::test]
    async fn test_read_nonexistent_fails() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let result = ws.read("does_not_exist.txt").await;
        assert!(result.is_err());
    }

    // -- Path traversal security --------------------------------------------

    #[tokio::test]
    async fn test_path_traversal_read_blocked() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let result = ws.read("../../etc/passwd").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[tokio::test]
    async fn test_path_traversal_write_blocked() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let result = ws.write("../evil.txt", "pwned").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_path_traversal_exists_returns_false() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        assert!(!ws.exists("../../etc/passwd").await);
    }

    #[tokio::test]
    async fn test_path_traversal_variants() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let attacks = vec![
            "../../../etc/passwd",
            "memory/../../etc/passwd",
            "./../../etc/shadow",
            "memory/../../../root/.ssh/id_rsa",
        ];
        for attack in attacks {
            assert!(ws.read(attack).await.is_err(), "Should block: {}", attack);
        }
    }

    #[tokio::test]
    async fn test_write_traversal_variants() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let attacks = vec![
            ("../../../tmp/evil", "payload"),
            ("memory/../../evil.txt", "payload"),
        ];
        for (path, content) in attacks {
            assert!(
                ws.write(path, content).await.is_err(),
                "Should block: {}",
                path
            );
        }
    }

    #[tokio::test]
    async fn test_symlink_traversal_blocked() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        // Create a symlink inside workspace pointing outside
        let symlink_path = tmp.path().join("evil_link");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", &symlink_path).expect("symlink should succeed");
            let result = ws.read("evil_link/passwd").await;
            assert!(result.is_err(), "Symlink traversal should be blocked");
        }
    }

    #[tokio::test]
    async fn test_valid_subpath_allowed() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        assert!(ws.read("AGENTS.md").await.is_ok());
    }

    #[tokio::test]
    async fn test_nested_subpath_allowed() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("memory/test.md", "content")
            .await
            .expect("should write file");
        let content = ws
            .read("memory/test.md")
            .await
            .expect("async operation should succeed");
        assert!(content.contains("content"));
    }

    // -- High-level memory operations ---------------------------------------

    #[tokio::test]
    async fn test_get_agents() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let agents = ws
            .get_agents()
            .await
            .expect("async operation should succeed");
        assert!(agents.contains("Zeus"));
    }

    #[tokio::test]
    async fn test_get_soul() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let soul = ws.get_soul().await.expect("async operation should succeed");
        assert!(soul.contains("Personality"));
    }

    #[tokio::test]
    async fn test_get_user() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let user = ws.get_user().await.expect("async operation should succeed");
        assert!(user.contains("User Preferences"));
    }

    #[tokio::test]
    async fn test_get_heartbeat() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let hb = ws
            .get_heartbeat()
            .await
            .expect("async operation should succeed");
        assert!(hb.contains("Heartbeat"));
    }

    #[tokio::test]
    async fn test_get_memory() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let mem = ws
            .get_memory()
            .await
            .expect("async operation should succeed");
        assert!(mem.contains("Long-term Memory"));
    }

    #[tokio::test]
    async fn test_remember() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.remember("User prefers dark mode")
            .await
            .expect("async operation should succeed");
        let mem = ws
            .get_memory()
            .await
            .expect("async operation should succeed");
        assert!(mem.contains("User prefers dark mode"));
    }

    #[tokio::test]
    async fn test_remember_multiple() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.remember("fact 1")
            .await
            .expect("async operation should succeed");
        ws.remember("fact 2")
            .await
            .expect("async operation should succeed");
        ws.remember("fact 3")
            .await
            .expect("async operation should succeed");
        let mem = ws
            .get_memory()
            .await
            .expect("async operation should succeed");
        assert!(mem.contains("fact 1"));
        assert!(mem.contains("fact 2"));
        assert!(mem.contains("fact 3"));
    }

    #[tokio::test]
    async fn test_daily_note() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.note("Had a productive day")
            .await
            .expect("async operation should succeed");
        let daily = ws
            .get_daily()
            .await
            .expect("async operation should succeed");
        assert!(daily.contains("Had a productive day"));
    }

    #[tokio::test]
    async fn test_get_daily_creates_file() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let daily = ws
            .get_daily()
            .await
            .expect("async operation should succeed");
        let today = Local::now().format("%Y-%m-%d").to_string();
        assert!(daily.contains(&today));
    }

    // -- Heartbeat tasks ----------------------------------------------------

    #[tokio::test]
    async fn test_heartbeat_tasks_daily() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let tasks = ws
            .get_heartbeat_tasks("daily")
            .await
            .expect("async operation should succeed");
        assert!(!tasks.is_empty());
        assert!(tasks.iter().any(|t| t.contains("Check for unfinished tasks")));
    }

    #[tokio::test]
    async fn test_heartbeat_tasks_weekly() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let tasks = ws
            .get_heartbeat_tasks("weekly")
            .await
            .expect("async operation should succeed");
        assert!(!tasks.is_empty());
    }

    #[tokio::test]
    async fn test_heartbeat_tasks_nonexistent_frequency() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let tasks = ws
            .get_heartbeat_tasks("yearly")
            .await
            .expect("async operation should succeed");
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_complete_heartbeat_task() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.complete_heartbeat_task("Check logs")
            .await
            .expect("async operation should succeed");
        let daily = ws
            .get_daily()
            .await
            .expect("async operation should succeed");
        assert!(daily.contains("Heartbeat: Check logs"));
    }

    // -- Context building ---------------------------------------------------

    #[tokio::test]
    async fn test_get_context() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let ctx = ws
            .get_context()
            .await
            .expect("async operation should succeed");
        assert!(ctx.contains("Zeus"));
        assert!(ctx.contains("Personality"));
        assert!(ctx.contains("User Context"));
        assert!(ctx.contains("Memory"));
    }

    // -- Context modes ------------------------------------------------------

    #[tokio::test]
    async fn test_no_active_context_by_default() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        assert!(ws.get_active_context().await.is_none());
    }

    #[tokio::test]
    async fn test_set_and_get_context_mode() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("contexts/dev.md", "# Dev Mode\nCode first.")
            .await
            .expect("should write context file");
        ws.set_context_mode(Some("dev"))
            .await
            .expect("should set context mode");
        assert_eq!(ws.get_active_context().await, Some("dev".to_string()));
    }

    #[tokio::test]
    async fn test_set_context_nonexistent_fails() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let result = ws.set_context_mode(Some("nonexistent")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_clear_context_mode() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("contexts/review.md", "# Review Mode")
            .await
            .expect("should write context file");
        ws.set_context_mode(Some("review"))
            .await
            .expect("should set mode");
        assert!(ws.get_active_context().await.is_some());

        ws.set_context_mode(None).await.expect("should clear mode");
        assert!(ws.get_active_context().await.is_none());
    }

    #[tokio::test]
    async fn test_context_mode_injected_into_get_context() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("contexts/research.md", "Focus on exploration and reading.")
            .await
            .expect("should write context file");
        ws.set_context_mode(Some("research"))
            .await
            .expect("should set mode");

        let ctx = ws
            .get_context()
            .await
            .expect("async operation should succeed");
        assert!(ctx.contains("Active Context: research"));
        assert!(ctx.contains("Focus on exploration and reading."));
    }

    #[tokio::test]
    async fn test_list_context_modes() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        ws.write("contexts/dev.md", "dev mode")
            .await
            .expect("should write");
        ws.write("contexts/review.md", "review mode")
            .await
            .expect("should write");
        ws.write("contexts/research.md", "research mode")
            .await
            .expect("should write");

        let modes = ws.list_context_modes().await;
        assert!(modes.contains(&"dev".to_string()));
        assert!(modes.contains(&"review".to_string()));
        assert!(modes.contains(&"research".to_string()));
    }

    #[tokio::test]
    async fn test_list_context_modes_empty_without_dir() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let modes = ws.list_context_modes().await;
        assert!(modes.is_empty());
    }

    // -- List files ---------------------------------------------------------

    #[tokio::test]
    async fn test_list_root() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let files = ws.list(".").await.expect("async operation should succeed");
        assert!(files.contains(&"AGENTS.md".to_string()));
        assert!(files.contains(&"SOUL.md".to_string()));
        assert!(files.contains(&"memory".to_string()));
        assert!(files.contains(&"daily".to_string()));
    }

    // -- Per-agent workspace -----------------------------------------------

    #[tokio::test]
    async fn test_for_agent_creates_scoped_workspace() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        let agent_ws = ws.for_agent("pr-monitor");
        assert_eq!(agent_ws.label(), "pr-monitor");
        assert!(agent_ws.root().ends_with("agents/pr-monitor"));
    }

    #[tokio::test]
    async fn test_init_agent_creates_files() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent(
            "pr-monitor",
            "name: PR Monitor\nemoji: 📋\nrole: Watch PRs",
            Some("## Hourly\n- Check open PRs"),
        )
        .await
        .expect("init_agent should succeed");

        assert!(tmp.path().join("agents/pr-monitor/IDENTITY.md").exists());
        assert!(tmp.path().join("agents/pr-monitor/HEARTBEAT.md").exists());
        assert!(tmp
            .path()
            .join("agents/pr-monitor/memory/MEMORY.md")
            .exists());
    }

    #[tokio::test]
    async fn test_agent_workspace_get_identity() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("test-agent", "name: TestBot\nemoji: 🤖", None)
            .await
            .expect("init_agent should succeed");

        let agent_ws = ws.for_agent("test-agent");
        let identity = agent_ws.get_identity().await.expect("should read identity");
        assert!(identity.contains("TestBot"));
    }

    #[tokio::test]
    async fn test_agent_workspace_soul_fallback() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("test-agent", "name: TestBot", None)
            .await
            .expect("init_agent should succeed");

        // No agent-specific SOUL.md — should fall back to global
        let agent_ws = ws.for_agent("test-agent");
        let soul = agent_ws.get_soul().await.expect("should read soul");
        assert!(soul.contains("Personality")); // from global DEFAULT_SOUL
    }

    #[tokio::test]
    async fn test_agent_workspace_soul_override() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("test-agent", "name: TestBot", None)
            .await
            .expect("init_agent should succeed");

        // Write agent-specific SOUL.md
        ws.write_agent_file("test-agent", "SOUL.md", "I am a sarcastic bot.")
            .await
            .expect("should write");

        let agent_ws = ws.for_agent("test-agent");
        let soul = agent_ws.get_soul().await.expect("should read soul");
        assert!(soul.contains("sarcastic"));
    }

    #[tokio::test]
    async fn test_agent_workspace_remember() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("test-agent", "name: TestBot", None)
            .await
            .expect("init_agent should succeed");

        let agent_ws = ws.for_agent("test-agent");
        agent_ws
            .remember("User prefers dark mode")
            .await
            .expect("should remember");

        let mem = agent_ws.get_memory().await.expect("should read memory");
        assert!(mem.contains("User prefers dark mode"));
    }

    #[tokio::test]
    async fn test_agent_workspace_context_building() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent(
            "pr-monitor",
            "name: PR Monitor\nrole: Watch GitHub PRs",
            None,
        )
        .await
        .expect("init_agent should succeed");

        let agent_ws = ws.for_agent("pr-monitor");
        let ctx = agent_ws.get_context().await.expect("should build context");

        // Should have global AGENTS.md
        assert!(ctx.contains("Zeus"));
        // Should have agent identity
        assert!(ctx.contains("Agent Identity"));
        assert!(ctx.contains("PR Monitor"));
        // Should have personality (fallback to global)
        assert!(ctx.contains("Personality"));
    }

    #[tokio::test]
    async fn test_list_agents_empty() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        let agents = ws.list_agents().await.expect("should list agents");
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn test_list_agents() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("alpha", "name: Alpha", None)
            .await
            .expect("init should succeed");
        ws.init_agent("beta", "name: Beta", None)
            .await
            .expect("init should succeed");
        ws.init_agent("gamma", "name: Gamma", None)
            .await
            .expect("init should succeed");

        let agents = ws.list_agents().await.expect("should list agents");
        assert_eq!(agents, vec!["alpha", "beta", "gamma"]);
    }

    #[tokio::test]
    async fn test_archive_agent() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("disposable", "name: Disposable", None)
            .await
            .expect("init should succeed");
        assert!(tmp.path().join("agents/disposable").exists());

        ws.archive_agent("disposable")
            .await
            .expect("should archive");
        assert!(!tmp.path().join("agents/disposable").exists());
        assert!(tmp.path().join("agents/.archived-disposable").exists());
    }

    #[tokio::test]
    async fn test_write_and_read_agent_file() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.write_agent_file("my-agent", "notes.txt", "some notes")
            .await
            .expect("should write");

        let content = ws
            .read_agent_file("my-agent", "notes.txt")
            .await
            .expect("should read");
        assert_eq!(content, "some notes");
    }

    #[tokio::test]
    async fn test_read_agent_file_fallback() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        // No agent-specific AGENTS.md — falls back to global
        let content = ws
            .read_agent_file("nonexistent-agent", "AGENTS.md")
            .await
            .expect("should fall back to global");
        assert!(content.contains("Zeus"));
    }

    #[tokio::test]
    async fn test_init_agent_without_heartbeat() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        ws.init_agent("no-hb", "name: NoHeartbeat", None)
            .await
            .expect("init should succeed");

        assert!(tmp.path().join("agents/no-hb/IDENTITY.md").exists());
        assert!(!tmp.path().join("agents/no-hb/HEARTBEAT.md").exists());
    }

    // -- List files ---------------------------------------------------------

    #[tokio::test]
    async fn test_list_sorted() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let files = ws.list(".").await.expect("async operation should succeed");
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
    }

    #[tokio::test]
    async fn test_list_subdirectory() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("async operation should succeed");

        let files = ws
            .list("memory")
            .await
            .expect("async operation should succeed");
        assert!(files.contains(&"MEMORY.md".to_string()));
    }

    // -- get_context_mtime_hash (cache key for system prompt) ---------------

    #[tokio::test]
    async fn test_get_context_mtime_hash_is_stable_when_files_unchanged() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        let h1 = ws.get_context_mtime_hash().await;
        let h2 = ws.get_context_mtime_hash().await;
        assert_eq!(
            h1, h2,
            "hash must be stable across calls with no filesystem changes"
        );
    }

    #[tokio::test]
    async fn test_get_context_mtime_hash_changes_when_agents_md_modified() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        let before = ws.get_context_mtime_hash().await;

        // Sleep long enough for mtime to definitely tick on any filesystem
        // (APFS/ext4 have sub-ms resolution but some environments don't).
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ws.write("AGENTS.md", "# modified content")
            .await
            .expect("write should succeed");

        let after = ws.get_context_mtime_hash().await;
        assert_ne!(
            before, after,
            "hash must change when a context file is modified"
        );
    }

    #[tokio::test]
    async fn test_get_context_mtime_hash_changes_when_memory_md_modified() {
        // MEMORY.md is in a subdirectory (memory/MEMORY.md) — verify the
        // helper still notices changes under subdirs, not just the root.
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        ws.init().await.expect("init should succeed");

        let before = ws.get_context_mtime_hash().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ws.write("memory/MEMORY.md", "# new memory")
            .await
            .expect("write should succeed");

        let after = ws.get_context_mtime_hash().await;
        assert_ne!(before, after);
    }

    #[tokio::test]
    async fn test_get_context_mtime_hash_handles_missing_files() {
        // Fresh workspace with NO files initialized — all context files
        // are absent. The helper must still return a stable hash (zero
        // mtimes for every entry) without panicking.
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ws = Workspace::new(tmp.path());
        // Deliberately skip ws.init() so no files exist.

        let h1 = ws.get_context_mtime_hash().await;
        let h2 = ws.get_context_mtime_hash().await;
        assert_eq!(h1, h2, "empty workspace must also produce a stable hash");

        // Now create AGENTS.md and verify the hash flips.
        ws.write("AGENTS.md", "hello")
            .await
            .expect("write should succeed");
        let h3 = ws.get_context_mtime_hash().await;
        assert_ne!(
            h1, h3,
            "creating a previously-missing context file should flip the hash"
        );
    }

    // -- Structured heartbeat task parsing -----------------------------------

    #[test]
    fn test_parse_structured_tasks_basic() {
        let content = r#"# Heartbeat Tasks

## tasks
- name: push-work
  interval: 30m
  prompt: "Push any uncommitted work"
- name: report
  interval: 1h
  prompt: "Report status to channel"
- name: current-task
  interval: 5m
  prompt: "Continue CURRENT TASK"
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].name, "push-work");
        assert_eq!(tasks[0].interval_secs, 1800); // 30m
        assert_eq!(tasks[0].prompt, "Push any uncommitted work");
        assert_eq!(tasks[1].name, "report");
        assert_eq!(tasks[1].interval_secs, 3600); // 1h
        assert_eq!(tasks[2].name, "current-task");
        assert_eq!(tasks[2].interval_secs, 300); // 5m
    }

    #[test]
    fn test_parse_structured_tasks_empty() {
        let content = "# Heartbeat Tasks\n\n## CURRENT TASK\nnothing\n";
        let tasks = parse_structured_tasks(content);
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_structured_tasks_with_comments() {
        let content = r#"## tasks
# This is a comment
- name: test-task
  interval: 10s
  prompt: "Do the thing"
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test-task");
        assert_eq!(tasks[0].interval_secs, 10);
    }

    #[test]
    fn test_parse_structured_tasks_stops_at_next_section() {
        // Form A `## tasks` block must terminate at the next header, but with
        // dual-form parsing the `## Daily` bullet now also becomes a task.
        let content = r#"## tasks
- name: task1
  interval: 5m
  prompt: "Do task 1"

## Daily
- Check for updates
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "task1");
        assert_eq!(tasks[0].interval_secs, 300);
        // Form B emits the bullet under `## Daily` as a daily-cadence task
        assert_eq!(tasks[1].name, "daily-1");
        assert_eq!(tasks[1].interval_secs, 86400);
        assert_eq!(tasks[1].prompt, "Check for updates");
    }

    #[test]
    fn test_parse_duration_str() {
        assert_eq!(parse_duration_str("30s"), 30);
        assert_eq!(parse_duration_str("5m"), 300);
        assert_eq!(parse_duration_str("1h"), 3600);
        assert_eq!(parse_duration_str("2h"), 7200);
        assert_eq!(parse_duration_str("1d"), 86400);
        assert_eq!(parse_duration_str("600"), 600);
        assert_eq!(parse_duration_str(""), 300); // default
    }

    #[test]
    fn test_parse_structured_tasks_default_heartbeat() {
        // DEFAULT_HEARTBEAT has BOTH a Form-A `## tasks` block (3 tasks) and
        // Form-B `## Daily` (2 bullets) + `## Weekly` (1 bullet) sections.
        // The dual-form parser must emit all 6 — previously the Daily/Weekly
        // bullets were silently dropped.
        let tasks = parse_structured_tasks(DEFAULT_HEARTBEAT);
        assert_eq!(tasks.len(), 6);
        assert_eq!(tasks[0].name, "push-work");
        assert_eq!(tasks[1].name, "report");
        assert_eq!(tasks[2].name, "current-task");
        assert_eq!(tasks[3].name, "daily-1");
        assert_eq!(tasks[3].interval_secs, 86400);
        assert_eq!(tasks[4].name, "daily-2");
        assert_eq!(tasks[5].name, "weekly-1");
        assert_eq!(tasks[5].interval_secs, 604800);
    }

    // -- Form B: frequency-bullet form (T21 template) -----------------------

    #[test]
    fn test_parse_structured_tasks_t21_hourly_bullets() {
        // This is the EXACT format the T21 HEARTBEAT.md template emits and what
        // every fleet workspace currently has on disk. Before the dual-form
        // parser, this returned 0 tasks → preflight_gate() was dormant.
        let content = r#"# HEARTBEAT.md — zeus106

## hourly
- First: push any uncommitted work
- Then: report what you did to your team channel
- Then: continue your CURRENT TASK

## CURRENT TASK
(Coordinator will assign your task here.)
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 3, "T21 hourly bullets must parse as 3 tasks");

        assert_eq!(tasks[0].name, "hourly-1");
        assert_eq!(tasks[0].interval_secs, 3600);
        assert_eq!(tasks[0].prompt, "First: push any uncommitted work");

        assert_eq!(tasks[1].name, "hourly-2");
        assert_eq!(tasks[1].interval_secs, 3600);
        assert_eq!(tasks[1].prompt, "Then: report what you did to your team channel");

        assert_eq!(tasks[2].name, "hourly-3");
        assert_eq!(tasks[2].interval_secs, 3600);
        assert_eq!(tasks[2].prompt, "Then: continue your CURRENT TASK");
    }

    #[test]
    fn test_parse_structured_tasks_bullet_form_daily_weekly() {
        let content = r#"## daily
- Review open PRs
- Sync with coordinator

## weekly
- Audit memory for stale entries
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 3);

        assert_eq!(tasks[0].name, "daily-1");
        assert_eq!(tasks[0].interval_secs, 86400);
        assert_eq!(tasks[0].prompt, "Review open PRs");

        assert_eq!(tasks[1].name, "daily-2");
        assert_eq!(tasks[1].interval_secs, 86400);

        assert_eq!(tasks[2].name, "weekly-1");
        assert_eq!(tasks[2].interval_secs, 604800);
        assert_eq!(tasks[2].prompt, "Audit memory for stale entries");
    }

    #[test]
    fn test_parse_structured_tasks_both_forms_coexist() {
        // Mixing Form A (`## tasks`) and Form B (`## hourly`) in one file.
        let content = r#"## hourly
- Quick status check

## tasks
- name: push-work
  interval: 30m
  prompt: "Push uncommitted work"
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "hourly-1");
        assert_eq!(tasks[0].interval_secs, 3600);
        assert_eq!(tasks[1].name, "push-work");
        assert_eq!(tasks[1].interval_secs, 1800);
    }

    #[test]
    fn test_parse_structured_tasks_bullet_ignores_non_bullet_lines() {
        // Prose lines under `## hourly` should not become tasks.
        let content = r#"## hourly
This is a header comment, not a task.
- Real task one
Some more prose.
- Real task two
"#;
        let tasks = parse_structured_tasks(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].prompt, "Real task one");
        assert_eq!(tasks[1].prompt, "Real task two");
    }

    #[test]
    fn test_parse_structured_tasks_bullet_section_terminates_at_next_header() {
        let content = r#"## hourly
- Task A
- Task B

## CURRENT TASK
- This bullet should NOT become a task

## daily
- Daily task
"#;
        let tasks = parse_structured_tasks(content);
        // hourly: 2, daily: 1, CURRENT TASK section: 0
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].name, "hourly-1");
        assert_eq!(tasks[1].name, "hourly-2");
        assert_eq!(tasks[2].name, "daily-1");
    }
}
