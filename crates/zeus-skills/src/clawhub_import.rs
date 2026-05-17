//! ClawHub Batch Import
//!
//! Fetches SKILL.md files from a configurable ClawHub URL,
//! parses them, and registers as available skills in the SkillManager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};
use zeus_core::Result;

use crate::clawhub::ClawHubClient;
use crate::{Skill, SkillManager, parse_skill_md};

/// Configuration for ClawHub imports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubImportConfig {
    /// ClawHub API base URL
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Directory to store imported skills
    #[serde(default)]
    pub skills_dir: Option<PathBuf>,
    /// Auto-import these skills on startup
    #[serde(default)]
    pub auto_import: Vec<String>,
    /// Skip aegis permission review (only for trusted sources)
    #[serde(default)]
    pub skip_review: bool,
    /// Maximum concurrent fetches
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_base_url() -> String {
    "https://raw.githubusercontent.com/anthropics/skills/main".to_string()
}

fn default_max_concurrent() -> usize {
    4
}

impl Default for ClawHubImportConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            skills_dir: None,
            auto_import: Vec::new(),
            skip_review: false,
            max_concurrent: default_max_concurrent(),
        }
    }
}

impl ClawHubImportConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(url) = std::env::var("CLAWHUB_BASE_URL") {
            config.base_url = url;
        }
        if let Ok(dir) = std::env::var("ZEUS_SKILLS_DIR") {
            config.skills_dir = Some(PathBuf::from(dir));
        }
        if let Ok(skills) = std::env::var("CLAWHUB_AUTO_IMPORT") {
            config.auto_import = skills.split(',').map(|s| s.trim().to_string()).collect();
        }
        config
    }
}

/// Result of a single skill import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub name: String,
    pub version: String,
    pub success: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

/// Result of a batch import operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchImportResult {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<ImportResult>,
}

/// ClawHub skill importer
///
/// Fetches SKILL.md files from ClawHub, parses and validates them,
/// then registers them in a SkillManager.
pub struct ClawHubImporter {
    config: ClawHubImportConfig,
    client: ClawHubClient,
}

impl ClawHubImporter {
    /// Create a new importer with the given config
    pub fn new(config: ClawHubImportConfig) -> Self {
        let skills_dir = config
            .skills_dir
            .clone()
            .unwrap_or_else(|| zeus_core::default_config_dir().join("skills"));

        let client = ClawHubClient::with_url(skills_dir, &config.base_url);
        Self { config, client }
    }

    /// Create with default config
    pub fn with_defaults() -> Self {
        Self::new(ClawHubImportConfig::default())
    }

    /// Import a single skill by name from ClawHub.
    ///
    /// Fetches the SKILL.md, validates it, reviews permissions,
    /// writes to disk, and returns the parsed Skill.
    pub async fn import_skill(&mut self, name: &str) -> Result<ImportResult> {
        info!("Importing skill '{}' from ClawHub", name);

        // Check if already installed
        if self.client.get_installed(name).is_some() {
            return Ok(ImportResult {
                name: name.to_string(),
                version: self
                    .client
                    .get_installed(name)
                    .map(|m| m.version.clone())
                    .unwrap_or_default(),
                success: true,
                error: None,
                warnings: vec!["Already installed, skipping".to_string()],
            });
        }

        match self.client.install(name).await {
            Ok(result) => {
                info!("Successfully imported skill '{}' v{}", name, result.version);
                Ok(ImportResult {
                    name: result.name,
                    version: result.version,
                    success: true,
                    error: None,
                    warnings: result.warnings,
                })
            }
            Err(e) => {
                warn!("Failed to import skill '{}': {}", name, e);
                Ok(ImportResult {
                    name: name.to_string(),
                    version: String::new(),
                    success: false,
                    error: Some(e.to_string()),
                    warnings: Vec::new(),
                })
            }
        }
    }

    /// Import a skill from local SKILL.md content (no network fetch).
    pub fn import_local(&mut self, name: &str, content: &str) -> Result<ImportResult> {
        info!("Importing local skill '{}'", name);

        match self.client.install_local(name, content) {
            Ok(result) => {
                info!(
                    "Successfully imported local skill '{}' v{}",
                    name, result.version
                );
                Ok(ImportResult {
                    name: result.name,
                    version: result.version,
                    success: true,
                    error: None,
                    warnings: result.warnings,
                })
            }
            Err(e) => {
                warn!("Failed to import local skill '{}': {}", name, e);
                Ok(ImportResult {
                    name: name.to_string(),
                    version: String::new(),
                    success: false,
                    error: Some(e.to_string()),
                    warnings: Vec::new(),
                })
            }
        }
    }

