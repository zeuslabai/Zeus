//! Zeus Skills - OpenClaw Compatible Skill System & Plugin Framework
//!
//! Provides SKILL.md file parsing, ClawHub client for skill discovery,
//! skill execution engine, and a full plugin system with support for
//! Node.js, Python, shell-based plugins, WASM sandboxing, and native
//! dynamic library loading with hot-reload.

pub mod bridge;
pub mod clawhub;
pub mod clawhub_import;
pub mod dependency_resolver;
pub mod dynamic_plugins;
pub mod installer;
pub mod loader;
pub mod migration;
pub mod openclaw;
pub mod plugin;
pub mod security_scan;
pub mod skill_matcher;
pub mod skill_permissions;
pub mod skill_plugin;
pub mod skill_versioning;
pub mod wasm_metrics;
pub mod wasm_sandbox;

// Re-export key types for convenience
pub use bridge::{NodePlugin, ProcessBridge, PythonPlugin};
pub use clawhub::{ClawHubClient, SkillSummary as ClawHubSkillSummary};
pub use clawhub_import::{BatchImportResult, ClawHubImportConfig, ClawHubImporter, ImportResult};
pub use dependency_resolver::{DependencyGraph, DependencyResolver, InstallPlan, VersionConflict};
pub use dynamic_plugins::{
    DynamicPluginError, DynamicPluginLoader, DynamicPluginLoaderBuilder, NativePluginInfo,
    PluginEvent,
};
pub use installer::{InstallResult, InstallSource, SkillInstaller, ValidationResult};
pub use loader::{PluginLoader, PluginManifest, PluginRuntime, PluginToolDef};
pub use plugin::{Plugin, PluginRegistry};
pub use security_scan::{security_scan, ScanResult, SecurityFinding, Severity};
pub use skill_permissions::{SkillCapability, SkillPermissionPolicy, SkillPermissionRegistry};
pub use skill_plugin::SkillPlugin;
pub use skill_versioning::{
    DependencyResolution, SkillDependency, SkillVersion, SkillVersionRegistry, UpgradeInfo,
    VersionReq,
};
pub use wasm_metrics::{
    MetricsReport, SkillMetricsReport, WasmExecutionMetrics, WasmMetricsCollector,
};
pub use wasm_sandbox::{
    WasmCapabilities, WasmError, WasmExecutionResult, WasmPluginMetadata, WasmSandbox,
    WasmSandboxBuilder,
};
pub use migration::{ConfigMapping, MigrationEngine, MigrationReport, SkillMigrationResult};
// Re-export OpenClaw types from the dedicated module
pub use openclaw::{
    CommandDispatchSpec, GatingResult, InstallSpec as OpenClawInstallSpec, OpenClawMetadata,
    OpenClawSkill, RequirementsSpec, SkillInvocationPolicy, SkillSource, check_requirements,
    load_skills_from_dir, load_skills_with_precedence, parse_frontmatter, parse_openclaw_skill,
    resolve_metadata, resolve_read_when, slugify,
};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result};

/// A resolved skill command specification for registration in the agent.
/// Uses `CommandDispatchSpec` from the `openclaw` module for dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCommandSpec {
    /// Command name (sanitized, max 32 chars, `^[a-z0-9_]+$`).
    pub name: String,
    /// Original skill name.
    pub skill_name: String,
    /// Short description (max 100 chars).
    pub description: String,
    /// Optional deterministic dispatch behavior.
    pub dispatch: Option<CommandDispatchSpec>,
}

/// Max length for skill command names.
const SKILL_COMMAND_MAX_LENGTH: usize = 32;

// ---------------------------------------------------------------------------
// Progressive disclosure — the three levels.
//
// Skills are disclosed to the agent in three escalating depths so the
// always-loaded prompt stays small while full content remains one `read_file`
// away:
//
//   L1  index    — `SkillManager::get_summary`: always loaded. Name + path +
//                   a capped tagline. NEVER carries the body.
//   L2  body     — `SkillManager::get_triggered_context`: injected ONLY when a
//                   skill's `read_when` keywords match the request. Bounded to
//                   a generous ceiling with a `read_file` pointer past it.
//   L3  raw      — `Skill::raw_content`: `#[serde(skip)]`, never serialized
//                   into any prompt. Reachable only via `read_file`.
// ---------------------------------------------------------------------------

/// L1 ceiling: max chars for a per-skill tagline in the always-loaded index.
/// Tight on purpose — L1 is a pointer, not the body.
const L1_TAGLINE_CHARS: usize = 120;

/// L2 ceiling: max chars for a triggered skill's body before a `read_file`
/// pointer is appended. Generous on purpose — the user just triggered this
/// skill; the win is the ceiling against an unbounded dump, not aggressive
/// truncation.
const L2_BODY_CHARS: usize = 8_000;

/// Skill manager for loading and executing skills
pub struct SkillManager {
    skills: HashMap<String, Skill>,
    skills_dir: PathBuf,
    /// Optional credential vault for injecting API keys into skill subprocesses.
    credential_vault: Option<std::sync::Arc<zeus_aegis::CredentialVault>>,
    /// Skill names forced always-active for this run (e.g. a persona's
    /// `default_skills`). Unioned with keyword-triggered skills in
    /// [`SkillManager::get_triggered_context`]. Names that don't correspond to a
    /// loaded skill are silently ignored at activation time.
    force_active: HashSet<String>,
}

impl SkillManager {
    /// Create a new skill manager
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dir,
            credential_vault: None,
            force_active: HashSet::new(),
        }
    }

    /// Attach a credential vault for API key injection into skill subprocesses.
    pub fn with_vault(mut self, vault: std::sync::Arc<zeus_aegis::CredentialVault>) -> Self {
        self.credential_vault = Some(vault);
        self
    }
}

impl Default for SkillManager {
    fn default() -> Self {
        let skills_dir = zeus_core::default_config_dir().join("skills");
        Self::new(skills_dir)
    }
}

impl SkillManager {
    /// Load all skills from the primary skills directory.
    ///
    /// Scans for `<dir>/<skill_name>/SKILL.md` entries.
    pub async fn load_all(&mut self) -> Result<usize> {
        if !self.skills_dir.exists() {
            std::fs::create_dir_all(&self.skills_dir)?;
            return Ok(0);
        }

        // #163 Cut-2b: one-time, write-only-missing backfill of permission
        // policies for skills installed before policy-at-ingestion landed.
        // The ClawHubClient loads the installed registry on construction; the
        // sweep persists a *derived* policy only for skills with no recorded
        // entry — so pre-existing skills get the exact policy a fresh install
        // would give, instead of being treated as the strictest (Paranoid) by
        // the exec seam. Idempotent: a startup that finds every skill already
        // recorded is a no-op, so this is naturally re-run-safe.
        let backfill_client = clawhub::ClawHubClient::new(self.skills_dir.clone());
        backfill_client.backfill_skill_policies();

        let dir = self.skills_dir.clone();
        self.load_skills_from_dir(&dir, false).await
    }

    /// Load skills from an additional directory, merging into the existing
    /// in-memory map. Later loads with the same skill name override earlier ones
    /// (last-write-wins). Use this to layer in workspace/community skills on top
    /// of the primary `~/.zeus/skills/` directory.
    ///
    /// If `nested` is true, the loader treats `<dir>` as a parent of multiple
    /// repos and scans `<dir>/<repo>/skills/<skill>/SKILL.md`. Otherwise it
    /// scans `<dir>/<skill>/SKILL.md`.
    pub async fn load_extra_dir(&mut self, dir: &Path, nested: bool) -> Result<usize> {
        if !dir.exists() {
            return Ok(0);
        }
        self.load_skills_from_dir(dir, nested).await
    }

