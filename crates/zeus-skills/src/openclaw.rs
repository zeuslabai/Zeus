//! OpenClaw-Compatible Skill Format Parser
//!
//! Parses SKILL.md files following the OpenClaw specification:
//! - YAML frontmatter (`---` delimited) with structured metadata
//! - `metadata.openclaw` block (JSON inside YAML) for runtime requirements
//! - Environment variable, binary, and config gating
//! - Skill invocation policies (user-invocable, model-invocation, command dispatch)
//! - Install specifications for dependency management
//!
//! Reference: https://docs.openclaw.ai/tools/skills
//! Source: https://github.com/openclaw/clawhub/blob/main/docs/skill-format.md

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

// ============================================================================
// Frontmatter types
// ============================================================================

/// Raw parsed frontmatter — all values are strings (matching OpenClaw's ParsedSkillFrontmatter).
pub type ParsedFrontmatter = HashMap<String, String>;

/// OpenClaw skill metadata extracted from `metadata.openclaw` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenClawMetadata {
    /// If true, skill is always active (no gating).
    #[serde(default)]
    pub always: Option<bool>,
    /// Override the skill invocation key.
    #[serde(default, alias = "skillKey")]
    pub skill_key: Option<String>,
    /// Main credential environment variable.
    #[serde(default, alias = "primaryEnv")]
    pub primary_env: Option<String>,
    /// Display emoji for the skill.
    #[serde(default)]
    pub emoji: Option<String>,
    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// OS restrictions (e.g., ["macos"], ["linux", "darwin"]).
    #[serde(default)]
    pub os: Option<Vec<String>>,
    /// Runtime requirements.
    #[serde(default)]
    pub requires: Option<RequirementsSpec>,
    /// Dependency install specifications.
    #[serde(default)]
    pub install: Option<Vec<InstallSpec>>,
}

/// Runtime requirements for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequirementsSpec {
    /// Environment variables that must be set.
    #[serde(default)]
    pub env: Option<Vec<String>>,
    /// CLI binaries that must ALL be present on PATH.
    #[serde(default)]
    pub bins: Option<Vec<String>>,
    /// CLI binaries where at least ONE must be present.
    #[serde(default, alias = "anyBins")]
    pub any_bins: Option<Vec<String>>,
    /// Config paths that must be truthy.
    #[serde(default)]
    pub config: Option<Vec<String>>,
}

/// Dependency install specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSpec {
    /// Unique ID for this install option.
    #[serde(default)]
    pub id: Option<String>,
    /// Install method: brew, node, go, uv, download.
    pub kind: String,
    /// Human-readable label.
    #[serde(default)]
    pub label: Option<String>,
    /// Binaries provided by this install.
    #[serde(default)]
    pub bins: Option<Vec<String>>,
    /// OS restrictions for this specific installer.
    #[serde(default)]
    pub os: Option<Vec<String>>,
    /// Homebrew formula name.
    #[serde(default)]
    pub formula: Option<String>,
    /// npm/node package name.
    #[serde(default)]
    pub package: Option<String>,
    /// Go module path.
    #[serde(default)]
    pub module: Option<String>,
    /// Download URL.
    #[serde(default)]
    pub url: Option<String>,
}

/// Skill invocation policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInvocationPolicy {
    /// Whether the skill is exposed as a slash command (default: true).
    pub user_invocable: bool,
    /// Whether to exclude from the model prompt (default: false).
    pub disable_model_invocation: bool,
}

impl Default for SkillInvocationPolicy {
    fn default() -> Self {
        Self {
            user_invocable: true,
            disable_model_invocation: false,
        }
    }
}

/// Command dispatch specification for deterministic tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDispatchSpec {
    /// Dispatch kind (currently only "tool").
    pub kind: String,
    /// Tool name to invoke.
    pub tool_name: String,
    /// How to forward args ("raw" = unprocessed string).
    #[serde(default)]
    pub arg_mode: Option<String>,
}

/// A fully parsed OpenClaw skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawSkill {
    /// Skill name (from frontmatter `name` field).
    pub name: String,
    /// Slug (filesystem-safe name, from directory name or sanitized name).
    pub slug: String,
    /// Description (from frontmatter `description` field).
    pub description: String,
    /// Version string.
    #[serde(default)]
    pub version: Option<String>,
    /// OpenClaw metadata block.
    #[serde(default)]
    pub metadata: Option<OpenClawMetadata>,
    /// Invocation policy.
    #[serde(default)]
    pub invocation: SkillInvocationPolicy,
    /// Command dispatch spec (if `command-dispatch: tool`).
    #[serde(default)]
    pub command_dispatch: Option<CommandDispatchSpec>,
    /// Allowed tools (from `allowed-tools` frontmatter).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// The markdown body (instructions for the agent).
    pub instructions: String,
    /// Raw frontmatter key-value pairs.
    #[serde(skip)]
    pub raw_frontmatter: ParsedFrontmatter,
    /// Path to the skill directory.
    #[serde(skip)]
    pub path: PathBuf,
}

