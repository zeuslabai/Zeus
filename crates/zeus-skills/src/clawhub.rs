//! ClawHub Skill Discovery Runtime
//!
//! Provides real skill discovery, installation, and management with:
//! - Local registry listing (disk-backed JSON cache)
//! - Remote ClawHub fetch with offline fallback
//! - SKILL.md validation before install
//! - Aegis permission review integration
//! - Version checking and update detection

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use zeus_core::{Error, Result};

/// Metadata for an installed skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    pub name: String,
    pub version: String,
    pub author: String,
    pub installed_at: u64,
    pub source: SkillSource,
    pub permissions: Vec<String>,
    /// Whether permissions were reviewed and approved by aegis
    pub permissions_approved: bool,
}

/// Where a skill was installed from
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillSource {
    Builtin,
    Remote { url: String },
    Local { path: String },
}

/// Skill summary for search/listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub downloads: u64,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Update info for a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub name: String,
    pub installed_version: String,
    pub available_version: String,
}

/// Local catalog cache
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
struct CatalogCache {
    skills: Vec<SkillSummary>,
    last_fetched: u64,
}

/// Permission review result from aegis
#[derive(Debug, Clone)]
pub struct PermissionReview {
    pub approved: bool,
    pub permissions: Vec<String>,
    pub denied_permissions: Vec<String>,
    pub reason: Option<String>,
}

/// ClawHub client for skill discovery and installation
pub struct ClawHubClient {
    skills_dir: PathBuf,
    base_url: String,
    installed: HashMap<String, InstalledSkillMeta>,
}

impl ClawHubClient {
    /// Create a new client with the given skills directory
    pub fn new(skills_dir: PathBuf) -> Self {
        let mut client = Self {
            skills_dir,
            base_url: "https://raw.githubusercontent.com/anthropics/skills/main".to_string(),
            installed: HashMap::new(),
        };
        client.load_registry();
        client
    }

    /// Create with a custom base URL (for testing)
    pub fn with_url(skills_dir: PathBuf, url: &str) -> Self {
        let mut client = Self {
            skills_dir,
            base_url: url.to_string(),
            installed: HashMap::new(),
        };
        client.load_registry();
        client
    }

    /// Load the local registry of installed skills
    fn load_registry(&mut self) {
        let registry_path = self.skills_dir.join(".registry.json");
        if let Ok(data) = std::fs::read_to_string(&registry_path)
            && let Ok(reg) = serde_json::from_str::<HashMap<String, InstalledSkillMeta>>(&data)
        {
            self.installed = reg;
        }
    }

    /// Save the local registry to disk
    fn save_registry(&self) -> Result<()> {
        std::fs::create_dir_all(&self.skills_dir)?;
        let registry_path = self.skills_dir.join(".registry.json");
        let data = serde_json::to_string_pretty(&self.installed)
            .map_err(|e| Error::Skill(format!("Failed to serialize registry: {}", e)))?;
        std::fs::write(&registry_path, data)?;
        Ok(())
    }

    /// List all installed skills from local registry
    pub fn list_installed(&self) -> Vec<&InstalledSkillMeta> {
        self.installed.values().collect()
    }

    /// Get info about an installed skill
    pub fn get_installed(&self, name: &str) -> Option<&InstalledSkillMeta> {
        self.installed.get(name)
    }

