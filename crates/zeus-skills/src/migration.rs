//! OpenClaw Migration Engine
//!
//! Scans an OpenClaw installation directory, discovers all SKILL.md files,
//! parses them using Zeus's OpenClaw-compatible parser, and imports them
//! into the Zeus skills directory with a detailed migration report.
//!
//! Usage:
//!   `zeus migrate openclaw /path/to/openclaw`
//!
//! Supports:
//! - `skills/*/SKILL.md` — community skills
//! - `extensions/*/skills/*/SKILL.md` — extension-bundled skills
//! - Config key mapping (OpenClaw → Zeus config.toml)
//! - Compatibility scoring per skill

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;
use zeus_core::Result;

use crate::{Skill, parse_skill_md};

// ============================================================================
// Migration report types
// ============================================================================

/// Result of migrating a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMigrationResult {
    /// Original skill name.
    pub name: String,
    /// Source path (relative to OpenClaw root).
    pub source_path: String,
    /// Whether the skill was successfully migrated.
    pub success: bool,
    /// Compatibility score (0.0 - 1.0).
    pub compatibility: f64,
    /// Issues encountered (non-fatal warnings).
    pub warnings: Vec<String>,
    /// Fatal error if migration failed.
    pub error: Option<String>,
    /// Zeus skill path after migration (if successful).
    pub zeus_path: Option<String>,
}

/// Overall migration report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Source OpenClaw directory.
    pub source_dir: String,
    /// Target Zeus skills directory.
    pub target_dir: String,
    /// Total SKILL.md files discovered.
    pub discovered: usize,
    /// Successfully migrated count.
    pub migrated: usize,
    /// Failed count.
    pub failed: usize,
    /// Skipped count (already exists).
    pub skipped: usize,
    /// Per-skill results.
    pub skills: Vec<SkillMigrationResult>,
    /// Config key mappings suggested.
    pub config_mappings: Vec<ConfigMapping>,
    /// Overall compatibility score (average of successful migrations).
    pub overall_compatibility: f64,
}

/// A config key mapping from OpenClaw to Zeus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapping {
    /// OpenClaw config key (e.g., "channels.discord.token").
    pub openclaw_key: String,
    /// Zeus config.toml equivalent.
    pub zeus_key: String,
    /// Whether an exact mapping exists.
    pub exact_match: bool,
    /// Notes about the mapping.
    pub notes: Option<String>,
}

// ============================================================================
// Discovery
// ============================================================================

/// Discovered SKILL.md file with its relative path.
#[derive(Debug, Clone)]
struct DiscoveredSkill {
    /// Absolute path to SKILL.md.
    abs_path: PathBuf,
    /// Path relative to the OpenClaw root.
    rel_path: String,
    /// Category (community, extension, or top-level).
    _category: SkillCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SkillCategory {
    Community,
    Extension,
    TopLevel,
}

/// Scan an OpenClaw directory for all SKILL.md files.
fn discover_skills(openclaw_root: &Path) -> Vec<DiscoveredSkill> {
    let mut found = Vec::new();

    // 1. skills/*/SKILL.md (community skills)
    let skills_dir = openclaw_root.join("skills");
    if skills_dir.is_dir() {
        scan_skill_dirs(&skills_dir, openclaw_root, SkillCategory::Community, &mut found);
    }

    // 2. extensions/*/skills/*/SKILL.md (extension-bundled)
    let ext_dir = openclaw_root.join("extensions");
    if ext_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&ext_dir)
    {
        for entry in entries.flatten() {
            let ext_skills = entry.path().join("skills");
            if ext_skills.is_dir() {
                scan_skill_dirs(&ext_skills, openclaw_root, SkillCategory::Extension, &mut found);
            }
            // Also check extension root for SKILL.md
            let ext_skill = entry.path().join("SKILL.md");
            if ext_skill.is_file() {
                let rel = ext_skill
                    .strip_prefix(openclaw_root)
                    .unwrap_or(&ext_skill)
                    .to_string_lossy()
                    .to_string();
                found.push(DiscoveredSkill {
                    abs_path: ext_skill,
                    rel_path: rel,
                    _category: SkillCategory::Extension,
                });
            }
        }
    }