// ============================================================================
// Frontmatter parser
// ============================================================================

/// Parse YAML frontmatter from SKILL.md content.
///
/// Handles the `---` delimited block at the top of the file.
/// Falls back to line-based parsing if YAML parsing fails (matching OpenClaw behavior).
pub fn parse_frontmatter(content: &str) -> (ParsedFrontmatter, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");

    if !normalized.starts_with("---") {
        return (HashMap::new(), normalized);
    }

    // Find closing ---
    let rest = &normalized[3..];
    let end_idx = if let Some(idx) = rest.find("\n---") {
        idx
    } else {
        return (HashMap::new(), normalized);
    };

    let block = &rest[1..end_idx]; // skip the first newline after opening ---
    let body = &rest[end_idx + 4..]; // skip past \n---
    let body = body.strip_prefix('\n').unwrap_or(body);

    // Try YAML parsing first
    let yaml_result = parse_yaml_frontmatter(block);
    // Also do line-based parsing
    let line_result = parse_line_frontmatter(block);

    let frontmatter = match yaml_result {
        Some(yaml_map) => {
            // YAML parser handles trailing commas and produces valid JSON.
            // Prefer YAML-parsed values over line-based for structured data.
            // Only fall back to line-parsed for keys YAML missed.
            let mut merged = yaml_map;
            for (key, value) in &line_result {
                if !merged.contains_key(key) {
                    merged.insert(key.clone(), value.clone());
                }
            }
            merged
        }
        None => line_result,
    };

    (frontmatter, body.to_string())
}

/// Parse frontmatter block as YAML, coercing all values to strings.
fn parse_yaml_frontmatter(block: &str) -> Option<ParsedFrontmatter> {
    let parsed: serde_yaml::Value = serde_yaml::from_str(block).ok()?;
    let mapping = parsed.as_mapping()?;

    let mut result = HashMap::new();
    for (key, value) in mapping {
        let key_str = match key {
            serde_yaml::Value::String(s) => s.trim().to_string(),
            _ => continue,
        };
        if key_str.is_empty() {
            continue;
        }
        if let Some(coerced) = coerce_yaml_value(value) {
            result.insert(key_str, coerced);
        }
    }

    Some(result)
}

/// Coerce a YAML value to a string (matching OpenClaw's coerceFrontmatterValue).
fn coerce_yaml_value(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::String(s) => Some(s.trim().to_string()),
        serde_yaml::Value::Sequence(_) | serde_yaml::Value::Mapping(_) => {
            serde_json::to_string(value).ok()
        }
        _ => None,
    }
}

/// Line-based frontmatter parser (fallback, handles multi-line values).
fn parse_line_frontmatter(block: &str) -> ParsedFrontmatter {
    let mut result = HashMap::new();
    let lines: Vec<&str> = block.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Match key: value pattern
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            if key.is_empty()
                || !key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                i += 1;
                continue;
            }

            let inline_value = line[colon_pos + 1..].trim();

            if inline_value.is_empty() && i + 1 < lines.len() {
                // Check for multi-line value (indented continuation)
                let next = lines[i + 1];
                if next.starts_with(' ') || next.starts_with('\t') {
                    let mut value_lines = Vec::new();
                    let mut j = i + 1;
                    while j < lines.len() {
                        let l = lines[j];
                        if !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') {
                            break;
                        }
                        value_lines.push(l);
                        j += 1;
                    }
                    let combined = value_lines.join("\n").trim().to_string();
                    if !combined.is_empty() {
                        result.insert(key.to_string(), combined);
                    }
                    i = j;
                    continue;
                }
            }

            let value = strip_quotes(inline_value);
            if !value.is_empty() {
                result.insert(key.to_string(), value);
            }
        }

        i += 1;
    }

    result
}

/// Strip surrounding quotes from a value string.
fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ============================================================================
// Metadata resolver
// ============================================================================