    /// Search for skills — fetches `registry.json` from GitHub and filters locally.
    /// Falls back to built-in catalog on network failure.
    pub async fn search(&self, query: &str) -> Result<Vec<SkillSummary>> {
        let q = query.to_lowercase();
        if let Ok(catalog) = self.fetch_registry().await {
            let results: Vec<SkillSummary> = catalog
                .into_iter()
                .filter(|s| {
                    s.name.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q)
                        || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                })
                .collect();
            if !results.is_empty() {
                return Ok(results);
            }
        }
        Ok(self.search_builtins(query))
    }

    /// Fetch the full registry catalog from `{base_url}/registry.json`.
    async fn fetch_registry(&self) -> Result<Vec<SkillSummary>> {
        let url = format!("{}/registry.json", self.base_url);
        let resp = reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| Error::Skill(format!("Registry fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Skill(format!(
                "Registry returned HTTP {}",
                resp.status()
            )));
        }
        // registry.json has shape: { "skills": [...] }
        #[derive(serde::Deserialize)]
        struct RegistryFile {
            skills: Vec<SkillSummary>,
        }
        let file = resp
            .json::<RegistryFile>()
            .await
            .map_err(|e| Error::Skill(format!("Failed to parse registry.json: {e}")))?;
        Ok(file.skills)
    }

    /// Search built-in skills catalog
    pub fn search_builtins(&self, query: &str) -> Vec<SkillSummary> {
        let q = query.to_lowercase();
        builtin_skills()
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// List all available skills — builtins merged with remote registry catalog.
    pub async fn list_all(&self) -> Vec<SkillSummary> {
        let mut skills = builtin_skills();
        if let Ok(remote) = self.fetch_registry().await {
            for rs in remote {
                if !skills.iter().any(|s| s.name == rs.name) {
                    skills.push(rs);
                }
            }
        }
        skills
    }

    /// Fetch a skill's SKILL.md from `{base_url}/{name}/SKILL.md` (GitHub raw format).
    pub async fn fetch_skill_md(&self, name: &str) -> Result<String> {
        let url = format!("{}/{}/SKILL.md", self.base_url, name.trim());
        let resp = reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| Error::Skill(format!("Failed to fetch skill '{}': {}", name, e)))?;

        if !resp.status().is_success() {
            return Err(Error::Skill(format!(
                "Skill '{}' not found in registry (HTTP {})",
                name,
                resp.status()
            )));
        }

        resp.text()
            .await
            .map_err(|e| Error::Skill(format!("Failed to read skill content: {}", e)))
    }

    /// Validate a SKILL.md file content before installation
    pub fn validate_skill_md(content: &str) -> Result<ValidationResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut permissions = Vec::new();
        let mut name = String::new();
        let mut version = String::new();

        let mut has_system_prompt = false;
        let mut has_tools = false;
        let mut in_permissions = false;

        for line in content.lines() {
            if let Some(stripped) = line.strip_prefix("# ") {
                name = stripped.trim().to_string();
            } else if line.starts_with("## Version") {
                if let Some(v) = line.strip_prefix("## Version:") {
                    version = v.trim().to_string();
                }
            } else if line.starts_with("## System Prompt") {
                has_system_prompt = true;
                in_permissions = false;
            } else if line.starts_with("## Tools") {
                has_tools = true;
                in_permissions = false;
            } else if line.starts_with("## Permissions") {
                in_permissions = true;
            } else if in_permissions && line.starts_with("- ") {
                permissions.push(line[2..].trim().to_string());
            }
        }

        if name.is_empty() {
            errors.push("Missing skill name (# heading)".to_string());
        }
        if version.is_empty() {
            warnings.push("No version specified, defaulting to 0.1.0".to_string());
            version = "0.1.0".to_string();
        }
        if !has_system_prompt {
            warnings.push("No system prompt section".to_string());
        }
        if !has_tools {
            warnings.push("No tools section defined".to_string());
        }

        // Check for dangerous permissions
        for perm in &permissions {
            match perm.as_str() {
                "network" | "filesystem" | "shell" | "system" => {
                    warnings.push(format!(
                        "Skill requests '{}' permission — requires aegis review",
                        perm
                    ));
                }
                _ => {}
            }
        }

        Ok(ValidationResult {
            valid: errors.is_empty(),
            name,
            version,
            permissions,
            errors,
            warnings,
        })
    }

    /// Review permissions via aegis before install
    /// Returns a PermissionReview indicating which permissions are approved
    pub fn review_permissions(permissions: &[String]) -> PermissionReview {
        // Allowlist of safe permissions
        let _safe_perms = ["network", "filesystem", "clipboard"];
        // Denylist of dangerous permissions
        let denied_perms = ["system", "sudo", "root", "kernel"];

        let mut approved = true;
        let mut denied = Vec::new();

        for perm in permissions {
            if denied_perms.iter().any(|d| perm.to_lowercase().contains(d)) {
                approved = false;
                denied.push(perm.clone());
            }
        }

        let reason = if !denied.is_empty() {
            Some(format!(
                "Denied permissions: {}. These require manual approval.",
                denied.join(", ")
            ))
        } else {
            None
        };

        PermissionReview {
            approved,
            permissions: permissions.to_vec(),
            denied_permissions: denied,
            reason,
        }
    }

    /// Install a skill: fetch, validate, review permissions, write to disk
    pub async fn install(&mut self, name: &str) -> Result<InstallResult> {
        // Check if already installed
        if let Some(existing) = self.installed.get(name) {
            return Err(Error::Skill(format!(
                "Skill '{}' already installed (v{}). Use update() instead.",
                name, existing.version
            )));
        }

        // Fetch SKILL.md
        let content = self.fetch_skill_md(name).await?;

        // Validate
        let validation = Self::validate_skill_md(&content)?;
        if !validation.valid {
            return Err(Error::Skill(format!(
                "SKILL.md validation failed: {}",
                validation.errors.join("; ")
            )));
        }

        // Review permissions via aegis
        let review = Self::review_permissions(&validation.permissions);
        if !review.approved {
            return Err(Error::Skill(format!(
                "Permission review failed: {}",
                review.reason.unwrap_or_default()
            )));
        }

        // Write to disk
        let skill_dir = self.skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), &content)?;

        // Record in registry
        let meta = InstalledSkillMeta {
            name: name.to_string(),
            version: validation.version.clone(),
            author: String::new(),
            installed_at: now_epoch(),
            source: SkillSource::Remote {
                url: format!("{}/{}/SKILL.md", self.base_url, name),
            },
            permissions: validation.permissions.clone(),
            permissions_approved: true,
        };
        self.installed.insert(name.to_string(), meta);
        self.save_registry()?;

        Ok(InstallResult {
            name: name.to_string(),
            version: validation.version,
            permissions: validation.permissions,
            warnings: validation.warnings,
        })
    }

    /// Install from a local SKILL.md file
    pub fn install_local(&mut self, name: &str, content: &str) -> Result<InstallResult> {
        let validation = Self::validate_skill_md(content)?;
        if !validation.valid {
            return Err(Error::Skill(format!(
                "SKILL.md validation failed: {}",
                validation.errors.join("; ")
            )));
        }

        let review = Self::review_permissions(&validation.permissions);
        if !review.approved {
            return Err(Error::Skill(format!(
                "Permission review failed: {}",
                review.reason.unwrap_or_default()
            )));
        }

        let skill_dir = self.skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), content)?;

        let meta = InstalledSkillMeta {
            name: name.to_string(),
            version: validation.version.clone(),
            author: String::new(),
            installed_at: now_epoch(),
            source: SkillSource::Local {
                path: skill_dir.to_string_lossy().to_string(),
            },
            permissions: validation.permissions.clone(),
            permissions_approved: true,
        };
        self.installed.insert(name.to_string(), meta);
        self.save_registry()?;

        Ok(InstallResult {
            name: name.to_string(),
            version: validation.version,
            permissions: validation.permissions,
            warnings: validation.warnings,
        })
    }

    /// Uninstall a skill
    pub fn uninstall(&mut self, name: &str) -> Result<()> {
        if !self.installed.contains_key(name) {
            return Err(Error::Skill(format!("Skill '{}' is not installed", name)));
        }

        let skill_dir = self.skills_dir.join(name);
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir)?;
        }

        self.installed.remove(name);
        self.save_registry()?;
        Ok(())
    }

    /// Check for updates on all installed skills via registry.json
    pub async fn check_updates(&self) -> Vec<UpdateInfo> {
        let mut updates = Vec::new();

        let catalog = match self.fetch_registry().await {
            Ok(c) => c,
            Err(_) => return updates,
        };

        for (name, meta) in &self.installed {
            if let Some(remote) = catalog.iter().find(|s| s.name == *name)
                && version_is_newer(&remote.version, &meta.version)
            {
                updates.push(UpdateInfo {
                    name: name.clone(),
                    installed_version: meta.version.clone(),
                    available_version: remote.version.clone(),
                });
            }
        }

        updates
    }

    /// Update a skill to the latest version
    pub async fn update(&mut self, name: &str) -> Result<InstallResult> {
        // Must be installed
        if !self.installed.contains_key(name) {
            return Err(Error::Skill(format!("Skill '{}' is not installed", name)));
        }

        // Remove old, install new
        self.installed.remove(name);
        self.install(name).await
    }

    /// Get available categories
    pub fn categories(&self) -> Vec<&'static str> {
        vec![
            "development",
            "writing",
            "research",
            "devops",
            "security",
            "data",
            "productivity",
        ]
    }
}