    // 3. Top-level SKILL.md (rare but possible)
    let top_skill = openclaw_root.join("SKILL.md");
    if top_skill.is_file() {
        found.push(DiscoveredSkill {
            abs_path: top_skill,
            rel_path: "SKILL.md".to_string(),
            _category: SkillCategory::TopLevel,
        });
    }

    found
}

fn scan_skill_dirs(
    dir: &Path,
    root: &Path,
    category: SkillCategory,
    out: &mut Vec<DiscoveredSkill>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let skill_file = entry.path().join("SKILL.md");
            if skill_file.is_file() {
                let rel = skill_file
                    .strip_prefix(root)
                    .unwrap_or(&skill_file)
                    .to_string_lossy()
                    .to_string();
                out.push(DiscoveredSkill {
                    abs_path: skill_file,
                    rel_path: rel,
                    _category: category.clone(),
                });
            }
        }
    }
}

// ============================================================================
// Config mapping
// ============================================================================

/// Generate config key mappings from OpenClaw → Zeus.
fn build_config_mappings(skills: &[Skill]) -> Vec<ConfigMapping> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut mappings = Vec::new();

    // Collect all required config keys from skill metadata
    for skill in skills {
        if let Some(ref oc) = skill.metadata
            && let Some(ref reqs) = oc.requires
        {
            if let Some(ref configs) = reqs.config {
                for cfg_key in configs {
                    if seen.insert(cfg_key.to_string()) {
                        mappings.push(map_config_key(cfg_key));
                    }
                }
            }
            if let Some(ref envs) = reqs.env {
                for env_key in envs {
                    if seen.insert(env_key.to_string()) {
                        mappings.push(map_env_key(env_key));
                    }
                }
            }
        }
    }

    // Add standard mappings
    let standard: [(&str, &str, bool, Option<&str>); 7] = [
        ("model", "model", true, None),
        ("workspace", "workspace", true, None),
        ("channels.telegram.apiId", "channels.telegram.api_id", true, None),
        ("channels.telegram.apiHash", "channels.telegram.api_hash", true, None),
        ("channels.discord.token", "channels.discord.token", true, None),
        ("channels.slack.botToken", "channels.slack.bot_token", true, None),
        ("channels.slack.appToken", "channels.slack.app_token", true, None),
    ];

    for (oc, zeus, exact, notes) in standard {
        if seen.insert(oc.to_string()) {
            mappings.push(ConfigMapping {
                openclaw_key: oc.to_string(),
                zeus_key: zeus.to_string(),
                exact_match: exact,
                notes: notes.map(String::from),
            });
        }
    }

    mappings
}

fn map_config_key(key: &str) -> ConfigMapping {
    // Map OpenClaw config keys to Zeus equivalents
    let (zeus_key, exact) = match key {
        k if k.starts_with("channels.discord") => (k.to_string(), true),
        k if k.starts_with("channels.telegram") => {
            (k.replace("apiId", "api_id").replace("apiHash", "api_hash"), true)
        }
        k if k.starts_with("channels.slack") => {
            (k.replace("botToken", "bot_token").replace("appToken", "app_token"), true)
        }
        k if k.starts_with("channels.") => (k.to_string(), true),
        _ => (key.to_string(), false),
    };

    ConfigMapping {
        openclaw_key: key.to_string(),
        zeus_key,
        exact_match: exact,
        notes: if exact {
            None
        } else {
            Some("Manual mapping may be needed".to_string())
        },
    }
}

fn map_env_key(key: &str) -> ConfigMapping {
    ConfigMapping {
        openclaw_key: format!("env:{}", key),
        zeus_key: format!("~/.zeus/.env: {}", key),
        exact_match: true,
        notes: Some("Add to ~/.zeus/.env".to_string()),
    }
}