/// Resolve the `metadata.openclaw` block from parsed frontmatter.
///
/// Supports aliases: `metadata.openclaw`, `metadata.clawdbot`, `metadata.clawdis`.
pub fn resolve_metadata(frontmatter: &ParsedFrontmatter) -> Option<OpenClawMetadata> {
    let metadata_str = frontmatter.get("metadata")?;

    // The metadata field is a JSON string containing the openclaw block
    let metadata_obj: serde_json::Value = serde_json::from_str(metadata_str).ok()?;
    let metadata_map = metadata_obj.as_object()?;

    // Try openclaw, clawdbot, clawdis aliases
    let openclaw_block = metadata_map
        .get("openclaw")
        .or_else(|| metadata_map.get("clawdbot"))
        .or_else(|| metadata_map.get("clawdis"))?;

    serde_json::from_value(openclaw_block.clone()).ok()
}

/// Resolve skill invocation policy from frontmatter.
pub fn resolve_invocation_policy(frontmatter: &ParsedFrontmatter) -> SkillInvocationPolicy {
    let user_invocable = frontmatter
        .get("user-invocable")
        .map(|v| parse_bool(v, true))
        .unwrap_or(true);

    let disable_model = frontmatter
        .get("disable-model-invocation")
        .map(|v| parse_bool(v, false))
        .unwrap_or(false);

    SkillInvocationPolicy {
        user_invocable,
        disable_model_invocation: disable_model,
    }
}

/// Resolve command dispatch spec from frontmatter.
pub fn resolve_command_dispatch(frontmatter: &ParsedFrontmatter) -> Option<CommandDispatchSpec> {
    let dispatch = frontmatter.get("command-dispatch")?;
    if dispatch != "tool" {
        return None;
    }

    let tool_name = frontmatter.get("command-tool")?.clone();
    let arg_mode = frontmatter.get("command-arg-mode").cloned();

    Some(CommandDispatchSpec {
        kind: "tool".to_string(),
        tool_name,
        arg_mode,
    })
}

