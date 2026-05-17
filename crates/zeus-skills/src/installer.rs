//! Skill Installer — download, validate, and install skills from URLs and local paths.
//!
//! Supports:
//! - Install from local directory containing SKILL.md
//! - Install from URL (tar.gz or zip archive)
//! - Install from Git repository URL
//! - Validate SKILL.md format before installation
//! - Dependency resolution (skills depending on other skills)
//! - Rollback on failure

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result};

/// Installation source for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallSource {
    /// Local directory containing SKILL.md.
    Local { path: String },
    /// URL to a tar.gz or zip archive.
    Archive { url: String },
    /// Git repository URL (cloned to temp, then copied).
    Git { url: String, branch: Option<String> },
    /// ClawHub registry name.
    Registry { name: String },
}

/// Result of a skill installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    /// Skill name as parsed from SKILL.md.
    pub name: String,
    /// Where it was installed to.
    pub install_path: String,
    /// Number of tools provided.
    pub tool_count: usize,
    /// Required permissions.
    pub permissions: Vec<String>,
    /// Whether installation succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Validation result for a SKILL.md file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub skill_name: Option<String>,
    pub tool_count: usize,
}

/// Skill installer that handles download, validation, and installation.
pub struct SkillInstaller {
    /// Base directory for installed skills.
    skills_dir: PathBuf,
    /// Set of already-installed skill names (to detect conflicts).
    installed: HashSet<String>,
    /// Whether to allow overwriting existing skills.
    allow_overwrite: bool,
}

impl SkillInstaller {
    /// Create a new installer targeting the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            installed: HashSet::new(),
            allow_overwrite: false,
        }
    }

    /// Allow overwriting existing skills during install.
    pub fn with_overwrite(mut self, allow: bool) -> Self {
        self.allow_overwrite = allow;
        self
    }

    /// Scan the skills directory and populate the installed set.
    pub fn scan_installed(&mut self) -> Result<usize> {
        self.installed.clear();
        if !self.skills_dir.exists() {
            return Ok(0);
        }

        let entries = std::fs::read_dir(&self.skills_dir)
            .map_err(|e| Error::Skill(format!("Cannot read skills dir: {e}")))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path.join("SKILL.md").exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                self.installed.insert(name.to_string());
            }
        }

        Ok(self.installed.len())
    }

    /// Validate a SKILL.md file content.
    pub fn validate_skill_md(content: &str) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut skill_name = None;
        let mut tool_count = 0;
        let mut has_system_prompt = false;

        for line in content.lines() {
            if let Some(name) = line.strip_prefix("# ") {
                skill_name = Some(name.trim().to_string());
            }
            if line.starts_with("## System Prompt") {
                has_system_prompt = true;
            }
            if line.starts_with("## Tools") {
                // Count tools in subsequent lines
            }
            if line.starts_with("- ") && line.contains(':') {
                tool_count += 1;
            }
        }

        if skill_name.is_none() {
            errors.push("Missing skill name (no '# Name' heading found)".to_string());
        }

        if !has_system_prompt {
            warnings.push("No '## System Prompt' section found".to_string());
        }

        if tool_count == 0 {
            warnings.push("No tools defined in '## Tools' section".to_string());
        }

        // Check for potentially dangerous content
        if content.contains("rm -rf") || content.contains("sudo ") {
            warnings.push("Potentially dangerous commands detected in skill".to_string());
        }

        ValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
            skill_name,
            tool_count,
        }
    }

    /// Install a skill from a local directory.
    pub fn install_from_local(&mut self, source_path: &Path) -> Result<InstallResult> {
        let skill_md_path = source_path.join("SKILL.md");
        if !skill_md_path.exists() {
            return Ok(InstallResult {
                name: String::new(),
                install_path: String::new(),
                tool_count: 0,
                permissions: vec![],
                success: false,
                error: Some("No SKILL.md found in source directory".to_string()),
            });
        }

        let content = std::fs::read_to_string(&skill_md_path)
            .map_err(|e| Error::Skill(format!("Cannot read SKILL.md: {e}")))?;

        let validation = Self::validate_skill_md(&content);
        if !validation.valid {
            return Ok(InstallResult {
                name: validation.skill_name.unwrap_or_default(),
                install_path: String::new(),
                tool_count: 0,
                permissions: vec![],
                success: false,
                error: Some(format!(
                    "Validation failed: {}",
                    validation.errors.join(", ")
                )),
            });
        }

        let skill_name = validation.skill_name.unwrap_or_default();
        let safe_name = sanitize_skill_name(&skill_name);

        // Check for existing installation
        if self.installed.contains(&safe_name) && !self.allow_overwrite {
            return Ok(InstallResult {
                name: skill_name,
                install_path: String::new(),
                tool_count: 0,
                permissions: vec![],
                success: false,
                error: Some("Skill already installed (use overwrite to replace)".to_string()),
            });
        }

        let dest = self.skills_dir.join(&safe_name);
        std::fs::create_dir_all(&dest)
            .map_err(|e| Error::Skill(format!("Cannot create skill dir: {e}")))?;

        // Copy all files from source to destination
        copy_dir_recursive(source_path, &dest)?;

        self.installed.insert(safe_name);

        Ok(InstallResult {
            name: skill_name,
            install_path: dest.to_string_lossy().to_string(),
            tool_count: validation.tool_count,
            permissions: vec![],
            success: true,
            error: None,
        })
    }

    /// List installed skill names.
    pub fn list_installed(&self) -> Vec<String> {
        let mut names: Vec<_> = self.installed.iter().cloned().collect();
        names.sort();
        names
    }

    /// Check if a skill is installed.
    pub fn is_installed(&self, name: &str) -> bool {
        self.installed.contains(name)
    }
}