// ============================================================================
// Compatibility scoring
// ============================================================================

fn score_compatibility(skill: &Skill) -> (f64, Vec<String>) {
    let mut score: f64 = 1.0;
    let mut warnings = Vec::new();

    // Check for unsupported features
    if let Some(ref oc) = skill.metadata
        && let Some(ref reqs) = oc.requires
        && let Some(ref bins) = reqs.bins
    {
        for bin in bins {
            if bin == "node" || bin == "npx" || bin == "bun" || bin == "bunx" {
                score -= 0.3;
                warnings.push(format!(
                    "Requires '{}' binary — Node.js runtime skills need process bridge",
                    bin
                ));
            }
        }
    }

    // Shell/script implementations are fully compatible
    // Native implementations need porting
    for tool in &skill.tools {
        if matches!(&tool.implementation, crate::ToolImplementation::Native) {
            score -= 0.2;
            warnings.push(format!(
                "Tool '{}' has native implementation — needs porting",
                tool.name
            ));
        }
    }

    // System prompt presence is a good sign
    if skill.system_prompt.is_empty() {
        score -= 0.1;
        warnings.push("No system prompt defined".to_string());
    }

    (score.max(0.0), warnings)
}

// ============================================================================
// Migration engine
// ============================================================================

/// OpenClaw migration engine.
pub struct MigrationEngine {
    /// Target Zeus skills directory.
    target_dir: PathBuf,
    /// Whether to overwrite existing skills.
    overwrite: bool,
    /// Dry run mode (don't write files).
    dry_run: bool,
}

impl MigrationEngine {
    /// Create a new migration engine targeting the Zeus skills directory.
    pub fn new(target_dir: PathBuf) -> Self {
        Self {
            target_dir,
            overwrite: false,
            dry_run: false,
        }
    }

    /// Set overwrite mode.
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Set dry run mode.
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Run the full migration from an OpenClaw installation directory.
    pub fn migrate(&self, openclaw_root: &Path) -> Result<MigrationReport> {
        info!(
            "Scanning OpenClaw installation at {}",
            openclaw_root.display()
        );

        let discovered = discover_skills(openclaw_root);
        info!("Discovered {} SKILL.md files", discovered.len());

        let mut results = Vec::new();
        let mut migrated_skills = Vec::new();
        let mut migrated = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;

        for disc in &discovered {
            let result = self.migrate_skill(disc);
            match &result {
                r if r.success => {
                    migrated += 1;
                    // Parse again for config mapping collection
                    if let Ok(content) = std::fs::read_to_string(&disc.abs_path)
                        && let Ok(skill) = parse_skill_md(
                            &content,
                            disc.abs_path.parent().unwrap_or(&disc.abs_path).to_path_buf(),
                        )
                    {
                        migrated_skills.push(skill);
                    }
                }
                r if r.error.as_deref() == Some("Already exists") => skipped += 1,
                _ => failed += 1,
            }
            results.push(result);
        }

        let config_mappings = build_config_mappings(&migrated_skills);

        let overall_compat = if migrated > 0 {
            let sum: f64 = results
                .iter()
                .filter(|r| r.success)
                .map(|r| r.compatibility)
                .sum();
            sum / migrated as f64
        } else {
            0.0
        };

        let report = MigrationReport {
            source_dir: openclaw_root.to_string_lossy().to_string(),
            target_dir: self.target_dir.to_string_lossy().to_string(),
            discovered: discovered.len(),
            migrated,
            failed,
            skipped,
            skills: results,
            config_mappings,
            overall_compatibility: overall_compat,
        };

        info!(
            "Migration complete: {}/{} migrated, {} failed, {} skipped. Compatibility: {:.0}%",
            migrated,
            discovered.len(),
            failed,
            skipped,
            overall_compat * 100.0
        );

        Ok(report)
    }