    /// Import multiple skills by name.
    ///
    /// Fetches skills concurrently (up to max_concurrent) and returns
    /// a BatchImportResult with per-skill results.
    pub async fn import_batch(&mut self, names: &[String]) -> BatchImportResult {
        let total = names.len();
        let mut results = Vec::with_capacity(total);
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        // Process in chunks to limit concurrency
        for chunk in names.chunks(self.config.max_concurrent) {
            for name in chunk {
                match self.import_skill(name).await {
                    Ok(result) => {
                        if result.success {
                            succeeded += 1;
                        } else {
                            failed += 1;
                        }
                        results.push(result);
                    }
                    Err(e) => {
                        failed += 1;
                        results.push(ImportResult {
                            name: name.clone(),
                            version: String::new(),
                            success: false,
                            error: Some(e.to_string()),
                            warnings: Vec::new(),
                        });
                    }
                }
            }
        }

        info!("Batch import complete: {}/{} succeeded", succeeded, total);

        BatchImportResult {
            total,
            succeeded,
            failed,
            results,
        }
    }

    /// Run auto-import for skills listed in config.auto_import.
    ///
    /// Intended to be called on startup to ensure required skills are available.
    pub async fn run_auto_import(&mut self) -> BatchImportResult {
        let names = self.config.auto_import.clone();
        if names.is_empty() {
            return BatchImportResult {
                total: 0,
                succeeded: 0,
                failed: 0,
                results: Vec::new(),
            };
        }
        info!("Auto-importing {} skills from ClawHub", names.len());
        self.import_batch(&names).await
    }

    /// Fetch a SKILL.md from ClawHub and parse it without installing.
    ///
    /// Useful for previewing a skill before importing.
    pub async fn preview_skill(&self, name: &str) -> Result<Skill> {
        let content = self.client.fetch_skill_md(name).await?;
        let skill = parse_skill_md(&content, PathBuf::from(name))?;
        Ok(skill)
    }

    /// Validate a SKILL.md string and return the validation result
    pub fn validate(content: &str) -> Result<crate::clawhub::ValidationResult> {
        ClawHubClient::validate_skill_md(content)
    }

    /// Register all imported skills into a SkillManager.
    ///
    /// Loads every skill from the skills directory into the manager.
    pub async fn register_all(&self, manager: &mut SkillManager) -> Result<usize> {
        manager.load_all().await
    }

    /// Get the underlying ClawHub client for direct access
    pub fn client(&self) -> &ClawHubClient {
        &self.client
    }

    /// Get the import config
    pub fn config(&self) -> &ClawHubImportConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_config_default() {
        let config = ClawHubImportConfig::default();
        assert_eq!(
            config.base_url,
            "https://raw.githubusercontent.com/anthropics/skills/main"
        );
        assert!(config.skills_dir.is_none());
        assert!(config.auto_import.is_empty());
        assert!(!config.skip_review);
        assert_eq!(config.max_concurrent, 4);
    }

    #[test]
    fn test_import_config_serialization() {
        let config = ClawHubImportConfig {
            base_url: "https://custom.example.com/api".to_string(),
            skills_dir: Some(PathBuf::from("/tmp/skills")),
            auto_import: vec!["git".to_string(), "code-review".to_string()],
            skip_review: false,
            max_concurrent: 2,
        };
        let json = serde_json::to_string(&config).expect("should serialize");
        let parsed: ClawHubImportConfig = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(parsed.base_url, "https://custom.example.com/api");
        assert_eq!(parsed.auto_import.len(), 2);
        assert_eq!(parsed.max_concurrent, 2);
    }

    #[test]
    fn test_importer_new() {
        let config = ClawHubImportConfig {
            skills_dir: Some(PathBuf::from("/tmp/zeus-test-import")),
            ..Default::default()
        };
        let importer = ClawHubImporter::new(config);
        assert_eq!(
            importer.config().base_url,
            "https://raw.githubusercontent.com/anthropics/skills/main"
        );
    }

    #[test]
    fn test_importer_with_defaults() {
        let importer = ClawHubImporter::with_defaults();
        assert_eq!(importer.config().max_concurrent, 4);
    }

    #[test]
    fn test_import_result_serialization() {
        let result = ImportResult {
            name: "test-skill".to_string(),
            version: "1.0.0".to_string(),
            success: true,
            error: None,
            warnings: vec!["Some warning".to_string()],
        };
        let json = serde_json::to_string(&result).expect("should serialize");
        assert!(json.contains("test-skill"));
        assert!(json.contains("1.0.0"));
    }