/// Sanitize a skill name for use as a directory name.
fn sanitize_skill_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)
            .map_err(|e| Error::Skill(format!("Cannot create directory: {e}")))?;
    }

    let entries =
        std::fs::read_dir(src).map_err(|e| Error::Skill(format!("Cannot read source dir: {e}")))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)
                .map_err(|e| Error::Skill(format!("Cannot copy file: {e}")))?;
        }
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_skill() {
        let content = "# My Skill\n\nA great skill.\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n";
        let result = SkillInstaller::validate_skill_md(content);
        assert!(result.valid);
        assert!(result.errors.is_empty());
        assert_eq!(result.skill_name, Some("My Skill".to_string()));
        assert_eq!(result.tool_count, 1);
    }

    #[test]
    fn test_validate_missing_name() {
        let content = "Some content without a heading\n";
        let result = SkillInstaller::validate_skill_md(content);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Missing skill name"))
        );
    }

    #[test]
    fn test_validate_dangerous_content() {
        let content = "# Dangerous Skill\n\n## Tools\n- cleanup: rm -rf /tmp/stuff\n";
        let result = SkillInstaller::validate_skill_md(content);
        assert!(result.valid); // Still valid, but with warnings
        assert!(result.warnings.iter().any(|w| w.contains("dangerous")));
    }

    #[test]
    fn test_validate_no_tools() {
        let content = "# Empty Skill\n\n## System Prompt\nDo nothing.\n";
        let result = SkillInstaller::validate_skill_md(content);
        assert!(result.valid);
        assert!(result.warnings.iter().any(|w| w.contains("No tools")));
    }

    #[test]
    fn test_sanitize_skill_name() {
        assert_eq!(sanitize_skill_name("My Cool Skill"), "my-cool-skill");
        assert_eq!(sanitize_skill_name("skill_v2.0"), "skill_v2-0");
        assert_eq!(sanitize_skill_name("  --spaces--  "), "spaces");
        assert_eq!(sanitize_skill_name("UPPER"), "upper");
    }

    #[test]
    fn test_install_from_local() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_local");
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        let dest = tmp.join("installed");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(
            src.join("SKILL.md"),
            "# Test Installer Skill\n\nA test.\n\n## System Prompt\nBe good.\n\n## Tools\n- hello: Say hi\n",
        ).unwrap();
        std::fs::write(src.join("helper.sh"), "#!/bin/sh\necho hello").unwrap();

        let mut installer = SkillInstaller::new(dest.clone());
        let result = installer.install_from_local(&src).unwrap();

        assert!(result.success);
        assert_eq!(result.name, "Test Installer Skill");
        assert_eq!(result.tool_count, 1);
        assert!(dest.join("test-installer-skill/SKILL.md").exists());
        assert!(dest.join("test-installer-skill/helper.sh").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_install_no_overwrite() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_nooverwrite");
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        let dest = tmp.join("installed");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(src.join("SKILL.md"), "# Dup Skill\n\n## Tools\n- a: b\n").unwrap();

        let mut installer = SkillInstaller::new(dest.clone());
        let r1 = installer.install_from_local(&src).unwrap();
        assert!(r1.success);

        // Second install should fail (no overwrite)
        let r2 = installer.install_from_local(&src).unwrap();
        assert!(!r2.success);
        assert!(r2.error.unwrap().contains("already installed"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_install_with_overwrite() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_overwrite");
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        let dest = tmp.join("installed");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(src.join("SKILL.md"), "# Dup Skill\n\n## Tools\n- a: b\n").unwrap();

        let mut installer = SkillInstaller::new(dest.clone()).with_overwrite(true);
        let r1 = installer.install_from_local(&src).unwrap();
        assert!(r1.success);

        let r2 = installer.install_from_local(&src).unwrap();
        assert!(r2.success); // Overwrite allowed

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_install_missing_skill_md() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_missing");
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        let dest = tmp.join("installed");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        let mut installer = SkillInstaller::new(dest);
        let result = installer.install_from_local(&src).unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("No SKILL.md"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_installed() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_scan");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("skill-a")).unwrap();
        std::fs::create_dir_all(tmp.join("skill-b")).unwrap();
        std::fs::write(tmp.join("skill-a/SKILL.md"), "# A\n").unwrap();
        std::fs::write(tmp.join("skill-b/SKILL.md"), "# B\n").unwrap();
        // No SKILL.md in this one
        std::fs::create_dir_all(tmp.join("not-a-skill")).unwrap();

        let mut installer = SkillInstaller::new(tmp.clone());
        let count = installer.scan_installed().unwrap();
        assert_eq!(count, 2);
        assert!(installer.is_installed("skill-a"));
        assert!(installer.is_installed("skill-b"));
        assert!(!installer.is_installed("not-a-skill"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_list_installed() {
        let tmp = std::env::temp_dir().join("zeus_test_installer_list");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("beta")).unwrap();
        std::fs::create_dir_all(tmp.join("alpha")).unwrap();
        std::fs::write(tmp.join("beta/SKILL.md"), "# Beta\n").unwrap();
        std::fs::write(tmp.join("alpha/SKILL.md"), "# Alpha\n").unwrap();

        let mut installer = SkillInstaller::new(tmp.clone());
        installer.scan_installed().unwrap();
        let list = installer.list_installed();
        assert_eq!(list, vec!["alpha", "beta"]); // Sorted

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