    /// Migrate a single discovered skill.
    fn migrate_skill(&self, disc: &DiscoveredSkill) -> SkillMigrationResult {
        // Read the SKILL.md content
        let content = match std::fs::read_to_string(&disc.abs_path) {
            Ok(c) => c,
            Err(e) => {
                return SkillMigrationResult {
                    name: disc.rel_path.clone(),
                    source_path: disc.rel_path.clone(),
                    success: false,
                    compatibility: 0.0,
                    warnings: Vec::new(),
                    error: Some(format!("Failed to read: {}", e)),
                    zeus_path: None,
                };
            }
        };

        // Parse using Zeus's OpenClaw-compatible parser
        let skill = match parse_skill_md(
            &content,
            disc.abs_path.parent().unwrap_or(&disc.abs_path).to_path_buf(),
        ) {
            Ok(s) => s,
            Err(e) => {
                return SkillMigrationResult {
                    name: disc.rel_path.clone(),
                    source_path: disc.rel_path.clone(),
                    success: false,
                    compatibility: 0.0,
                    warnings: Vec::new(),
                    error: Some(format!("Parse error: {}", e)),
                    zeus_path: None,
                };
            }
        };

        let (compat, mut warnings) = score_compatibility(&skill);

        // Determine target path
        let slug = crate::slugify(&skill.name);
        let target = self.target_dir.join(&slug);
        let target_skill = target.join("SKILL.md");

        // Check if already exists
        if target_skill.exists() && !self.overwrite {
            return SkillMigrationResult {
                name: skill.name,
                source_path: disc.rel_path.clone(),
                success: false,
                compatibility: compat,
                warnings,
                error: Some("Already exists".to_string()),
                zeus_path: Some(target_skill.to_string_lossy().to_string()),
            };
        }

        // Write to Zeus skills directory (unless dry run)
        if !self.dry_run {
            if let Err(e) = std::fs::create_dir_all(&target) {
                return SkillMigrationResult {
                    name: skill.name,
                    source_path: disc.rel_path.clone(),
                    success: false,
                    compatibility: compat,
                    warnings,
                    error: Some(format!("Failed to create dir: {}", e)),
                    zeus_path: None,
                };
            }
            if let Err(e) = std::fs::write(&target_skill, &content) {
                return SkillMigrationResult {
                    name: skill.name,
                    source_path: disc.rel_path.clone(),
                    success: false,
                    compatibility: compat,
                    warnings,
                    error: Some(format!("Failed to write: {}", e)),
                    zeus_path: None,
                };
            }
        } else {
            warnings.push("Dry run — no files written".to_string());
        }

        SkillMigrationResult {
            name: skill.name,
            source_path: disc.rel_path.clone(),
            success: true,
            compatibility: compat,
            warnings,
            error: None,
            zeus_path: Some(target_skill.to_string_lossy().to_string()),
        }
    }