/// Resolve `read_when` trigger keywords from frontmatter.
///
/// OpenClaw skills declare auto-activation triggers as a YAML list:
/// ```yaml
/// read_when:
///   - git commit
///   - pull request
/// ```
/// YAML sequences are coerced to JSON arrays by `parse_yaml_frontmatter`.
/// Falls back to comma-separated string if JSON parse fails.
pub fn resolve_read_when(frontmatter: &ParsedFrontmatter) -> Vec<String> {
    let Some(val) = frontmatter
        .get("read_when")
        .or_else(|| frontmatter.get("readWhen"))
    else {
        return Vec::new();
    };
    // YAML sequences are coerced to JSON arrays by parse_yaml_frontmatter
    if let Ok(triggers) = serde_json::from_str::<Vec<String>>(val) {
        return triggers.into_iter().filter(|s| !s.is_empty()).collect();
    }
    // Fall back to comma-separated plain string
    val.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolve allowed-tools from frontmatter.
pub fn resolve_allowed_tools(frontmatter: &ParsedFrontmatter) -> Vec<String> {
    if let Some(tools_str) = frontmatter.get("allowed-tools") {
        // Try JSON array first
        if let Ok(tools) = serde_json::from_str::<Vec<String>>(tools_str) {
            return tools;
        }
        // Fall back to comma-separated
        return tools_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    Vec::new()
}

/// Parse a boolean from a frontmatter string value.
fn parse_bool(s: &str, default: bool) -> bool {
    match s.to_lowercase().trim() {
        "true" | "yes" | "1" => true,
        "false" | "no" | "0" => false,
        _ => default,
    }
}

// ============================================================================
// Full skill parser
// ============================================================================

/// Parse a SKILL.md file into a fully resolved OpenClawSkill.
///
/// Handles both OpenClaw format (YAML frontmatter) and legacy Zeus format
/// (markdown headings). Auto-detects based on `---` frontmatter presence.
pub fn parse_openclaw_skill(content: &str, path: PathBuf) -> Option<OpenClawSkill> {
    let (frontmatter, body) = parse_frontmatter(content);

    if frontmatter.is_empty() {
        // No frontmatter — not an OpenClaw skill
        return None;
    }

    let name = frontmatter.get("name")?.clone();
    let slug = slugify(&name);
    let description = frontmatter.get("description").cloned().unwrap_or_default();
    let version = frontmatter.get("version").cloned();
    let metadata = resolve_metadata(&frontmatter);
    let invocation = resolve_invocation_policy(&frontmatter);
    let command_dispatch = resolve_command_dispatch(&frontmatter);
    let allowed_tools = resolve_allowed_tools(&frontmatter);

    Some(OpenClawSkill {
        name,
        slug,
        description,
        version,
        metadata,
        invocation,
        command_dispatch,
        allowed_tools,
        instructions: body,
        raw_frontmatter: frontmatter,
        path,
    })
}

/// Convert a skill name to a URL-safe slug: ^[a-z0-9][a-z0-9-]*$
pub fn slugify(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse repeated dashes and trim
    let mut result = String::new();
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

// ============================================================================
// Gating — requirement checks
// ============================================================================

/// Result of checking if a skill's requirements are met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatingResult {
    /// Whether all requirements are met.
    pub eligible: bool,
    /// Missing environment variables.
    pub missing_env: Vec<String>,
    /// Missing required binaries (all must exist).
    pub missing_bins: Vec<String>,
    /// Whether anyBins requirement is met (at least one found).
    pub any_bin_satisfied: Option<bool>,
    /// Missing config paths.
    pub missing_config: Vec<String>,
    /// OS mismatch (if skill restricts to specific OS).
    pub os_mismatch: Option<String>,
    /// Human-readable summary.
    pub summary: String,
}

/// Check if a skill's runtime requirements are satisfied.
///
/// Validates:
/// - `requires.env`: all listed env vars must be set (non-empty)
/// - `requires.bins`: all listed binaries must be on PATH
/// - `requires.anyBins`: at least one listed binary must be on PATH
/// - `os`: current OS must match one of the listed platforms
/// - `always: true` bypasses all checks
pub fn check_requirements(metadata: &OpenClawMetadata) -> GatingResult {
    // If always=true, skip all checks
    if metadata.always == Some(true) {
        return GatingResult {
            eligible: true,
            missing_env: vec![],
            missing_bins: vec![],
            any_bin_satisfied: None,
            missing_config: vec![],
            os_mismatch: None,
            summary: "always=true, no gating".to_string(),
        };
    }

    let mut missing_env = Vec::new();
    let mut missing_bins = Vec::new();
    let mut any_bin_satisfied = None;
    let missing_config = Vec::new(); // Config checking deferred — needs config context
    let mut os_mismatch = None;

    // Check OS restriction
    if let Some(ref os_list) = metadata.os {
        let current_os = current_platform();
        if !os_list
            .iter()
            .any(|os| os == &current_os || os == "darwin" && current_os == "macos")
        {
            os_mismatch = Some(format!(
                "skill requires {:?}, current platform is '{}'",
                os_list, current_os
            ));
        }
    }

    // Check required environment variables
    if let Some(ref requires) = metadata.requires {
        if let Some(ref env_vars) = requires.env {
            for var in env_vars {
                match std::env::var(var) {
                    Ok(val) if !val.is_empty() => {}
                    _ => missing_env.push(var.clone()),
                }
            }
        }

        // Check required binaries (all must exist)
        if let Some(ref bins) = requires.bins {
            for bin in bins {
                if !is_binary_on_path(bin) {
                    missing_bins.push(bin.clone());
                }
            }
        }

        // Check anyBins (at least one must exist)
        if let Some(ref any_bins) = requires.any_bins
            && !any_bins.is_empty()
        {
            let found = any_bins.iter().any(|b| is_binary_on_path(b));
            any_bin_satisfied = Some(found);
        }
    }

    // Also check primaryEnv if set
    if let Some(ref primary) = metadata.primary_env {
        match std::env::var(primary) {
            Ok(val) if !val.is_empty() => {}
            _ => {
                if !missing_env.contains(primary) {
                    missing_env.push(primary.clone());
                }
            }
        }
    }

    let eligible = missing_env.is_empty()
        && missing_bins.is_empty()
        && any_bin_satisfied != Some(false)
        && missing_config.is_empty()
        && os_mismatch.is_none();

    let mut reasons = Vec::new();
    if !missing_env.is_empty() {
        reasons.push(format!("missing env: {}", missing_env.join(", ")));
    }
    if !missing_bins.is_empty() {
        reasons.push(format!("missing bins: {}", missing_bins.join(", ")));
    }
    if any_bin_satisfied == Some(false) {
        reasons.push("no anyBins found".to_string());
    }
    if let Some(ref mismatch) = os_mismatch {
        reasons.push(mismatch.clone());
    }

    let summary = if eligible {
        "all requirements met".to_string()
    } else {
        reasons.join("; ")
    };

    GatingResult {
        eligible,
        missing_env,
        missing_bins,
        any_bin_satisfied,
        missing_config,
        os_mismatch,
        summary,
    }
}

/// Get the current platform identifier (matching OpenClaw convention).
fn current_platform() -> String {
    if cfg!(target_os = "macos") {
        "darwin".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "windows") {
        "win32".to_string()
    } else if cfg!(target_os = "freebsd") {
        "freebsd".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

/// Check if a binary exists on PATH.
fn is_binary_on_path(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = Path::new(dir).join(name);
            if candidate.exists() {
                return true;
            }
        }
    }
    false
}

// ============================================================================
// Skill loading with precedence
// ============================================================================

/// Skill source with precedence level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillSource {
    /// Workspace-local skills (highest precedence).
    Workspace = 0,
    /// User-managed skills (~/.zeus/skills).
    Managed = 1,
    /// Bundled skills (lowest precedence).
    Bundled = 2,
    /// Extra directories (lowest precedence).
    Extra = 3,
}

/// Load skills from a directory, returning parsed OpenClaw skills.
pub fn load_skills_from_dir(dir: &Path, source: SkillSource) -> Vec<(OpenClawSkill, SkillSource)> {
    let mut skills = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return skills,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Try SKILL.md (case-insensitive)
        let skill_md = if path.join("SKILL.md").exists() {
            path.join("SKILL.md")
        } else if path.join("skill.md").exists() {
            path.join("skill.md")
        } else {
            continue;
        };

        match std::fs::read_to_string(&skill_md) {
            Ok(content) => {
                if let Some(skill) = parse_openclaw_skill(&content, path.clone()) {
                    skills.push((skill, source));
                } else {
                    warn!("Failed to parse OpenClaw skill at {}", skill_md.display());
                }
            }
            Err(e) => {
                warn!("Cannot read {}: {}", skill_md.display(), e);
            }
        }
    }

    skills
}