    /// Internal helper: scan a directory for skills, optionally walking one
    /// extra level for `.community_skills/<repo>/skills/<skill>/` layouts.
    async fn load_skills_from_dir(&mut self, dir: &Path, nested: bool) -> Result<usize> {
        let mut count = 0;
        let entries = std::fs::read_dir(dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Direct skill: <dir>/<skill>/SKILL.md
            let skill_file = path.join("SKILL.md");
            if skill_file.exists() {
                if let Ok(skill) = self.load_skill(&skill_file).await {
                    self.skills.insert(skill.name.clone(), skill);
                    count += 1;
                }
                continue;
            }

            // Nested layout: <dir>/<repo>/skills/<skill>/SKILL.md
            if nested {
                let nested_root = path.join("skills");
                if nested_root.is_dir()
                    && let Ok(inner) = std::fs::read_dir(&nested_root)
                {
                    for sub in inner.flatten() {
                        let sub_path = sub.path();
                        if sub_path.is_dir() {
                            let sf = sub_path.join("SKILL.md");
                            if sf.exists()
                                && let Ok(skill) = self.load_skill(&sf).await
                            {
                                self.skills.insert(skill.name.clone(), skill);
                                count += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Load a single skill from a SKILL.md file
    pub async fn load_skill(&self, path: &PathBuf) -> Result<Skill> {
        let content = std::fs::read_to_string(path)?;
        parse_skill_md(&content, path.parent().unwrap_or(path).to_path_buf())
    }

    /// Get a skill by name
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// List all loaded skills
    pub fn list(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    /// Access the raw skills map (used by SkillMatcher for indexing).
    pub fn skills(&self) -> &HashMap<String, Skill> {
        &self.skills
    }

    /// Execute a skill tool, injecting credentials as env vars into the subprocess.
    ///
    /// Credential names are taken from the skill's OpenClaw metadata
    /// (`requires.env` + `primaryEnv`). Values are resolved from the
    /// `CredentialVault` and passed directly to `Command::envs()` —
    /// they are never logged or visible to the LLM.
    pub async fn execute(
        &self,
        skill_name: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String> {
        let skill = self
            .skills
            .get(skill_name)
            .ok_or_else(|| Error::Skill(format!("Skill not found: {}", skill_name)))?;

        let tool = skill
            .tools
            .iter()
            .find(|t| t.name == tool_name)
            .ok_or_else(|| Error::Skill(format!("Tool not found: {}", tool_name)))?;

        // Collect credential env var names from skill OpenClaw metadata.
        let cred_names = collect_skill_credential_names(skill);

        // Resolve values from vault (missing names silently skipped).
        let mut env_vars = if !cred_names.is_empty() {
            if let Some(ref vault) = self.credential_vault {
                vault.resolve_map(&cred_names).await.unwrap_or_default()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        // Inject SKILL_DIR so scripts can reference sibling files.
        if let Some(parent) = skill.path.parent() {
            env_vars.insert("SKILL_DIR".to_string(), parent.to_string_lossy().to_string());
        }

        match &tool.implementation {
            ToolImplementation::Shell { command } => {
                execute_shell_with_env(command, &args, env_vars).await
            }
            ToolImplementation::Script {
                interpreter,
                script,
            } => execute_script_with_env(interpreter, script, &args, env_vars).await,
            ToolImplementation::Native => {
                Err(Error::Skill("Native tools not yet supported".to_string()))
            }
        }
    }

    /// Install a skill from ClawHub (validates and reviews permissions via aegis)
    pub async fn install(&mut self, name: &str) -> Result<()> {
        let mut client = clawhub::ClawHubClient::new(self.skills_dir.clone());
        client.install(name).await?;

        // Reload skills
        self.load_all().await?;
        Ok(())
    }

    /// Cap a description down to a short tagline for the L1 index.
    ///
    /// Mirrors the 100-char UTF-8-safe truncation used by the command-spec
    /// builder, but tighter (`L1_TAGLINE_CHARS`) because L1 is the always-loaded
    /// skill index — it must stay a pointer, never the body. A truncated tagline
    /// is suffixed with `…` so the agent knows there is more to read via
    /// `read_file`.
    fn tagline(description: &str) -> String {
        let d = description.trim();
        if d.is_empty() {
            return String::new();
        }
        // Collapse to the first line — taglines are single-line by contract.
        let first_line = d.lines().next().unwrap_or(d).trim();
        if first_line.chars().count() <= L1_TAGLINE_CHARS {
            return first_line.to_string();
        }
        let end = zeus_core::floor_char_boundary(
            first_line,
            L1_TAGLINE_CHARS.saturating_sub(1),
        );
        format!("{}…", &first_line[..end])
    }

    /// Returns a compact summary of available skills for the system prompt.
    /// Instead of loading full SKILL.md content, this returns a short list
    /// that the agent can use to decide which skill to load in detail.
    ///
    /// # Progressive disclosure
    ///
    /// This is **L1** — the always-loaded skill index. It carries only the
    /// skill name, path, and a capped tagline (see [`SkillManager::tagline`]).
    /// It must never carry a skill's body; that lives in **L2**
    /// ([`SkillManager::get_triggered_context`], injected only on `read_when`
    /// match) and **L3** (`Skill::raw_content`, `#[serde(skip)]`, never
    /// serialized into any prompt — only reachable via `read_file`).
    pub fn get_summary(&self) -> String {
        if self.skills.is_empty() {
            return "No skills loaded.".to_string();
        }

        let mut summary = String::from("Available skills:\n");
        let mut skills: Vec<_> = self.skills.values().collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        for skill in &skills {
            // L1 (index): per-skill description is capped to a short tagline.
            // L1 is always loaded into the system prompt, so it must never carry
            // the full body — only enough to let the agent decide whether to
            // read_file the skill for the full instructions (L2/L3).
            let tagline = Self::tagline(&skill.description);
            summary.push_str(&format!(
                "- {} ({}): {}\n",
                skill.name,
                skill.path.display(),
                if tagline.is_empty() {
                    "No description".to_string()
                } else {
                    tagline
                },
            ));
        }
        summary.push_str("\nUse read_file on a skill's path to load its full instructions.");
        summary
    }

    /// Build command specs for all loaded skills (OpenClaw-compatible).
    /// Produces a deduplicated list of slash-command registrations with
    /// sanitized names and optional dispatch targets.
    pub fn build_command_specs(&self) -> Vec<SkillCommandSpec> {
        let mut specs = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        for skill in self.skills.values() {
            // Skip skills that aren't user-invocable
            if !skill.invocation.user_invocable {
                continue;
            }

            // Determine command name: skillKey override > sanitized skill name
            let raw_name = skill
                .metadata
                .as_ref()
                .and_then(|m| m.skill_key.as_deref())
                .unwrap_or(&skill.name);
            let cmd_name = sanitize_command_name(raw_name);

            // Deduplicate — first registration wins
            if seen_names.contains(&cmd_name) {
                continue;
            }
            seen_names.insert(cmd_name.clone());

            // Truncate description to 100 chars (UTF-8 safe)
            let desc = if skill.description.len() > 100 {
                let end = zeus_core::floor_char_boundary(&skill.description, 97);
                format!("{}...", &skill.description[..end])
            } else if skill.description.is_empty() {
                format!("Run the {} skill", skill.name)
            } else {
                skill.description.clone()
            };

            specs.push(SkillCommandSpec {
                name: cmd_name,
                skill_name: skill.name.clone(),
                description: desc,
                dispatch: skill.command_dispatch.clone(),
            });
        }

        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    /// Find skills whose `read_when` keywords match the given message (case-insensitive substring).
    /// Returns skills in alphabetical order for deterministic injection order.
    ///
    /// **Note:** matching is simple substring — `read_when: ["go"]` would trigger on "Let's go to
    /// the store". Keep trigger phrases specific (e.g. "git commit" not "git") to avoid false
    /// positives. Word-boundary matching can be added later if needed.
    /// Force a set of skills always-active for this run, regardless of keyword
    /// triggering. Used to apply a persona's `default_skills`.
    ///
    /// Only names that correspond to a **currently loaded** skill take effect;
    /// unknown or not-yet-loaded names are silently ignored (no-op), matching the
    /// behavior of a keyword that matches nothing. Call this *after* skills are
    /// loaded and *before* the manager is shared (it mutates).
    pub fn enable_skills(&mut self, names: &[String]) {
        for name in names {
            if self.skills.contains_key(name) {
                self.force_active.insert(name.clone());
            }
        }
    }

    /// Skills triggered for `message`: the union of keyword-triggered skills and
    /// the force-active set (a persona's `default_skills`). Sorted by name,
    /// deduplicated.
    pub fn find_triggered_skills(&self, message: &str) -> Vec<&Skill> {
        let lower = message.to_lowercase();
        let mut triggered: Vec<&Skill> = self
            .skills
            .values()
            .filter(|s| {
                self.force_active.contains(&s.name)
                    || (!s.read_when.is_empty()
                        && s.read_when
                            .iter()
                            .any(|kw| lower.contains(&kw.to_lowercase())))
            })
            .collect();
        triggered.sort_by(|a, b| a.name.cmp(&b.name));
        triggered
    }

    /// Bound a triggered skill's body to the generous L2 ceiling.
    ///
    /// L2 is the body of a skill the user *just triggered* via `read_when`, so
    /// the cap is intentionally generous (`L2_BODY_CHARS`) — the win is a
    /// ceiling against an unbounded dump, not aggressive truncation. When the
    /// body exceeds the ceiling it is cut on a UTF-8 boundary and a
    /// `…[truncated — read_file <path> for full]` pointer is appended so the
    /// agent can fetch the rest (L3) on demand.
    fn bound_body(body: &str, path: &Path) -> String {
        if body.chars().count() <= L2_BODY_CHARS {
            return body.to_string();
        }
        let end = zeus_core::floor_char_boundary(body, L2_BODY_CHARS);
        format!(
            "{}\n…[truncated — read_file {} for full]",
            &body[..end],
            path.display()
        )
    }

    /// Build a context block injected into the system prompt for all skills
    /// triggered by `read_when` keyword matching against `message`.
    /// Returns `None` if no skills match.
    ///
    /// # Progressive disclosure
    ///
    /// This is **L2** — a skill's body, injected *only* when the skill's
    /// `read_when` keywords match the request (see
    /// [`SkillManager::find_triggered_skills`]). Each body is bounded to a
    /// generous ceiling (see [`SkillManager::bound_body`]); past it, a
    /// `read_file` pointer to the full content (**L3**, `raw_content`) is
    /// appended. **L1** ([`SkillManager::get_summary`]) is the always-loaded
    /// index and never reaches this depth.
    pub fn get_triggered_context(&self, message: &str) -> Option<String> {
        let triggered = self.find_triggered_skills(message);
        if triggered.is_empty() {
            return None;
        }
        let mut ctx = String::from(
            "[Auto-Activated Skills]\nThe following skills are active for this request:\n",
        );
        for skill in &triggered {
            ctx.push_str(&format!("\n### {}\n", skill.name));
            if !skill.system_prompt.is_empty() {
                ctx.push_str(&Self::bound_body(&skill.system_prompt, &skill.path));
                ctx.push('\n');
            } else if !skill.description.is_empty() {
                ctx.push_str(&Self::bound_body(&skill.description, &skill.path));
                ctx.push('\n');
            }
        }
        Some(ctx)
    }

    /// Uninstall a skill
    pub fn uninstall(&mut self, name: &str) -> Result<()> {
        self.uninstall_with_options(name, &UninstallOptions::default())
    }

    /// Uninstall a skill with options (dry-run, force, keep-files)
    pub fn uninstall_with_options(&mut self, name: &str, options: &UninstallOptions) -> Result<()> {
        let skill_dir = self.skills_dir.join(name);
        let loaded = self.skills.contains_key(name);
        let on_disk = skill_dir.exists();

        if !loaded && !on_disk {
            return Err(Error::Skill(format!("Skill not found: {}", name)));
        }

        if options.dry_run {
            // Dry-run: just return Ok without doing anything
            return Ok(());
        }

        if !options.keep_files && on_disk {
            std::fs::remove_dir_all(&skill_dir)?;
        }

        self.skills.remove(name);
        Ok(())
    }

    /// Preview what uninstalling a skill would do (for dry-run)
    pub fn uninstall_preview(&self, name: &str) -> Result<UninstallPreview> {
        let skill_dir = self.skills_dir.join(name);
        let loaded = self.skills.contains_key(name);
        let on_disk = skill_dir.exists();

        if !loaded && !on_disk {
            return Err(Error::Skill(format!("Skill not found: {}", name)));
        }

        let mut files = Vec::new();
        let mut total_size = 0u64;

        if on_disk {
            Self::collect_files(&skill_dir, &mut files, &mut total_size);
        }

        Ok(UninstallPreview {
            name: name.to_string(),
            skill_dir: skill_dir.to_string_lossy().to_string(),
            files,
            total_size,
            loaded,
        })
    }

    /// Recursively collect file paths and sizes under a directory
    fn collect_files(dir: &std::path::Path, files: &mut Vec<String>, total_size: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_files(&path, files, total_size);
                } else if let Ok(meta) = path.metadata() {
                    *total_size += meta.len();
                    files.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
}

/// Options for skill uninstall
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UninstallOptions {
    /// If true, show what would be removed without actually removing
    #[serde(default)]
    pub dry_run: bool,
    /// If true, skip any confirmation (for API/CLI batch usage)
    #[serde(default)]
    pub force: bool,
    /// If true, remove from registry but keep files on disk
    #[serde(default)]
    pub keep_files: bool,
}

/// Preview of what an uninstall would do
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallPreview {
    /// Skill name
    pub name: String,
    /// Path to skill directory
    pub skill_dir: String,
    /// Files that would be removed
    pub files: Vec<String>,
    /// Total bytes that would be freed
    pub total_size: u64,
    /// Whether skill is currently loaded in memory
    pub loaded: bool,
}

/// A loaded skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name
    pub name: String,
    /// Description
    pub description: String,
    /// Version
    pub version: String,
    /// Author
    pub author: Option<String>,
    /// System prompt for the skill
    pub system_prompt: String,
    /// Tools provided by the skill
    pub tools: Vec<SkillTool>,
    /// Required permissions
    pub permissions: Vec<String>,
    /// Skill directory path
    #[serde(skip)]
    pub path: PathBuf,
    /// Raw SKILL.md content
    #[serde(skip)]
    pub raw_content: String,
    /// OpenClaw invocation policy
    #[serde(default)]
    pub invocation: SkillInvocationPolicy,
    /// OpenClaw command dispatch spec
    #[serde(default)]
    pub command_dispatch: Option<CommandDispatchSpec>,
    /// OpenClaw metadata (requirements, install specs, etc.)
    #[serde(default)]
    pub metadata: Option<OpenClawMetadata>,
    /// YAML frontmatter key-value pairs (raw)
    #[serde(default)]
    pub frontmatter: HashMap<String, String>,
    /// Auto-activation trigger keywords (from `read_when` frontmatter field).
    /// Skill system prompt is injected into context when any keyword matches user input.
    #[serde(default)]
    pub read_when: Vec<String>,
}

/// A tool defined in a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTool {
    /// Tool name
    pub name: String,
    /// Description
    pub description: String,
    /// Input schema (JSON Schema)
    pub input_schema: serde_json::Value,
    /// Implementation type
    pub implementation: ToolImplementation,
}

/// Tool implementation types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolImplementation {
    /// Shell command
    Shell { command: String },
    /// Script with interpreter
    Script { interpreter: String, script: String },
    /// Native Rust implementation
    Native,
}

/// Sanitize a skill name into a valid command name (`^[a-z0-9_]+$`, max 32 chars).
pub fn sanitize_command_name(name: &str) -> String {
    let normalized: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = normalized.trim_matches('_').to_string();
    if trimmed.len() > SKILL_COMMAND_MAX_LENGTH {
        let end = zeus_core::floor_char_boundary(&trimmed, SKILL_COMMAND_MAX_LENGTH);
        trimmed[..end].to_string()
    } else if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed
    }
}

/// Parse a SKILL.md file with OpenClaw-compatible frontmatter support.
///
/// Delegates frontmatter parsing to `openclaw::parse_frontmatter()` (serde_yaml
/// with line-based fallback), then parses the markdown body for tools and
/// permissions sections.
pub fn parse_skill_md(content: &str, path: PathBuf) -> Result<Skill> {
    // Use the openclaw module's parser (serde_yaml + line-based fallback)
    let (frontmatter, body) = openclaw::parse_frontmatter(content);

    // Extract fields from frontmatter first, fall back to markdown parsing
    let mut name = frontmatter.get("name").cloned().unwrap_or_default();
    let mut description = frontmatter.get("description").cloned().unwrap_or_default();
    let mut version = frontmatter
        .get("version")
        .cloned()
        .unwrap_or_else(|| "0.1.0".to_string());
    let mut author: Option<String> = frontmatter.get("author").cloned();
    let mut system_prompt = String::new();
    let mut tools = Vec::new();
    let mut permissions = Vec::new();

    let mut in_system_prompt = false;
    let mut in_tools = false;
    let mut in_permissions = false;
    let mut found_first_heading = false;

    // Parse the markdown body (after frontmatter)
    for line in body.lines() {
        if let Some(stripped) = line.strip_prefix("# ") {
            if !found_first_heading {
                found_first_heading = true;
                if name.is_empty() {
                    name = stripped.trim().to_string();
                }
                in_system_prompt = true;
                in_tools = false;
                in_permissions = false;
                continue;
            }
        } else if line.starts_with("## Description") {
            in_system_prompt = false;
            in_tools = false;
            in_permissions = false;
        } else if line.starts_with("## Version") {
            in_system_prompt = false;
            in_tools = false;
            in_permissions = false;
            if version == "0.1.0"
                && let Some(v) = line.strip_prefix("## Version:")
            {
                version = v.trim().to_string();
            }
        } else if line.starts_with("## Author") {
            in_system_prompt = false;
            in_tools = false;
            in_permissions = false;
            if author.is_none()
                && let Some(a) = line.strip_prefix("## Author:")
            {
                author = Some(a.trim().to_string());
            }
        } else if line.starts_with("## System Prompt") {
            in_system_prompt = true;
            in_tools = false;
            in_permissions = false;
            continue;
        } else if line.starts_with("## Tools") {
            in_system_prompt = false;
            in_tools = true;
            in_permissions = false;
            continue;
        } else if line.starts_with("## Permissions") {
            in_system_prompt = false;
            in_tools = false;
            in_permissions = true;
            continue;
        } else if line.starts_with("## ") {
            in_tools = false;
            in_permissions = false;
        }

        if in_system_prompt && !line.starts_with("## Tools") && !line.starts_with("## Permissions")
        {
            system_prompt.push_str(line);
            system_prompt.push('\n');
        } else if in_tools && line.starts_with("- ") {
            let tool_def = line[2..].trim();
            if let Some((tool_name, tool_desc)) = tool_def.split_once(':') {
                tools.push(SkillTool {
                    name: tool_name.trim().to_string(),
                    description: tool_desc.trim().to_string(),
                    input_schema: serde_json::json!({"type": "object", "properties": {}}),
                    implementation: ToolImplementation::Shell {
                        command: tool_name.trim().to_string(),
                    },
                });
            }
        } else if in_permissions && line.starts_with("- ") {
            permissions.push(line[2..].trim().to_string());
        } else if !line.is_empty()
            && description.is_empty()
            && !line.starts_with('#')
            && !in_system_prompt
        {
            description = line.to_string();
        }
    }

    if name.is_empty() {
        return Err(Error::Skill(
            "SKILL.md missing name (no frontmatter 'name' or '# heading')".to_string(),
        ));
    }

    // Use openclaw module for metadata/invocation/dispatch resolution
    let metadata = openclaw::resolve_metadata(&frontmatter);
    let invocation = openclaw::resolve_invocation_policy(&frontmatter);
    let command_dispatch = openclaw::resolve_command_dispatch(&frontmatter);
    let read_when = openclaw::resolve_read_when(&frontmatter);

    // Use homepage from metadata as fallback for description
    if description.is_empty()
        && let Some(ref meta) = metadata
        && let Some(ref hp) = meta.homepage
    {
        description = format!("See {}", hp);
    }

    Ok(Skill {
        name,
        description,
        version,
        author,
        system_prompt: system_prompt.trim().to_string(),
        tools,
        permissions,
        path,
        raw_content: content.to_string(),
        invocation,
        command_dispatch,
        metadata,
        frontmatter,
        read_when,
    })
}

/// Sanitize a string for safe use as a shell argument by wrapping in single quotes.
fn sanitize_shell_arg(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Single-pass placeholder substitution to prevent second-order injection.
///
/// Iterative `str::replace` is vulnerable: if arg1's sanitized value contains
/// `{arg2}` as a substring, the subsequent replacement of `{arg2}` breaks out
/// of the single-quote context and enables command injection.
///
/// This function scans left-to-right once, substituting `{key}` placeholders
/// as they are encountered, so replacement text is never re-scanned.
fn substitute_placeholders(
    template: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<String> {
    let mut result = String::with_capacity(template.len() + 64);
    let mut remaining = template;

    while let Some(start) = remaining.find('{') {
        // Push everything before the `{`
        result.push_str(&remaining[..start]);

        let after_brace = &remaining[start + 1..];
        if let Some(end) = after_brace.find('}') {
            let key = &after_brace[..end];

            if let Some(value) = args.get(key) {
                let raw = match value {
                    serde_json::Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                result.push_str(&sanitize_shell_arg(&raw));
            } else {
                // Unknown placeholder — preserve literal text
                result.push('{');
                result.push_str(key);
                result.push('}');
            }
            remaining = &after_brace[end + 1..];
        } else {
            // Unmatched `{` — push literal and continue
            result.push('{');
            remaining = after_brace;
        }
    }

    // Push remaining text after the last placeholder
    result.push_str(remaining);

    Ok(result)
}

/// Collect credential env var names required by a skill from OpenClaw metadata.
fn collect_skill_credential_names(skill: &Skill) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(ref meta) = skill.metadata {
        if let Some(ref reqs) = meta.requires
            && let Some(ref env) = reqs.env
        {
            names.extend(env.iter().cloned());
        }
        if let Some(ref primary) = meta.primary_env
            && !names.contains(primary)
        {
            names.push(primary.clone());
        }
    }
    names
}

/// Execute a shell command with safe single-pass placeholder substitution.
pub(crate) async fn execute_shell(command: &str, args: &serde_json::Value) -> Result<String> {
    use std::process::Command;

    let cmd = match args.as_object() {
        Some(obj) => substitute_placeholders(command, obj)?,
        None => command.to_string(),
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .map_err(|e| Error::Skill(format!("Failed to execute command: {}", e)))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::Skill(format!(
            "Command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Execute a shell command with credential env vars injected into the subprocess.
///
/// Credentials are passed via `Command::envs()` — they are set only on the child
/// process and are never logged or accessible to the parent agent process.
pub(crate) async fn execute_shell_with_env(
    command: &str,
    args: &serde_json::Value,
    env: HashMap<String, String>,
) -> Result<String> {
    use std::process::Command;

    let cmd = match args.as_object() {
        Some(obj) => substitute_placeholders(command, obj)?,
        None => command.to_string(),
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .envs(&env)
        .output()
        .map_err(|e| Error::Skill(format!("Failed to execute command: {}", e)))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::Skill(format!(
            "Command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Execute a script with credential env vars injected into the subprocess.
pub(crate) async fn execute_script_with_env(
    interpreter: &str,
    script: &str,
    args: &serde_json::Value,
    env: HashMap<String, String>,
) -> Result<String> {
    use std::process::Command;

    let temp_dir = std::env::temp_dir();
    let unique_name = format!(
        "zeus_skill_{}_{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let script_path = temp_dir.join(unique_name);
    std::fs::write(&script_path, script)?;

    let mut cmd = Command::new(interpreter);
    cmd.arg(&script_path);
    // Inject credentials as env vars (child process only)
    cmd.envs(&env);
    // Pass args as SKILL_* env vars
    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            let env_value = match value {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            cmd.env(format!("SKILL_{}", key.to_uppercase()), env_value);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| Error::Skill(format!("Failed to execute script: {}", e)))?;

    let _ = std::fs::remove_file(&script_path);

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::Skill(format!(
            "Script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Execute a script
pub(crate) async fn execute_script(
    interpreter: &str,
    script: &str,
    args: &serde_json::Value,
) -> Result<String> {
    use std::process::Command;

    // Write script to temp file with unique name to prevent symlink attacks
    let temp_dir = std::env::temp_dir();
    let unique_name = format!(
        "zeus_skill_{}_{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let script_path = temp_dir.join(unique_name);
    std::fs::write(&script_path, script)?;

    let mut cmd = Command::new(interpreter);
    cmd.arg(&script_path);

    // Pass args as environment variables
    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            let env_value = match value {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            cmd.env(format!("SKILL_{}", key.to_uppercase()), env_value);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| Error::Skill(format!("Failed to execute script: {}", e)))?;

    // Clean up
    let _ = std::fs::remove_file(&script_path);

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::Skill(format!(
            "Script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_md() {
        let content = r#"# Test Skill

A test skill for testing.

## Version: 1.0.0

## Author: Test Author

## System Prompt
You are a test assistant.

## Tools
- hello: Say hello

## Permissions
- network
"#;

        let skill = parse_skill_md(content, PathBuf::from(".")).expect("should parse successfully");
        assert_eq!(skill.name, "Test Skill");
        assert_eq!(skill.version, "1.0.0");
        assert_eq!(skill.author, Some("Test Author".to_string()));
        assert_eq!(skill.tools.len(), 1);
        assert_eq!(skill.tools[0].name, "hello");
    }

    #[test]
    fn test_skill_manager_default() {
        let manager = SkillManager::default();
        assert!(manager.skills.is_empty());
    }

    #[test]
    fn test_get_summary_empty() {
        let manager = SkillManager::new(PathBuf::from("/tmp/no-skills"));
        let summary = manager.get_summary();
        assert_eq!(summary, "No skills loaded.");
    }

    #[test]
    fn test_get_summary_with_skills() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "git-helper".to_string(),
            Skill {
                name: "git-helper".to_string(),
                description: "Git workflow automation".to_string(),
                version: "1.0.0".to_string(),
                author: Some("Zeus".to_string()),
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/git-helper"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );
        manager.skills.insert(
            "code-review".to_string(),
            Skill {
                name: "code-review".to_string(),
                description: "Automated code review".to_string(),
                version: "0.2.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/code-review"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let summary = manager.get_summary();
        assert!(summary.starts_with("Available skills:\n"));
        assert!(summary.contains("git-helper"));
        assert!(summary.contains("Git workflow automation"));
        assert!(summary.contains("code-review"));
        assert!(summary.contains("Automated code review"));
        assert!(summary.contains("Use read_file on a skill's path to load its full instructions."));
    }

    #[test]
    fn test_get_summary_with_empty_description() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "bare-skill".to_string(),
            Skill {
                name: "bare-skill".to_string(),
                description: String::new(),
                version: "0.1.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/bare-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let summary = manager.get_summary();
        assert!(summary.contains("No description"));
    }

    #[test]
    fn test_skill_description_extraction() {
        // With frontmatter, description comes from frontmatter
        let content = r#"---
name: my-skill
description: This skill does amazing things.
---
# My Skill

Body content here.
"#;
        let skill = parse_skill_md(content, PathBuf::from("/tmp/my-skill"))
            .expect("should parse successfully");
        assert_eq!(skill.description, "This skill does amazing things.");

        // Without frontmatter, text after # heading becomes system_prompt (OpenClaw style)
        // Description stays empty unless a non-heading, non-system-prompt line exists
        let content_no_desc = r#"# Minimal Skill

## System Prompt
Do stuff.
"#;
        let skill2 = parse_skill_md(content_no_desc, PathBuf::from("/tmp/minimal"))
            .expect("should parse successfully");
        assert_eq!(skill2.description, "");
    }

    #[test]
    fn test_uninstall_options_default() {
        let opts = UninstallOptions::default();
        assert!(!opts.dry_run);
        assert!(!opts.force);
        assert!(!opts.keep_files);
    }

    #[test]
    fn test_uninstall_dry_run() {
        let tmp = std::env::temp_dir().join("zeus_test_uninstall_dry");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).expect("should create directory");
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# My Skill\nTest.")
            .expect("should write file");

        let mut manager = SkillManager::new(tmp.clone());
        manager.skills.insert(
            "my-skill".to_string(),
            Skill {
                name: "my-skill".to_string(),
                description: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: tmp.join("my-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let opts = UninstallOptions {
            dry_run: true,
            ..Default::default()
        };
        manager
            .uninstall_with_options("my-skill", &opts)
            .expect("uninstall_with_options should succeed");

        // Skill should still exist (dry-run)
        assert!(tmp.join("my-skill/SKILL.md").exists());
        assert!(manager.skills.contains_key("my-skill"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_keep_files() {
        let tmp = std::env::temp_dir().join("zeus_test_uninstall_keep");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).expect("should create directory");
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# My Skill\nTest.")
            .expect("should write file");

        let mut manager = SkillManager::new(tmp.clone());
        manager.skills.insert(
            "my-skill".to_string(),
            Skill {
                name: "my-skill".to_string(),
                description: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: tmp.join("my-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let opts = UninstallOptions {
            keep_files: true,
            ..Default::default()
        };
        manager
            .uninstall_with_options("my-skill", &opts)
            .expect("uninstall_with_options should succeed");

        // Files should still exist, but skill removed from registry
        assert!(tmp.join("my-skill/SKILL.md").exists());
        assert!(!manager.skills.contains_key("my-skill"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_full_remove() {
        let tmp = std::env::temp_dir().join("zeus_test_uninstall_full");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).expect("should create directory");
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# My Skill\nTest.")
            .expect("should write file");

        let mut manager = SkillManager::new(tmp.clone());
        manager.skills.insert(
            "my-skill".to_string(),
            Skill {
                name: "my-skill".to_string(),
                description: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: tmp.join("my-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        manager
            .uninstall_with_options("my-skill", &UninstallOptions::default())
            .expect("default should succeed");

        // Both removed
        assert!(!tmp.join("my-skill").exists());
        assert!(!manager.skills.contains_key("my-skill"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_not_found() {
        let tmp = std::env::temp_dir().join("zeus_test_uninstall_notfound");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("should create directory");

        let mut manager = SkillManager::new(tmp.clone());
        let result = manager.uninstall_with_options("nonexistent", &UninstallOptions::default());
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_preview() {
        let tmp = std::env::temp_dir().join("zeus_test_uninstall_preview");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).expect("should create directory");
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# My Skill\nA test skill.")
            .expect("should write file");
        std::fs::write(tmp.join("my-skill/helper.sh"), "#!/bin/sh\necho hi")
            .expect("should write file");

        let mut manager = SkillManager::new(tmp.clone());
        manager.skills.insert(
            "my-skill".to_string(),
            Skill {
                name: "my-skill".to_string(),
                description: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: tmp.join("my-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let preview = manager
            .uninstall_preview("my-skill")
            .expect("uninstall_preview should succeed");
        assert_eq!(preview.name, "my-skill");
        assert_eq!(preview.files.len(), 2);
        assert!(preview.total_size > 0);
        assert!(preview.loaded);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nname: test-skill\nversion: 2.0.0\n---\n# Test Skill\nBody here.";
        let (fm, body) = openclaw::parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "test-skill");
        assert_eq!(fm.get("version").unwrap(), "2.0.0");
        assert!(body.starts_with("# Test Skill"));
    }

    #[test]
    fn test_parse_frontmatter_none() {
        let content = "# No Frontmatter\nJust markdown.";
        let (fm, body) = openclaw::parse_frontmatter(content);
        assert!(fm.is_empty());
        assert!(body.contains("# No Frontmatter"));
    }

    #[test]
    fn test_parse_frontmatter_json_metadata() {
        let content = r#"---
name: nano-pdf
description: PDF manipulation skill
metadata: {"openclaw": {"requires": {"bins": ["pdftotext"]}, "install": [{"kind": "brew", "formula": "poppler", "bins": ["pdftotext"]}]}}
---
# nano-pdf

Convert and manipulate PDF files.
"#;
        let (fm, body) = openclaw::parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "nano-pdf");
        let meta_str = fm.get("metadata").unwrap();
        assert!(meta_str.contains("openclaw"));

        let meta = openclaw::resolve_metadata(&fm);
        assert!(meta.is_some());
        let meta = meta.unwrap();
        let reqs = meta.requires.unwrap();
        assert_eq!(reqs.bins, Some(vec!["pdftotext".to_string()]));
        let installs = meta.install.unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].kind, "brew");
        assert_eq!(installs[0].formula.as_deref(), Some("poppler"));

        assert!(body.starts_with("# nano-pdf"));
    }

    #[test]
    fn test_parse_skill_md_with_frontmatter() {
        let content = r#"---
name: himalaya
description: Email management via himalaya CLI
user-invocable: true
command-dispatch: tool
command-tool: shell
command-arg-mode: raw
metadata: {"openclaw": {"requires": {"bins": ["himalaya"]}, "primaryEnv": "HIMALAYA_CONFIG", "homepage": "https://pimalaya.org/himalaya/"}}
---
# himalaya

You are a skill that manages email through the himalaya CLI tool.
Always use himalaya commands for reading, sending, and managing emails.
"#;
        let skill = parse_skill_md(content, PathBuf::from("/tmp/himalaya"))
            .expect("should parse frontmatter skill");
        assert_eq!(skill.name, "himalaya");
        assert_eq!(skill.description, "Email management via himalaya CLI");
        assert!(skill.invocation.user_invocable);
        assert!(!skill.invocation.disable_model_invocation);

        let dispatch = skill.command_dispatch.unwrap();
        assert_eq!(dispatch.kind, "tool");
        assert_eq!(dispatch.tool_name, "shell");
        assert_eq!(dispatch.arg_mode, Some("raw".to_string()));

        let meta = skill.metadata.unwrap();
        assert_eq!(meta.primary_env.as_deref(), Some("HIMALAYA_CONFIG"));
        assert_eq!(
            meta.homepage.as_deref(),
            Some("https://pimalaya.org/himalaya/")
        );
        let reqs = meta.requires.unwrap();
        assert_eq!(reqs.bins, Some(vec!["himalaya".to_string()]));

        assert!(skill.system_prompt.contains("himalaya CLI tool"));
    }

    #[test]
    fn test_invocation_policy_defaults() {
        let fm = HashMap::new();
        let inv = openclaw::resolve_invocation_policy(&fm);
        assert!(inv.user_invocable);
        assert!(!inv.disable_model_invocation);
    }

    #[test]
    fn test_invocation_policy_disabled() {
        let mut fm = HashMap::new();
        fm.insert("user-invocable".to_string(), "false".to_string());
        fm.insert("disable-model-invocation".to_string(), "true".to_string());
        let inv = openclaw::resolve_invocation_policy(&fm);
        assert!(!inv.user_invocable);
        assert!(inv.disable_model_invocation);
    }

    #[test]
    fn test_sanitize_command_name() {
        assert_eq!(sanitize_command_name("My Cool Skill!"), "my_cool_skill");
        assert_eq!(sanitize_command_name("git-helper"), "git_helper");
        assert_eq!(sanitize_command_name("___padded___"), "padded");
        assert_eq!(sanitize_command_name(""), "skill");
        // Long names get truncated
        let long = "a".repeat(50);
        assert_eq!(sanitize_command_name(&long).len(), SKILL_COMMAND_MAX_LENGTH);
    }

    #[test]
    fn test_build_command_specs() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "git-helper".to_string(),
            Skill {
                name: "git-helper".to_string(),
                description: "Git workflow automation".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/git-helper"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: Some(CommandDispatchSpec {
                    kind: "tool".to_string(),
                    tool_name: "shell".to_string(),
                    arg_mode: Some("raw".to_string()),
                }),
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );
        manager.skills.insert(
            "hidden-skill".to_string(),
            Skill {
                name: "hidden-skill".to_string(),
                description: "Not user-invocable".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/hidden-skill"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy {
                    user_invocable: false,
                    disable_model_invocation: false,
                },
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let specs = manager.build_command_specs();
        // hidden-skill should be excluded
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "git_helper");
        assert_eq!(specs[0].skill_name, "git-helper");
        assert!(specs[0].dispatch.is_some());
    }

    #[test]
    fn test_build_command_specs_skill_key_override() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "nano-pdf".to_string(),
            Skill {
                name: "nano-pdf".to_string(),
                description: "PDF tools".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: String::new(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/nano-pdf"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: Some(OpenClawMetadata {
                    skill_key: Some("pdf".to_string()),
                    ..Default::default()
                }),
                frontmatter: HashMap::new(),
                read_when: vec![],
            },
        );

        let specs = manager.build_command_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "pdf");
        assert_eq!(specs[0].skill_name, "nano-pdf");
    }

    #[test]
    fn test_openclaw_metadata_with_os_and_env() {
        let mut fm = HashMap::new();
        fm.insert("metadata".to_string(), r#"{"openclaw": {"os": ["macos", "linux"], "requires": {"env": ["OPENAI_API_KEY"], "config": ["~/.config/app"]}, "always": true, "emoji": "📧"}}"#.to_string());
        let meta = openclaw::resolve_metadata(&fm).unwrap();
        assert_eq!(meta.always, Some(true));
        assert_eq!(meta.emoji.as_deref(), Some("📧"));
        let os = meta.os.unwrap();
        assert_eq!(os, vec!["macos", "linux"]);
        let reqs = meta.requires.unwrap();
        assert_eq!(reqs.env, Some(vec!["OPENAI_API_KEY".to_string()]));
        assert_eq!(reqs.config, Some(vec!["~/.config/app".to_string()]));
    }

    #[test]
    fn test_dispatch_resolution() {
        // No dispatch fields
        let fm = HashMap::new();
        assert!(openclaw::resolve_command_dispatch(&fm).is_none());

        // With dispatch fields
        let mut fm2 = HashMap::new();
        fm2.insert("command-dispatch".to_string(), "tool".to_string());
        fm2.insert("command-tool".to_string(), "shell".to_string());
        let dispatch = openclaw::resolve_command_dispatch(&fm2).unwrap();
        assert_eq!(dispatch.kind, "tool");
        assert_eq!(dispatch.tool_name, "shell");

        // Unsupported dispatch kind
        let mut fm3 = HashMap::new();
        fm3.insert("command-dispatch".to_string(), "webhook".to_string());
        assert!(openclaw::resolve_command_dispatch(&fm3).is_none());
    }

    #[test]
    fn test_read_when_parsed_from_frontmatter() {
        let content = "---\nname: git-helper\ndescription: Git helper\nread_when:\n  - git commit\n  - pull request\n  - branch\n---\n# git-helper\nUse git tools.\n";
        let skill =
            parse_skill_md(content, PathBuf::from("/tmp/git-helper")).expect("should parse");
        assert_eq!(
            skill.read_when,
            vec!["git commit", "pull request", "branch"]
        );
    }

    #[test]
    fn test_read_when_empty_when_absent() {
        let content = "# Simple\n\n## System Prompt\nDo things.\n";
        let skill = parse_skill_md(content, PathBuf::from("/tmp/simple")).expect("should parse");
        assert!(skill.read_when.is_empty());
    }

    #[test]
    fn test_find_triggered_skills_match() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "git-helper".to_string(),
            Skill {
                name: "git-helper".to_string(),
                description: "Git helper".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: "Use git for version control.".to_string(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/git-helper"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec!["git commit".to_string(), "pull request".to_string()],
            },
        );
        manager.skills.insert(
            "docker".to_string(),
            Skill {
                name: "docker".to_string(),
                description: "Docker helper".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: "Use docker for containers.".to_string(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/docker"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec!["docker".to_string(), "container".to_string()],
            },
        );

        // Should match git-helper
        let triggered = manager.find_triggered_skills("How do I git commit this?");
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].name, "git-helper");

        // Should match docker
        let triggered = manager.find_triggered_skills("Start a docker container");
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].name, "docker");

        // Case-insensitive
        let triggered = manager.find_triggered_skills("Create a PULL REQUEST please");
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].name, "git-helper");

        // No match
        let triggered = manager.find_triggered_skills("What is the weather today?");
        assert!(triggered.is_empty());
    }

    #[test]
    fn test_get_triggered_context_none_when_no_match() {
        let manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        assert!(manager.get_triggered_context("anything").is_none());
    }

    #[test]
    fn test_get_triggered_context_injects_system_prompt() {
        let mut manager = SkillManager::new(PathBuf::from("/tmp/skills"));
        manager.skills.insert(
            "git-helper".to_string(),
            Skill {
                name: "git-helper".to_string(),
                description: "Git helper".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                system_prompt: "Always use git commands for version control operations."
                    .to_string(),
                tools: vec![],
                permissions: vec![],
                path: PathBuf::from("/tmp/skills/git-helper"),
                raw_content: String::new(),
                invocation: SkillInvocationPolicy::default(),
                command_dispatch: None,
                metadata: None,
                frontmatter: HashMap::new(),
                read_when: vec!["git".to_string()],
            },
        );

        let ctx = manager
            .get_triggered_context("git status shows untracked files")
            .expect("should return context");
        assert!(ctx.contains("Auto-Activated Skills"));
        assert!(ctx.contains("git-helper"));
        assert!(ctx.contains("Always use git commands"));
    }

    #[test]
    fn test_legacy_markdown_still_parses() {
        // Ensure backwards compatibility with old-style SKILL.md (no frontmatter)
        let content = r#"# Legacy Skill

A classic skill definition.

## System Prompt
You are a legacy helper.

## Tools
- search: Search for things
- fetch: Fetch a URL

## Permissions
- network
- filesystem
"#;
        let skill = parse_skill_md(content, PathBuf::from("/tmp/legacy"))
            .expect("legacy format should still parse");
        assert_eq!(skill.name, "Legacy Skill");
        assert_eq!(skill.tools.len(), 2);
        assert_eq!(skill.permissions, vec!["network", "filesystem"]);
        assert!(skill.system_prompt.contains("legacy helper"));
        assert!(skill.metadata.is_none());
        assert!(skill.frontmatter.is_empty());
    }

    /// Helper: write a minimal valid SKILL.md into a directory.
    fn write_skill(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let body = format!(
            "# {name}\n\nA test skill.\n\n## Version: 1.0.0\n\n## System Prompt\nDo {name}.\n\n## Tools\n- run: do work\n",
            name = name
        );
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[tokio::test]
    async fn load_extra_dir_flat_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path().join("primary");
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&primary).unwrap();
        write_skill(&workspace.join("alpha"), "Alpha");
        write_skill(&workspace.join("beta"), "Beta");

        let mut mgr = SkillManager::new(primary);
        let p = mgr.load_all().await.unwrap();
        assert_eq!(p, 0, "primary is empty");
        let w = mgr.load_extra_dir(&workspace, false).await.unwrap();
        assert_eq!(w, 2, "should load two flat-layout skills");
        assert!(mgr.get("Alpha").is_some());
        assert!(mgr.get("Beta").is_some());
    }

    #[tokio::test]
    async fn load_extra_dir_nested_community_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join(".community_skills");
        // Layout: .community_skills/<repo>/skills/<skill>/SKILL.md
        write_skill(&community.join("repo1").join("skills").join("gamma"), "Gamma");
        write_skill(&community.join("repo1").join("skills").join("delta"), "Delta");
        write_skill(&community.join("repo2").join("skills").join("epsilon"), "Epsilon");

        let mut mgr = SkillManager::new(tmp.path().join("primary"));
        let n = mgr.load_extra_dir(&community, true).await.unwrap();
        assert_eq!(n, 3, "should walk repo/skills/* across all repos");
        assert!(mgr.get("Gamma").is_some());
        assert!(mgr.get("Delta").is_some());
        assert!(mgr.get("Epsilon").is_some());
    }

    #[tokio::test]
    async fn load_extra_dir_missing_dir_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = SkillManager::new(tmp.path().join("primary"));
        let n = mgr
            .load_extra_dir(&tmp.path().join("does_not_exist"), false)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn load_extra_dir_overrides_existing_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path().join("primary");
        let overlay = tmp.path().join("overlay");
        write_skill(&primary.join("shared"), "Shared");
        write_skill(&overlay.join("shared"), "Shared");

        let mut mgr = SkillManager::new(primary);
        assert_eq!(mgr.load_all().await.unwrap(), 1);
        // Overlay re-loads same name → still one skill, last-write-wins.
        assert_eq!(mgr.load_extra_dir(&overlay, false).await.unwrap(), 1);
        assert_eq!(mgr.list().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Progressive-disclosure contract tests (GAP#1).
    //
    // These lock the three levels against regression:
    //   L1 (get_summary)            — never carries a skill body.
    //   L2 (get_triggered_context)  — body only on `read_when` match, bounded.
    //   L3 (raw_content)            — never reaches any prompt-bound output.
    // -----------------------------------------------------------------------

    /// Build a skill with a body long enough to exceed the L1 tagline and L2
    /// body ceilings, plus a distinctive `raw_content` marker for L3 checks.
    fn disclosure_skill(name: &str, read_when: Vec<String>) -> Skill {
        let long_body = format!("BODY-MARKER {}", "x".repeat(L2_BODY_CHARS + 500));
        let long_desc = format!("DESC-MARKER {}", "y".repeat(L1_TAGLINE_CHARS + 200));
        Skill {
            name: name.to_string(),
            description: long_desc,
            version: "1.0.0".to_string(),
            author: None,
            system_prompt: long_body,
            tools: vec![],
            permissions: vec![],
            path: PathBuf::from(format!("/tmp/skills/{name}")),
            raw_content: "RAW-CONTENT-L3-SECRET".to_string(),
            invocation: SkillInvocationPolicy::default(),
            command_dispatch: None,
            metadata: None,
            frontmatter: HashMap::new(),
            read_when,
        }
    }

    #[test]
    fn test_l1_caps_description_to_tagline() {
        // tagline() collapses to first line and caps at L1_TAGLINE_CHARS,
        // appending an ellipsis when truncated.
        let long = format!("first line {}", "z".repeat(L1_TAGLINE_CHARS + 50));
        let tag = SkillManager::tagline(&long);
        assert!(tag.chars().count() <= L1_TAGLINE_CHARS);
        assert!(tag.ends_with('…'), "truncated tagline must signal more");

        // A short, single-line description passes through unchanged.
        assert_eq!(SkillManager::tagline("short desc"), "short desc");

        // Multi-line collapses to the first line only — body never leaks.
        let multi = "headline\nhidden second line\nhidden third";
        let tag = SkillManager::tagline(multi);
        assert_eq!(tag, "headline");
        assert!(!tag.contains("hidden"));

        // Empty stays empty (caller substitutes "No description").
        assert_eq!(SkillManager::tagline("   "), "");
    }

    #[test]
    fn test_l1_summary_never_carries_body() {
        // CONTRACT: L1 (get_summary) must never contain a skill's body or its
        // L3 raw_content — only the name, path, and capped tagline.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills
            .insert("big".to_string(), disclosure_skill("big", vec![]));

        let summary = mgr.get_summary();
        assert!(summary.contains("big"));
        // The body marker (system_prompt) must NOT appear in L1.
        assert!(
            !summary.contains("BODY-MARKER"),
            "L1 leaked the skill body"
        );
        // L3 raw_content must NOT appear in L1.
        assert!(
            !summary.contains("RAW-CONTENT-L3-SECRET"),
            "L1 leaked raw_content"
        );
        // The description appears only as a capped tagline, not in full.
        assert!(
            !summary.contains(&"y".repeat(L1_TAGLINE_CHARS + 1)),
            "L1 carried the uncapped description"
        );
    }

    #[test]
    fn test_l2_only_injects_on_read_when_match() {
        // CONTRACT: L2 (get_triggered_context) injects a body ONLY when the
        // skill's read_when keywords match the request.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills.insert(
            "deploy".to_string(),
            disclosure_skill("deploy", vec!["deploy".to_string()]),
        );

        // No keyword in the message → no triggered context at all.
        assert!(mgr.get_triggered_context("write some docs").is_none());

        // Keyword present → body is injected.
        let ctx = mgr
            .get_triggered_context("please deploy the service")
            .expect("matching read_when must trigger L2");
        assert!(ctx.contains("BODY-MARKER"), "L2 must inject the body on match");
    }

    #[test]
    fn test_l2_body_is_bounded_with_pointer() {
        // CONTRACT: L2 caps the body at L2_BODY_CHARS and appends a read_file
        // pointer to the full content — generous ceiling, not a starve.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills.insert(
            "deploy".to_string(),
            disclosure_skill("deploy", vec!["deploy".to_string()]),
        );

        let ctx = mgr
            .get_triggered_context("deploy now")
            .expect("must trigger");
        // Pointer appended when truncated.
        assert!(
            ctx.contains("[truncated — read_file"),
            "bounded body must append a read_file pointer"
        );
        assert!(
            ctx.contains("/tmp/skills/deploy"),
            "pointer must name the skill path"
        );
        // The full body did NOT all make it through (it exceeded the ceiling).
        assert!(
            !ctx.contains(&"x".repeat(L2_BODY_CHARS + 1)),
            "L2 body exceeded its ceiling"
        );

        // A body UNDER the ceiling passes through whole, no pointer.
        let mut small = disclosure_skill("small", vec!["small".to_string()]);
        small.system_prompt = "short body".to_string();
        mgr.skills.insert("small".to_string(), small);
        let ctx2 = mgr.get_triggered_context("small task").expect("trigger");
        assert!(ctx2.contains("short body"));
        // 'small' is under the ceiling so its line carries no pointer; the
        // pointer present in ctx2 (if any) belongs to 'deploy', not 'small'.
        assert!(!ctx2.contains("read_file /tmp/skills/small"));
    }

    #[test]
    fn test_l3_raw_content_never_reaches_any_prompt() {
        // CONTRACT: L3 (raw_content) must never appear in L1 or L2 output —
        // it is `#[serde(skip)]` and reachable only via read_file.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills.insert(
            "deploy".to_string(),
            disclosure_skill("deploy", vec!["deploy".to_string()]),
        );

        let l1 = mgr.get_summary();
        let l2 = mgr
            .get_triggered_context("deploy please")
            .expect("trigger");

        assert!(
            !l1.contains("RAW-CONTENT-L3-SECRET"),
            "L3 raw_content leaked into L1"
        );
        assert!(
            !l2.contains("RAW-CONTENT-L3-SECRET"),
            "L3 raw_content leaked into L2"
        );
    }

    // ── GAP#2: persona default_skills → force-active wiring ──────────────

    #[test]
    fn test_enable_skills_forces_non_keyword_skill_active() {
        // A skill with NO read_when never triggers by keyword. Force-activating
        // it must make it appear in the triggered context regardless of message.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills
            .insert("git".to_string(), disclosure_skill("git", vec![]));

        // Sanity: not triggered before enabling (no read_when, no match).
        assert!(
            mgr.get_triggered_context("totally unrelated message")
                .is_none(),
            "skill with no read_when must not trigger by keyword"
        );

        mgr.enable_skills(&["git".to_string()]);

        let ctx = mgr
            .get_triggered_context("totally unrelated message")
            .expect("force-active skill must appear in triggered context");
        assert!(ctx.contains("### git"), "force-active skill missing from L2");
    }

    #[test]
    fn test_enable_skills_unions_with_keyword_triggered() {
        // Force-active ∪ keyword-triggered, deduped — a skill that is BOTH
        // force-active and keyword-matching appears exactly once.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills.insert(
            "deploy".to_string(),
            disclosure_skill("deploy", vec!["deploy".to_string()]),
        );
        mgr.skills
            .insert("git".to_string(), disclosure_skill("git", vec![]));

        mgr.enable_skills(&["git".to_string()]);

        let triggered = mgr.find_triggered_skills("please deploy now");
        let names: Vec<&str> = triggered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["deploy", "git"],
            "union must contain both keyword-triggered (deploy) and force-active (git)"
        );

        // A force-active skill that ALSO keyword-matches is not double-counted.
        mgr.enable_skills(&["deploy".to_string()]);
        let again = mgr.find_triggered_skills("please deploy now");
        assert_eq!(again.len(), 2, "force-active + keyword overlap must dedupe");
    }

    #[test]
    fn test_enable_skills_ignores_unknown_names() {
        // Nail #3: names that aren't loaded are silently ignored — never force
        // a skill that doesn't exist. No panic, no phantom entry.
        let mut mgr = SkillManager::new(PathBuf::from("/tmp/skills"));
        mgr.skills
            .insert("git".to_string(), disclosure_skill("git", vec![]));

        mgr.enable_skills(&[
            "git".to_string(),
            "does-not-exist".to_string(),
            "also-missing".to_string(),
        ]);

        // Only the loaded skill is force-active; the two unknowns are no-ops.
        let triggered = mgr.find_triggered_skills("nothing matches here");
        let names: Vec<&str> = triggered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["git"], "unknown skill names must be no-ops");

        // Empty input is also a clean no-op.
        mgr.enable_skills(&[]);
        assert_eq!(
            mgr.find_triggered_skills("nothing matches here").len(),
            1,
            "empty enable_skills must not change the active set"
        );
    }
}