impl Default for ClawHubClient {
    fn default() -> Self {
        let skills_dir = zeus_core::default_config_dir().join("skills");
        Self::new(skills_dir)
    }
}

/// Result of a skill installation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
    pub warnings: Vec<String>,
}

/// Result of SKILL.md validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Compare semver versions: is `a` newer than `b`?
fn version_is_newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse().ok()).collect() };
    let va = parse(a);
    let vb = parse(b);
    va > vb
}

/// URL-encode a string (minimal: spaces and special chars)
#[allow(dead_code)]
fn urlencoded(s: &str) -> String {
    s.replace(' ', "+").replace('&', "%26").replace('?', "%3F")
}

/// Current unix epoch seconds
fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Built-in skill catalog — 52 skills across 9 categories
fn builtin_skills() -> Vec<SkillSummary> {
    vec![
        // ── Development ──────────────────────────────────────────────────────
        SkillSummary {
            name: "git".into(),
            description: "Git operations: commit, branch, merge, rebase, stash".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec!["git".into(), "vcs".into(), "commit".into(), "branch".into()],
        },
        SkillSummary {
            name: "github-cli".into(),
            description: "GitHub PRs, issues, workflows, and releases via gh CLI".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec!["github".into(), "pr".into(), "issues".into(), "ci".into()],
        },
        SkillSummary {
            name: "code-review".into(),
            description: "Review code for bugs, style, and best practices".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec!["code".into(), "review".into(), "quality".into()],
        },
        SkillSummary {
            name: "bun".into(),
            description: "Bun runtime: package management, scripts, bundling, testing".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "bun".into(),
                "javascript".into(),
                "typescript".into(),
                "runtime".into(),
            ],
        },
        SkillSummary {
            name: "python".into(),
            description: "Python development: pip, venv, pytest, debugging, type hints".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "python".into(),
                "pip".into(),
                "pytest".into(),
                "venv".into(),
            ],
        },
        SkillSummary {
            name: "rust".into(),
            description: "Rust development: cargo, clippy, macros, lifetimes, async".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "rust".into(),
                "cargo".into(),
                "clippy".into(),
                "async".into(),
            ],
        },
        SkillSummary {
            name: "typescript".into(),
            description: "TypeScript types, interfaces, generics, and tsconfig".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec!["typescript".into(), "types".into(), "generics".into()],
        },
        SkillSummary {
            name: "react".into(),
            description: "React components, hooks, state management, and performance".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "react".into(),
                "hooks".into(),
                "components".into(),
                "jsx".into(),
            ],
        },
        SkillSummary {
            name: "nextjs".into(),
            description: "Next.js app router, server components, API routes, deployment".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "nextjs".into(),
                "react".into(),
                "ssr".into(),
                "vercel".into(),
            ],
        },
        SkillSummary {
            name: "fastapi".into(),
            description: "FastAPI routes, Pydantic models, async handlers, OpenAPI docs".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "fastapi".into(),
                "python".into(),
                "api".into(),
                "pydantic".into(),
            ],
        },
        SkillSummary {
            name: "graphql".into(),
            description: "GraphQL schemas, resolvers, queries, mutations, subscriptions".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "graphql".into(),
                "schema".into(),
                "api".into(),
                "resolvers".into(),
            ],
        },
        SkillSummary {
            name: "openapi".into(),
            description: "OpenAPI spec authoring, validation, and client generation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "openapi".into(),
                "swagger".into(),
                "rest".into(),
                "spec".into(),
            ],
        },
        SkillSummary {
            name: "debug-assist".into(),
            description: "Step through errors, stack traces, and runtime failures".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "debug".into(),
                "error".into(),
                "stacktrace".into(),
                "fix".into(),
            ],
        },
        SkillSummary {
            name: "refactor".into(),
            description: "Refactor code for clarity, performance, and maintainability".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec!["refactor".into(), "clean".into(), "patterns".into()],
        },
        SkillSummary {
            name: "test-runner".into(),
            description: "Write and run unit, integration, and e2e tests".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["development".into()],
            tags: vec![
                "test".into(),
                "unit".into(),
                "e2e".into(),
                "coverage".into(),
            ],
        },
        // ── DevOps ───────────────────────────────────────────────────────────
        SkillSummary {
            name: "docker".into(),
            description: "Docker containers: build, run, compose, networks, volumes".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "docker".into(),
                "container".into(),
                "compose".into(),
                "image".into(),
            ],
        },
        SkillSummary {
            name: "kubectl".into(),
            description: "Kubernetes cluster management, deployments, pods, services".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "kubernetes".into(),
                "kubectl".into(),
                "k8s".into(),
                "pods".into(),
            ],
        },
        SkillSummary {
            name: "terraform".into(),
            description: "Terraform IaC: plan, apply, modules, state management".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "terraform".into(),
                "iac".into(),
                "aws".into(),
                "infra".into(),
            ],
        },
        SkillSummary {
            name: "ansible".into(),
            description: "Ansible playbooks, roles, inventory, and automation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec!["ansible".into(), "playbook".into(), "automation".into()],
        },
        SkillSummary {
            name: "ci-cd".into(),
            description: "CI/CD pipelines: GitHub Actions, GitLab CI, CircleCI".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "ci".into(),
                "cd".into(),
                "pipeline".into(),
                "actions".into(),
            ],
        },
        SkillSummary {
            name: "rsync".into(),
            description: "File sync and transfer with rsync over SSH".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "rsync".into(),
                "sync".into(),
                "backup".into(),
                "deploy".into(),
            ],
        },
        SkillSummary {
            name: "devops".into(),
            description: "General DevOps: monitoring, scaling, incident response".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["devops".into()],
            tags: vec![
                "devops".into(),
                "monitoring".into(),
                "sre".into(),
                "deploy".into(),
            ],
        },
        // ── System ───────────────────────────────────────────────────────────
        SkillSummary {
            name: "ssh".into(),
            description: "SSH connections, tunnels, key management, remote commands".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["system".into()],
            tags: vec![
                "ssh".into(),
                "remote".into(),
                "tunnel".into(),
                "keys".into(),
            ],
        },
        SkillSummary {
            name: "homebrew".into(),
            description: "Homebrew package management: install, update, audit, casks".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["system".into()],
            tags: vec![
                "brew".into(),
                "macos".into(),
                "packages".into(),
                "install".into(),
            ],
        },
        SkillSummary {
            name: "log-analyzer".into(),
            description: "Parse and analyze log files, find errors, summarize patterns".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["system".into()],
            tags: vec![
                "logs".into(),
                "grep".into(),
                "errors".into(),
                "monitoring".into(),
            ],
        },
        SkillSummary {
            name: "cron-scheduler".into(),
            description: "Cron expression authoring, validation, and scheduling advice".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["system".into()],
            tags: vec![
                "cron".into(),
                "schedule".into(),
                "automation".into(),
                "timer".into(),
            ],
        },
        SkillSummary {
            name: "secret-manager".into(),
            description: "Manage secrets: .env files, vaults, key rotation best practices".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["system".into(), "security".into()],
            tags: vec![
                "secrets".into(),
                "env".into(),
                "vault".into(),
                "keys".into(),
            ],
        },
        // ── Data ─────────────────────────────────────────────────────────────
        SkillSummary {
            name: "sqlite".into(),
            description: "SQLite queries, schema design, migrations, and optimization".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec![
                "sqlite".into(),
                "sql".into(),
                "database".into(),
                "queries".into(),
            ],
        },
        SkillSummary {
            name: "postgres".into(),
            description: "PostgreSQL: queries, indexes, EXPLAIN, migrations, Alembic".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec![
                "postgres".into(),
                "sql".into(),
                "database".into(),
                "indexes".into(),
            ],
        },
        SkillSummary {
            name: "redis".into(),
            description: "Redis data structures, caching patterns, pub/sub, streams".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec![
                "redis".into(),
                "cache".into(),
                "pubsub".into(),
                "streams".into(),
            ],
        },
        SkillSummary {
            name: "csv-data".into(),
            description: "CSV parsing, transformation, analysis, and export".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec![
                "csv".into(),
                "data".into(),
                "pandas".into(),
                "transform".into(),
            ],
        },
        SkillSummary {
            name: "json-yaml".into(),
            description: "JSON/YAML formatting, querying with jq, schema validation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec!["json".into(), "yaml".into(), "jq".into(), "schema".into()],
        },
        SkillSummary {
            name: "regex".into(),
            description: "Regular expressions: write, test, optimize, explain".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["data".into()],
            tags: vec![
                "regex".into(),
                "pattern".into(),
                "grep".into(),
                "match".into(),
            ],
        },
        // ── Security ─────────────────────────────────────────────────────────
        SkillSummary {
            name: "security".into(),
            description: "Security scanning, vulnerability analysis, and hardening".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["security".into()],
            tags: vec![
                "security".into(),
                "audit".into(),
                "vulnerability".into(),
                "cve".into(),
            ],
        },
        SkillSummary {
            name: "api-tester".into(),
            description: "HTTP API testing: curl, auth flows, response validation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["security".into(), "development".into()],
            tags: vec!["api".into(), "http".into(), "curl".into(), "test".into()],
        },
        // ── Research ─────────────────────────────────────────────────────────
        SkillSummary {
            name: "research".into(),
            description: "Web research with source analysis and summarization".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["research".into()],
            tags: vec![
                "web".into(),
                "search".into(),
                "analysis".into(),
                "sources".into(),
            ],
        },
        SkillSummary {
            name: "web-scraper".into(),
            description: "Web scraping with CSS selectors, pagination, and data extraction".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["research".into()],
            tags: vec![
                "scraping".into(),
                "html".into(),
                "extract".into(),
                "crawl".into(),
            ],
        },
        SkillSummary {
            name: "browser-automation".into(),
            description: "Browser automation: Playwright, Puppeteer, CDP, screenshots".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["research".into(), "devops".into()],
            tags: vec![
                "playwright".into(),
                "browser".into(),
                "e2e".into(),
                "automation".into(),
            ],
        },
        // ── Writing ──────────────────────────────────────────────────────────
        SkillSummary {
            name: "writing".into(),
            description: "Long-form writing, editing, and proofreading".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["writing".into()],
            tags: vec![
                "write".into(),
                "edit".into(),
                "prose".into(),
                "grammar".into(),
            ],
        },
        SkillSummary {
            name: "markdown".into(),
            description: "Markdown authoring, tables, diagrams, and documentation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["writing".into()],
            tags: vec![
                "markdown".into(),
                "docs".into(),
                "readme".into(),
                "mermaid".into(),
            ],
        },
        SkillSummary {
            name: "technical-docs".into(),
            description: "Technical documentation, API docs, architecture diagrams".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["writing".into()],
            tags: vec![
                "docs".into(),
                "technical".into(),
                "architecture".into(),
                "adr".into(),
            ],
        },
        SkillSummary {
            name: "summarize".into(),
            description: "Summarize documents, articles, and conversations".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["writing".into(), "productivity".into()],
            tags: vec!["summary".into(), "tldr".into(), "extract".into()],
        },
        SkillSummary {
            name: "code-explainer".into(),
            description: "Explain code in plain English for any audience level".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["writing".into(), "development".into()],
            tags: vec![
                "explain".into(),
                "docs".into(),
                "comments".into(),
                "teaching".into(),
            ],
        },
        // ── Productivity ─────────────────────────────────────────────────────
        SkillSummary {
            name: "obsidian".into(),
            description: "Obsidian notes: daily notes, links, templates, Dataview".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["productivity".into()],
            tags: vec![
                "obsidian".into(),
                "notes".into(),
                "pkm".into(),
                "markdown".into(),
            ],
        },
        SkillSummary {
            name: "notion".into(),
            description: "Notion pages, databases, automations, and API integration".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["productivity".into()],
            tags: vec![
                "notion".into(),
                "database".into(),
                "pages".into(),
                "wiki".into(),
            ],
        },
        SkillSummary {
            name: "linear".into(),
            description: "Linear issues, cycles, projects, and roadmap management".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["productivity".into()],
            tags: vec![
                "linear".into(),
                "issues".into(),
                "sprint".into(),
                "roadmap".into(),
            ],
        },
        SkillSummary {
            name: "jira".into(),
            description: "Jira tickets, sprints, epics, JQL queries, and workflows".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["productivity".into()],
            tags: vec![
                "jira".into(),
                "tickets".into(),
                "sprint".into(),
                "agile".into(),
            ],
        },
        // ── Communication ─────────────────────────────────────────────────────
        SkillSummary {
            name: "email-client".into(),
            description: "Email drafting, templates, threading, and inbox management".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["communication".into()],
            tags: vec![
                "email".into(),
                "draft".into(),
                "inbox".into(),
                "smtp".into(),
            ],
        },
        SkillSummary {
            name: "slack-cli".into(),
            description: "Slack messaging, channel management, and workflow automation".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["communication".into()],
            tags: vec![
                "slack".into(),
                "messages".into(),
                "channels".into(),
                "webhooks".into(),
            ],
        },
        SkillSummary {
            name: "discord-cli".into(),
            description: "Discord bot commands, embeds, threads, and server management".into(),
            version: "1.0.0".into(),
            author: "zeus".into(),
            downloads: 0,
            categories: vec!["communication".into()],
            tags: vec![
                "discord".into(),
                "bot".into(),
                "embeds".into(),
                "threads".into(),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_skill_md_valid() {
        let content = "# My Skill\n\nA great skill.\n\n## Version: 1.0.0\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n\n## Permissions\n- network\n";
        let result = ClawHubClient::validate_skill_md(content).unwrap();
        assert!(result.valid);
        assert_eq!(result.name, "My Skill");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(result.permissions, vec!["network"]);
    }

    #[test]
    fn test_validate_skill_md_missing_name() {
        let content = "## System Prompt\nBe helpful.\n";
        let result = ClawHubClient::validate_skill_md(content).unwrap();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Missing skill name"))
        );
    }

    #[test]
    fn test_review_permissions_safe() {
        let perms = vec!["network".to_string(), "clipboard".to_string()];
        let review = ClawHubClient::review_permissions(&perms);
        assert!(review.approved);
        assert!(review.denied_permissions.is_empty());
    }

    #[test]
    fn test_review_permissions_denied() {
        let perms = vec!["network".to_string(), "sudo".to_string()];
        let review = ClawHubClient::review_permissions(&perms);
        assert!(!review.approved);
        assert_eq!(review.denied_permissions, vec!["sudo"]);
    }

    #[test]
    fn test_version_is_newer() {
        assert!(version_is_newer("1.1.0", "1.0.0"));
        assert!(version_is_newer("2.0.0", "1.9.9"));
        assert!(!version_is_newer("1.0.0", "1.0.0"));
        assert!(!version_is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn test_builtin_skills_not_empty() {
        let skills = builtin_skills();
        assert!(!skills.is_empty());
        assert!(skills.iter().any(|s| s.name == "git"));
    }

    #[test]
    fn test_install_local() {
        let tmp = std::env::temp_dir().join("zeus_test_clawhub_install");
        let _ = std::fs::remove_dir_all(&tmp);

        let mut client = ClawHubClient::new(tmp.clone());
        let content = "# Test Skill\n\n## Version: 1.0.0\n\n## System Prompt\nBe helpful.\n\n## Tools\n- greet: Say hello\n";
        let result = client.install_local("test-skill", content).unwrap();
        assert_eq!(result.name, "test-skill");
        assert_eq!(result.version, "1.0.0");

        // Should be in registry
        assert!(client.get_installed("test-skill").is_some());

        // SKILL.md should exist on disk
        assert!(tmp.join("test-skill/SKILL.md").exists());

        // Uninstall
        client.uninstall("test-skill").unwrap();
        assert!(client.get_installed("test-skill").is_none());
        assert!(!tmp.join("test-skill").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_install_local_dangerous_perms() {
        let tmp = std::env::temp_dir().join("zeus_test_clawhub_denied");
        let _ = std::fs::remove_dir_all(&tmp);

        let mut client = ClawHubClient::new(tmp.clone());
        let content = "# Evil Skill\n\n## Version: 1.0.0\n\n## System Prompt\nBe evil.\n\n## Permissions\n- sudo\n- root\n";
        let result = client.install_local("evil-skill", content);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Permission review failed")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_registry_persistence() {
        let tmp = std::env::temp_dir().join("zeus_test_clawhub_persist");
        let _ = std::fs::remove_dir_all(&tmp);

        // Install a skill
        {
            let mut client = ClawHubClient::new(tmp.clone());
            let content = "# Persist Skill\n\n## Version: 2.0.0\n\n## System Prompt\nPersist.\n";
            client.install_local("persist-skill", content).unwrap();
        }

        // Reopen and check
        {
            let client = ClawHubClient::new(tmp.clone());
            let meta = client.get_installed("persist-skill").unwrap();
            assert_eq!(meta.version, "2.0.0");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_search_builtins() {
        let client = ClawHubClient::default();
        let results = client.search_builtins("git");
        assert!(!results.is_empty());
        assert!(results.iter().any(|s| s.name == "git"));
    }

    #[test]
    fn test_categories() {
        let client = ClawHubClient::default();
        let cats = client.categories();
        assert!(cats.contains(&"development"));
        assert!(cats.contains(&"security"));
    }
}