/// Load skills from multiple directories with precedence.
///
/// Follows OpenClaw loading order: workspace > managed > bundled > extra.
/// Higher-precedence skills override lower-precedence ones with the same slug.
pub fn load_skills_with_precedence(
    workspace_dir: Option<&Path>,
    managed_dir: &Path,
    bundled_dir: Option<&Path>,
    extra_dirs: &[PathBuf],
) -> Vec<(OpenClawSkill, SkillSource)> {
    let mut skill_map: HashMap<String, (OpenClawSkill, SkillSource)> = HashMap::new();

    // Load in reverse precedence order (lowest first, so higher overwrites)
    for extra_dir in extra_dirs {
        for (skill, source) in load_skills_from_dir(extra_dir, SkillSource::Extra) {
            skill_map.insert(skill.slug.clone(), (skill, source));
        }
    }

    if let Some(bundled) = bundled_dir {
        for (skill, source) in load_skills_from_dir(bundled, SkillSource::Bundled) {
            skill_map.insert(skill.slug.clone(), (skill, source));
        }
    }

    for (skill, source) in load_skills_from_dir(managed_dir, SkillSource::Managed) {
        skill_map.insert(skill.slug.clone(), (skill, source));
    }

    if let Some(workspace) = workspace_dir {
        for (skill, source) in load_skills_from_dir(workspace, SkillSource::Workspace) {
            skill_map.insert(skill.slug.clone(), (skill, source));
        }
    }

    let mut skills: Vec<_> = skill_map.into_values().collect();
    skills.sort_by(|a, b| a.0.slug.cmp(&b.0.slug));
    skills
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Frontmatter parsing --

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
---

# GitHub Skill

Use gh to interact with GitHub.
"#;
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "github");
        assert_eq!(
            fm.get("description").unwrap(),
            "GitHub operations via gh CLI"
        );
        assert!(body.contains("# GitHub Skill"));
    }

    #[test]
    fn test_parse_frontmatter_with_metadata() {
        let content = r#"---
name: discord
description: "Discord ops via the message tool"
metadata: { "openclaw": { "emoji": "🎮", "requires": { "config": ["channels.discord.token"] } } }
---

# Discord Skill
"#;
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "discord");
        let metadata_str = fm.get("metadata").unwrap();
        assert!(metadata_str.contains("openclaw"));
        assert!(body.contains("# Discord Skill"));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "# Just Markdown\n\nNo frontmatter here.";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_empty());
        assert!(body.contains("# Just Markdown"));
    }

    #[test]
    fn test_parse_frontmatter_multiline_metadata() {
        // Real OpenClaw skills use trailing commas in metadata (valid YAML, not strict JSON).
        // Our parser handles this: YAML parses first, then coerces to JSON string.
        let content = r#"---
name: github
description: "GitHub operations"
metadata:
  {
    "openclaw":
      {
        "emoji": "🐙",
        "requires": { "bins": ["gh"] },
      },
  }
---

# GitHub Skill
"#;
        let (fm, _body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "github");
        let metadata_str = fm.get("metadata").unwrap();
        // YAML parsing coerces trailing-comma objects to valid JSON
        let parsed: serde_json::Value = serde_json::from_str(metadata_str)
            .expect("metadata should be valid JSON after YAML coercion");
        assert!(parsed.get("openclaw").is_some());
    }

    // -- Metadata resolution --

    #[test]
    fn test_resolve_metadata_basic() {
        let mut fm = HashMap::new();
        fm.insert(
            "metadata".to_string(),
            r#"{"openclaw":{"emoji":"🐙","requires":{"bins":["gh"]},"primaryEnv":"GITHUB_TOKEN"}}"#
                .to_string(),
        );

        let meta = resolve_metadata(&fm).unwrap();
        assert_eq!(meta.emoji, Some("🐙".to_string()));
        assert_eq!(meta.primary_env, Some("GITHUB_TOKEN".to_string()));
        let requires = meta.requires.unwrap();
        assert_eq!(requires.bins, Some(vec!["gh".to_string()]));
    }

    #[test]
    fn test_resolve_metadata_clawdbot_alias() {
        let mut fm = HashMap::new();
        fm.insert(
            "metadata".to_string(),
            r#"{"clawdbot":{"emoji":"🤖","always":true}}"#.to_string(),
        );

        let meta = resolve_metadata(&fm).unwrap();
        assert_eq!(meta.emoji, Some("🤖".to_string()));
        assert_eq!(meta.always, Some(true));
    }

    #[test]
    fn test_resolve_metadata_no_metadata() {
        let fm = HashMap::new();
        assert!(resolve_metadata(&fm).is_none());
    }

    #[test]
    fn test_resolve_metadata_invalid_json() {
        let mut fm = HashMap::new();
        fm.insert("metadata".to_string(), "not json".to_string());
        assert!(resolve_metadata(&fm).is_none());
    }

    // -- Invocation policy --

    #[test]
    fn test_invocation_policy_defaults() {
        let fm = HashMap::new();
        let policy = resolve_invocation_policy(&fm);
        assert!(policy.user_invocable);
        assert!(!policy.disable_model_invocation);
    }

    #[test]
    fn test_invocation_policy_custom() {
        let mut fm = HashMap::new();
        fm.insert("user-invocable".to_string(), "false".to_string());
        fm.insert("disable-model-invocation".to_string(), "true".to_string());

        let policy = resolve_invocation_policy(&fm);
        assert!(!policy.user_invocable);
        assert!(policy.disable_model_invocation);
    }

    // -- Command dispatch --

    #[test]
    fn test_command_dispatch() {
        let mut fm = HashMap::new();
        fm.insert("command-dispatch".to_string(), "tool".to_string());
        fm.insert("command-tool".to_string(), "shell".to_string());
        fm.insert("command-arg-mode".to_string(), "raw".to_string());

        let dispatch = resolve_command_dispatch(&fm).unwrap();
        assert_eq!(dispatch.kind, "tool");
        assert_eq!(dispatch.tool_name, "shell");
        assert_eq!(dispatch.arg_mode, Some("raw".to_string()));
    }

    #[test]
    fn test_command_dispatch_no_dispatch() {
        let fm = HashMap::new();
        assert!(resolve_command_dispatch(&fm).is_none());
    }

    // -- Allowed tools --

    #[test]
    fn test_allowed_tools_json_array() {
        let mut fm = HashMap::new();
        fm.insert(
            "allowed-tools".to_string(),
            r#"["message", "shell"]"#.to_string(),
        );

        let tools = resolve_allowed_tools(&fm);
        assert_eq!(tools, vec!["message", "shell"]);
    }

    #[test]
    fn test_allowed_tools_empty() {
        let fm = HashMap::new();
        let tools = resolve_allowed_tools(&fm);
        assert!(tools.is_empty());
    }

    // -- Slugify --

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("github"), "github");
        assert_eq!(slugify("My Cool Skill"), "my-cool-skill");
        assert_eq!(slugify("1password"), "1password");
        assert_eq!(slugify("bear-notes"), "bear-notes");
        assert_eq!(slugify("gh--issues"), "gh-issues");
    }

    // -- Full skill parsing --

    #[test]
    fn test_parse_openclaw_skill_github() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