    /// Generate a human-readable summary of the migration report.
    pub fn format_report(report: &MigrationReport) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "OpenClaw Migration Report\n{}\n",
            "=".repeat(40)
        ));
        out.push_str(&format!("Source: {}\n", report.source_dir));
        out.push_str(&format!("Target: {}\n", report.target_dir));
        out.push_str(&format!(
            "Discovered: {} | Migrated: {} | Failed: {} | Skipped: {}\n",
            report.discovered, report.migrated, report.failed, report.skipped
        ));
        out.push_str(&format!(
            "Overall compatibility: {:.0}%\n\n",
            report.overall_compatibility * 100.0
        ));

        if !report.skills.is_empty() {
            out.push_str("Skills:\n");
            for s in &report.skills {
                let status = if s.success {
                    "OK"
                } else if s.error.as_deref() == Some("Already exists") {
                    "SKIP"
                } else {
                    "FAIL"
                };
                out.push_str(&format!(
                    "  [{:4}] {} ({:.0}%) — {}\n",
                    status,
                    s.name,
                    s.compatibility * 100.0,
                    s.source_path
                ));
                for w in &s.warnings {
                    out.push_str(&format!("         ! {}\n", w));
                }
                if let Some(ref e) = s.error
                    && e != "Already exists"
                {
                    out.push_str(&format!("         ERROR: {}\n", e));
                }
            }
        }

        if !report.config_mappings.is_empty() {
            out.push_str("\nConfig Mappings (OpenClaw -> Zeus):\n");
            for m in &report.config_mappings {
                let mark = if m.exact_match { "=" } else { "~" };
                out.push_str(&format!(
                    "  {} {} {} {}\n",
                    m.openclaw_key,
                    mark,
                    m.zeus_key,
                    m.notes.as_deref().unwrap_or("")
                ));
            }
        }

        out
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_openclaw_dir(tmp: &Path) {
        // Create skill directories mimicking OpenClaw structure
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(skills_dir.join("my-skill")).unwrap();
        std::fs::write(
            skills_dir.join("my-skill/SKILL.md"),
            "---\nname: my-skill\ndescription: \"A test skill\"\n---\n\n# My Skill\n\nTest.\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n",
        ).unwrap();

        std::fs::create_dir_all(skills_dir.join("discord")).unwrap();
        std::fs::write(
            skills_dir.join("discord/SKILL.md"),
            "---\nname: discord\ndescription: \"Discord ops\"\nmetadata: { \"openclaw\": { \"emoji\": \"\\U0001F3AE\", \"requires\": { \"config\": [\"channels.discord.token\"] } } }\nallowed-tools: [\"message\"]\n---\n\n# Discord\n\nDiscord skill.\n\n## System Prompt\nUse message tool.\n",
        ).unwrap();

        // Extension with nested skill
        let ext_dir = tmp.join("extensions/my-ext/skills/ext-skill");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("SKILL.md"),
            "# Extension Skill\n\nFrom extension.\n\n## System Prompt\nHelp.\n\n## Tools\n- ext_do: Do stuff\n",
        ).unwrap();
    }

    #[test]
    fn test_discover_skills() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_openclaw_dir(tmp.path());

        let found = discover_skills(tmp.path());
        assert!(found.len() >= 3, "Should find at least 3 skills, found {}", found.len());
    }

    #[test]
    fn test_migrate_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        create_test_openclaw_dir(tmp.path());

        let engine = MigrationEngine::new(target.path().to_path_buf()).with_dry_run(true);
        let report = engine.migrate(tmp.path()).unwrap();

        assert_eq!(report.discovered, 3);
        assert_eq!(report.migrated, 3);
        assert_eq!(report.failed, 0);
        // Dry run — no files should be written
        assert!(!target.path().join("my-skill/SKILL.md").exists());
    }

    #[test]
    fn test_migrate_writes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        create_test_openclaw_dir(tmp.path());

        let engine = MigrationEngine::new(target.path().to_path_buf());
        let report = engine.migrate(tmp.path()).unwrap();

        assert_eq!(report.migrated, 3);
        // Files should be written
        assert!(target.path().join("my-skill/SKILL.md").exists());
        assert!(target.path().join("discord/SKILL.md").exists());
    }

    #[test]
    fn test_migrate_skip_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        create_test_openclaw_dir(tmp.path());

        // Pre-create one skill
        std::fs::create_dir_all(target.path().join("my-skill")).unwrap();
        std::fs::write(target.path().join("my-skill/SKILL.md"), "existing").unwrap();

        let engine = MigrationEngine::new(target.path().to_path_buf());
        let report = engine.migrate(tmp.path()).unwrap();

        assert_eq!(report.skipped, 1);
        assert_eq!(report.migrated, 2);
        // Existing file should not be overwritten
        let content = std::fs::read_to_string(target.path().join("my-skill/SKILL.md")).unwrap();
        assert_eq!(content, "existing");
    }

    #[test]
    fn test_migrate_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        create_test_openclaw_dir(tmp.path());

        // Pre-create one skill
        std::fs::create_dir_all(target.path().join("my-skill")).unwrap();
        std::fs::write(target.path().join("my-skill/SKILL.md"), "old").unwrap();

        let engine = MigrationEngine::new(target.path().to_path_buf()).with_overwrite(true);
        let report = engine.migrate(tmp.path()).unwrap();

        assert_eq!(report.migrated, 3);
        assert_eq!(report.skipped, 0);
        // File should be overwritten
        let content = std::fs::read_to_string(target.path().join("my-skill/SKILL.md")).unwrap();
        assert_ne!(content, "old");
    }

    #[test]
    fn test_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        let engine = MigrationEngine::new(target.path().to_path_buf());
        let report = engine.migrate(tmp.path()).unwrap();

        assert_eq!(report.discovered, 0);
        assert_eq!(report.migrated, 0);
    }

    #[test]
    fn test_compatibility_scoring() {
        let skill = Skill {
            name: "test".into(),
            description: "test".into(),
            system_prompt: "Be helpful.".into(),
            tools: vec![],
            path: PathBuf::new(),
            version: "1.0.0".into(),
            metadata: None,
            frontmatter: HashMap::new(),
            author: None,
            permissions: vec![],
            raw_content: String::new(),
            invocation: Default::default(),
            command_dispatch: None,
            read_when: vec![],
        };
        let (score, warnings) = score_compatibility(&skill);
        assert!(score >= 0.9, "Simple skill should have high compat: {}", score);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_config_mapping() {
        let mappings = build_config_mappings(&[]);
        // Should include standard mappings
        assert!(mappings.iter().any(|m| m.openclaw_key == "channels.discord.token"));
    }

    #[test]
    fn test_format_report() {
        let report = MigrationReport {
            source_dir: "/tmp/openclaw".into(),
            target_dir: "/tmp/zeus/skills".into(),
            discovered: 3,
            migrated: 2,
            failed: 1,
            skipped: 0,
            skills: vec![
                SkillMigrationResult {
                    name: "good-skill".into(),
                    source_path: "skills/good-skill/SKILL.md".into(),
                    success: true,
                    compatibility: 1.0,
                    warnings: vec![],
                    error: None,
                    zeus_path: Some("/tmp/zeus/skills/good-skill/SKILL.md".into()),
                },
                SkillMigrationResult {
                    name: "bad-skill".into(),
                    source_path: "skills/bad-skill/SKILL.md".into(),
                    success: false,
                    compatibility: 0.0,
                    warnings: vec![],
                    error: Some("Parse error: missing name".into()),
                    zeus_path: None,
                },
            ],
            config_mappings: vec![],
            overall_compatibility: 0.5,
        };

        let text = MigrationEngine::format_report(&report);
        assert!(text.contains("Migrated: 2"));
        assert!(text.contains("Failed: 1"));
        assert!(text.contains("good-skill"));
        assert!(text.contains("bad-skill"));
    }

    #[test]
    fn test_discover_extension_skills() {
        let tmp = tempfile::tempdir().unwrap();

        // Only create extension skills
        let ext = tmp.path().join("extensions/telegram/skills/tg-ops");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("SKILL.md"),
            "# Telegram Ops\n\nTelegram.\n\n## System Prompt\nHelp.\n",
        ).unwrap();

        let found = discover_skills(tmp.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].rel_path.contains("telegram"));
    }

    #[test]
    fn test_migrate_unreadable_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        // Create a directory named SKILL.md (not a file)
        let bad = tmp.path().join("skills/bad/SKILL.md");
        std::fs::create_dir_all(&bad).unwrap();

        let engine = MigrationEngine::new(target.path().to_path_buf());
        let report = engine.migrate(tmp.path()).unwrap();

        // Should discover 0 (directories aren't files)
        assert_eq!(report.discovered, 0);
    }
}