    #[test]
    fn test_batch_import_result_serialization() {
        let result = BatchImportResult {
            total: 3,
            succeeded: 2,
            failed: 1,
            results: vec![
                ImportResult {
                    name: "a".to_string(),
                    version: "1.0.0".to_string(),
                    success: true,
                    error: None,
                    warnings: Vec::new(),
                },
                ImportResult {
                    name: "b".to_string(),
                    version: "2.0.0".to_string(),
                    success: true,
                    error: None,
                    warnings: Vec::new(),
                },
                ImportResult {
                    name: "c".to_string(),
                    version: String::new(),
                    success: false,
                    error: Some("not found".to_string()),
                    warnings: Vec::new(),
                },
            ],
        };
        let json = serde_json::to_string(&result).expect("should serialize");
        let parsed: BatchImportResult = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(parsed.total, 3);
        assert_eq!(parsed.succeeded, 2);
        assert_eq!(parsed.failed, 1);
    }

    #[test]
    fn test_import_local_success() {
        let tmp = std::env::temp_dir().join("zeus_test_clawhub_import_local");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = ClawHubImportConfig {
            skills_dir: Some(tmp.clone()),
            ..Default::default()
        };
        let mut importer = ClawHubImporter::new(config);

        let content = "# Local Skill\n\n## Version: 1.0.0\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n";
        let result = importer
            .import_local("local-skill", content)
            .expect("should import");
        assert!(result.success);
        assert_eq!(result.name, "local-skill");
        assert_eq!(result.version, "1.0.0");

        // Verify on disk
        assert!(tmp.join("local-skill/SKILL.md").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_import_local_bad_permissions() {
        let tmp = std::env::temp_dir().join("zeus_test_clawhub_import_denied");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = ClawHubImportConfig {
            skills_dir: Some(tmp.clone()),
            ..Default::default()
        };
        let mut importer = ClawHubImporter::new(config);

        let content = "# Evil Skill\n\n## Version: 1.0.0\n\n## Permissions\n- sudo\n- root\n";
        let result = importer
            .import_local("evil", content)
            .expect("should return result");
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("Permission review failed")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_validate_valid() {
        let content = "# Good Skill\n\n## Version: 2.0.0\n\n## System Prompt\nHelp.\n\n## Tools\n- do: Do thing\n";
        let result = ClawHubImporter::validate(content).expect("should validate");
        assert!(result.valid);
        assert_eq!(result.name, "Good Skill");
        assert_eq!(result.version, "2.0.0");
    }

    #[test]
    fn test_validate_missing_name() {
        let content = "## System Prompt\nHelp.\n";
        let result = ClawHubImporter::validate(content).expect("should validate");
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Missing skill name"))
        );
    }

    #[tokio::test]
    async fn test_run_auto_import_empty() {
        let config = ClawHubImportConfig {
            skills_dir: Some(std::env::temp_dir().join("zeus_test_auto_empty")),
            auto_import: Vec::new(),
            ..Default::default()
        };
        let mut importer = ClawHubImporter::new(config);
        let result = importer.run_auto_import().await;
        assert_eq!(result.total, 0);
        assert_eq!(result.succeeded, 0);
    }

    #[tokio::test]
    async fn test_import_batch_mixed() {
        let tmp = std::env::temp_dir().join("zeus_test_batch_import");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = ClawHubImportConfig {
            skills_dir: Some(tmp.clone()),
            ..Default::default()
        };
        let mut importer = ClawHubImporter::new(config);

        // Pre-install one skill locally
        let content = "# Pre Skill\n\n## Version: 1.0.0\n\n## System Prompt\nHelp.\n";
        importer
            .import_local("pre-skill", content)
            .expect("should import");

        // Batch import: one already installed (skip), one remote (will fail since no server)
        let names = vec!["pre-skill".to_string(), "nonexistent-remote".to_string()];
        let result = importer.import_batch(&names).await;

        assert_eq!(result.total, 2);
        // pre-skill should succeed (already installed)
        assert!(result.results[0].success);
        assert!(
            result.results[0]
                .warnings
                .iter()
                .any(|w| w.contains("Already installed"))
        );
        // nonexistent-remote should fail (no server)
        assert!(!result.results[1].success);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_register_all() {
        let tmp = std::env::temp_dir().join("zeus_test_register_all");
        let _ = std::fs::remove_dir_all(&tmp);

        // Create a skill on disk
        std::fs::create_dir_all(tmp.join("my-skill")).expect("create dir");
        std::fs::write(
            tmp.join("my-skill/SKILL.md"),
            "# My Skill\n\nTest skill.\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n",
        )
        .expect("write file");

        let config = ClawHubImportConfig {
            skills_dir: Some(tmp.clone()),
            ..Default::default()
        };
        let importer = ClawHubImporter::new(config);
        let mut manager = SkillManager::new(tmp.clone());

        let count = importer
            .register_all(&mut manager)
            .await
            .expect("should register");
        assert_eq!(count, 1);
        assert!(manager.get("My Skill").is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