metadata: {"openclaw":{"emoji":"🐙","requires":{"bins":["gh"]}}}
---

# GitHub Skill

Use the `gh` CLI to interact with GitHub.

## When to Use

- Checking PR status
- Creating issues
"#;
        let skill = parse_openclaw_skill(content, PathBuf::from("/skills/github")).unwrap();
        assert_eq!(skill.name, "github");
        assert_eq!(skill.slug, "github");
        assert_eq!(skill.description, "GitHub operations via gh CLI");
        assert!(skill.instructions.contains("# GitHub Skill"));

        let meta = skill.metadata.unwrap();
        assert_eq!(meta.emoji, Some("🐙".to_string()));
        let requires = meta.requires.unwrap();
        assert_eq!(requires.bins, Some(vec!["gh".to_string()]));
    }

    #[test]
    fn test_parse_openclaw_skill_discord() {
        let content = r#"---
name: discord
description: "Discord ops via the message tool"
metadata: {"openclaw":{"emoji":"🎮","requires":{"config":["channels.discord.token"]}}}
allowed-tools: ["message"]
---

# Discord

Use the `message` tool.
"#;
        let skill = parse_openclaw_skill(content, PathBuf::from("/skills/discord")).unwrap();
        assert_eq!(skill.name, "discord");
        assert_eq!(skill.allowed_tools, vec!["message"]);
    }

    #[test]
    fn test_parse_openclaw_skill_no_frontmatter() {
        let content = "# Legacy Skill\n\nNo frontmatter.";
        assert!(parse_openclaw_skill(content, PathBuf::from("/skills/legacy")).is_none());
    }

    // -- Gating --

    #[test]
    fn test_gating_always_true() {
        let meta = OpenClawMetadata {
            always: Some(true),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(result.eligible);
    }

    #[test]
    fn test_gating_missing_env() {
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                env: Some(vec!["NONEXISTENT_VAR_12345".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(!result.eligible);
        assert!(
            result
                .missing_env
                .contains(&"NONEXISTENT_VAR_12345".to_string())
        );
    }

    #[test]
    fn test_gating_env_set() {
        // SAFETY: test-only, single-threaded test runner
        unsafe {
            std::env::set_var("ZEUS_TEST_GATING_VAR", "present");
        }
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                env: Some(vec!["ZEUS_TEST_GATING_VAR".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(result.eligible);
        unsafe {
            std::env::remove_var("ZEUS_TEST_GATING_VAR");
        }
    }

    #[test]
    fn test_gating_bins_found() {
        // `ls` should always be on PATH
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                bins: Some(vec!["ls".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(result.eligible);
    }

    #[test]
    fn test_gating_bins_missing() {
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                bins: Some(vec!["nonexistent_binary_xyz_12345".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(!result.eligible);
        assert!(
            result
                .missing_bins
                .contains(&"nonexistent_binary_xyz_12345".to_string())
        );
    }

    #[test]
    fn test_gating_any_bins_one_found() {
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                any_bins: Some(vec![
                    "nonexistent_abc".to_string(),
                    "ls".to_string(), // this should be found
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(result.eligible);
        assert_eq!(result.any_bin_satisfied, Some(true));
    }

    #[test]
    fn test_gating_any_bins_none_found() {
        let meta = OpenClawMetadata {
            requires: Some(RequirementsSpec {
                any_bins: Some(vec![
                    "nonexistent_abc_123".to_string(),
                    "nonexistent_xyz_456".to_string(),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(!result.eligible);
        assert_eq!(result.any_bin_satisfied, Some(false));
    }

    #[test]
    fn test_gating_os_match() {
        let meta = OpenClawMetadata {
            os: Some(vec!["darwin".to_string()]),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        // On macOS this should pass; on other platforms it would fail
        if cfg!(target_os = "macos") {
            assert!(result.eligible);
        } else {
            assert!(!result.eligible);
        }
    }

    #[test]
    fn test_gating_no_requirements() {
        let meta = OpenClawMetadata::default();
        let result = check_requirements(&meta);
        assert!(result.eligible);
    }

    #[test]
    fn test_gating_primary_env_missing() {
        let meta = OpenClawMetadata {
            primary_env: Some("NONEXISTENT_PRIMARY_KEY_999".to_string()),
            ..Default::default()
        };
        let result = check_requirements(&meta);
        assert!(!result.eligible);
        assert!(
            result
                .missing_env
                .contains(&"NONEXISTENT_PRIMARY_KEY_999".to_string())
        );
    }

    // -- Loading --

    #[test]
    fn test_load_skills_from_dir() {
        let tmp = std::env::temp_dir().join("zeus_test_openclaw_load");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).unwrap();
        std::fs::write(
            tmp.join("my-skill/SKILL.md"),
            "---\nname: my-skill\ndescription: Test skill\n---\n\n# My Skill\nInstructions here.\n",
        )
        .unwrap();

        let skills = load_skills_from_dir(&tmp, SkillSource::Managed);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].0.name, "my-skill");
        assert_eq!(skills[0].1, SkillSource::Managed);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_skills_precedence() {
        let tmp = std::env::temp_dir().join("zeus_test_openclaw_precedence");
        let _ = std::fs::remove_dir_all(&tmp);

        let managed = tmp.join("managed");
        let workspace = tmp.join("workspace");

        // Same skill in both dirs
        std::fs::create_dir_all(managed.join("overlap")).unwrap();
        std::fs::write(
            managed.join("overlap/SKILL.md"),
            "---\nname: overlap\ndescription: managed version\n---\n\nManaged.\n",
        )
        .unwrap();

        std::fs::create_dir_all(workspace.join("overlap")).unwrap();
        std::fs::write(
            workspace.join("overlap/SKILL.md"),
            "---\nname: overlap\ndescription: workspace version\n---\n\nWorkspace.\n",
        )
        .unwrap();

        let skills = load_skills_with_precedence(Some(&workspace), &managed, None, &[]);

        assert_eq!(skills.len(), 1);
        // Workspace should win
        assert_eq!(skills[0].0.description, "workspace version");
        assert_eq!(skills[0].1, SkillSource::Workspace);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_skills_empty_dir() {
        let tmp = std::env::temp_dir().join("zeus_test_openclaw_empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let skills = load_skills_from_dir(&tmp, SkillSource::Bundled);
        assert!(skills.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -- read_when --

    #[test]
    fn test_resolve_read_when_yaml_list() {
        let content =
            "---\nname: github\nread_when:\n  - git commit\n  - pull request\n  - github\n---\n";
        let (fm, _) = parse_frontmatter(content);
        let triggers = resolve_read_when(&fm);
        assert_eq!(triggers, vec!["git commit", "pull request", "github"]);
    }

    #[test]
    fn test_resolve_read_when_comma_separated() {
        let mut fm = HashMap::new();
        fm.insert(
            "read_when".to_string(),
            "git, github, pull request".to_string(),
        );
        let triggers = resolve_read_when(&fm);
        assert_eq!(triggers, vec!["git", "github", "pull request"]);
    }

    #[test]
    fn test_resolve_read_when_absent() {
        let fm = HashMap::new();
        assert!(resolve_read_when(&fm).is_empty());
    }

    #[test]
    fn test_resolve_read_when_camel_alias() {
        let mut fm = HashMap::new();
        fm.insert(
            "readWhen".to_string(),
            r#"["docker","container"]"#.to_string(),
        );
        let triggers = resolve_read_when(&fm);
        assert_eq!(triggers, vec!["docker", "container"]);
    }

    // -- Parse bool --

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("true", false));
        assert!(parse_bool("yes", false));
        assert!(parse_bool("1", false));
        assert!(!parse_bool("false", true));
        assert!(!parse_bool("no", true));
        assert!(!parse_bool("0", true));
        assert!(parse_bool("garbage", true)); // default
        assert!(!parse_bool("garbage", false)); // default
    }

    // -- Current platform --

    #[test]
    fn test_current_platform() {
        let platform = current_platform();
        if cfg!(target_os = "macos") {
            assert_eq!(platform, "darwin");
        } else if cfg!(target_os = "linux") {
            assert_eq!(platform, "linux");
        }
    }

    // -- Install spec parsing --

    #[test]
    fn test_parse_install_specs() {
        let content = r#"---
name: github
description: GitHub ops
metadata: {"openclaw":{"requires":{"bins":["gh"]},"install":[{"id":"brew","kind":"brew","formula":"gh","bins":["gh"],"label":"Install GitHub CLI (brew)"}]}}
---

# GitHub
"#;
        let skill = parse_openclaw_skill(content, PathBuf::from("/skills/github")).unwrap();
        let meta = skill.metadata.unwrap();
        let install = meta.install.unwrap();
        assert_eq!(install.len(), 1);
        assert_eq!(install[0].kind, "brew");
        assert_eq!(install[0].formula, Some("gh".to_string()));
        assert_eq!(install[0].bins, Some(vec!["gh".to_string()]));
    }
}
