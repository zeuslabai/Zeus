//! Skill Versioning — semver-based version management for Zeus skills.
//!
//! Provides:
//! - Semantic versioning (semver) for skills via SKILL.md `## Version:` field
//! - Version pinning on install (exact, range, latest)
//! - Upgrade detection: compare installed vs available versions
//! - Rollback support: keep previous version in `.rollback/` alongside current
//! - Dependency version constraints between skills
//!
//! Integrates with SkillInstaller for version-aware installation and SkillManager
//! for version queries.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

// ============================================================================
// Semver types (self-contained, no external crate needed)
// ============================================================================

/// Semantic version: major.minor.patch with optional pre-release tag.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SkillVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// Optional pre-release label (e.g., "alpha.1", "beta.2").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<String>,
}

impl SkillVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
        }
    }

    pub fn with_pre(major: u32, minor: u32, patch: u32, pre: &str) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: if pre.is_empty() {
                None
            } else {
                Some(pre.to_string())
            },
        }
    }

    /// Parse a version string like "1.2.3" or "1.2.3-alpha.1".
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let (version_part, pre) = if let Some(idx) = s.find('-') {
            (&s[..idx], Some(s[idx + 1..].to_string()))
        } else {
            (s, None)
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = if parts.len() == 3 {
            parts[2].parse().ok()?
        } else {
            0
        };

        Some(Self {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// Check if this is a pre-release version.
    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }

    /// Check if this version satisfies a version requirement.
    pub fn satisfies(&self, req: &VersionReq) -> bool {
        req.matches(self)
    }
}

impl fmt::Display for SkillVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl Ord for SkillVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then_with(|| {
                // Pre-release < release (1.0.0-alpha < 1.0.0)
                match (&self.pre, &other.pre) {
                    (None, None) => Ordering::Equal,
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (Some(a), Some(b)) => a.cmp(b),
                }
            })
    }
}

impl PartialOrd for SkillVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ============================================================================
// Version requirement (constraint)
// ============================================================================

/// Version requirement that a dependency can specify.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VersionReq {
    /// Exact version: "=1.2.3"
    Exact { version: SkillVersion },
    /// Minimum version (inclusive): ">=1.2.0"
    MinInclusive { version: SkillVersion },
    /// Compatible (same major): "^1.2.3" means >=1.2.3 and <2.0.0
    Compatible { version: SkillVersion },
    /// Tilde (same major.minor): "~1.2.3" means >=1.2.3 and <1.3.0
    Tilde { version: SkillVersion },
    /// Any version
    Any,
}

impl VersionReq {
    /// Parse a version requirement string.
    ///
    /// Supported formats: "=1.2.3", ">=1.0", "^1.2", "~1.2.3", "*", "1.2.3" (exact).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s == "*" || s.is_empty() {
            return Some(VersionReq::Any);
        }

        if let Some(rest) = s.strip_prefix(">=") {
            let v = SkillVersion::parse(rest)?;
            Some(VersionReq::MinInclusive { version: v })
        } else if let Some(rest) = s.strip_prefix('^') {
            let v = SkillVersion::parse(rest)?;
            Some(VersionReq::Compatible { version: v })
        } else if let Some(rest) = s.strip_prefix('~') {
            let v = SkillVersion::parse(rest)?;
            Some(VersionReq::Tilde { version: v })
        } else if let Some(rest) = s.strip_prefix('=') {
            let v = SkillVersion::parse(rest)?;
            Some(VersionReq::Exact { version: v })
        } else {
            // Bare version = exact
            let v = SkillVersion::parse(s)?;
            Some(VersionReq::Exact { version: v })
        }
    }

    /// Check if a version matches this requirement.
    pub fn matches(&self, v: &SkillVersion) -> bool {
        match self {
            VersionReq::Any => true,
            VersionReq::Exact { version } => v == version,
            VersionReq::MinInclusive { version } => v >= version,
            VersionReq::Compatible { version } => v >= version && v.major == version.major,
            VersionReq::Tilde { version } => {
                v >= version && v.major == version.major && v.minor == version.minor
            }
        }
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionReq::Any => write!(f, "*"),
            VersionReq::Exact { version } => write!(f, "={version}"),
            VersionReq::MinInclusive { version } => write!(f, ">={version}"),
            VersionReq::Compatible { version } => write!(f, "^{version}"),
            VersionReq::Tilde { version } => write!(f, "~{version}"),
        }
    }
}

// ============================================================================
// Skill dependency with version constraint
// ============================================================================

/// A dependency on another skill with a version constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDependency {
    /// Name of the required skill.
    pub name: String,
    /// Version requirement.
    pub version_req: VersionReq,
}

impl SkillDependency {
    pub fn new(name: &str, req: VersionReq) -> Self {
        Self {
            name: name.to_string(),
            version_req: req,
        }
    }
}

// ============================================================================
// Installed version record
// ============================================================================

/// Record of an installed skill version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillVersion {
    /// Skill name.
    pub name: String,
    /// Current installed version.
    pub version: SkillVersion,
    /// Installation timestamp (RFC3339).
    pub installed_at: String,
    /// Previous version (if upgraded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<SkillVersion>,
    /// Whether a rollback snapshot exists.
    pub has_rollback: bool,
}

// ============================================================================
// Version registry (persisted to disk)
// ============================================================================

/// Registry tracking installed skill versions.
pub struct SkillVersionRegistry {
    /// Map of skill_name → version record.
    versions: HashMap<String, InstalledSkillVersion>,
    /// Path to persist version data.
    persist_path: Option<PathBuf>,
}

impl SkillVersionRegistry {
    /// Create an in-memory registry.
    pub fn new() -> Self {
        Self {
            versions: HashMap::new(),
            persist_path: None,
        }
    }

    /// Create a registry with file persistence.
    pub fn with_persistence(path: PathBuf) -> Self {
        Self {
            versions: HashMap::new(),
            persist_path: Some(path),
        }
    }

    /// Load versions from the persistence file.
    pub fn load(&mut self) -> usize {
        let Some(path) = &self.persist_path else {
            return 0;
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return 0;
        };
        let Ok(map) = serde_json::from_str::<HashMap<String, InstalledSkillVersion>>(&content)
        else {
            return 0;
        };
        let count = map.len();
        self.versions = map;
        count
    }

    /// Persist current versions to disk.
    fn persist(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.versions) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Register an installed skill version.
    pub fn register(&mut self, name: &str, version: SkillVersion) {
        let now = chrono::Utc::now().to_rfc3339();
        let previous = self.versions.get(name).map(|r| r.version.clone());

        self.versions.insert(
            name.to_string(),
            InstalledSkillVersion {
                name: name.to_string(),
                version,
                installed_at: now,
                previous_version: previous,
                has_rollback: false,
            },
        );
        self.persist();
    }

    /// Register an upgrade (marks rollback available).
    pub fn register_upgrade(&mut self, name: &str, new_version: SkillVersion) {
        let now = chrono::Utc::now().to_rfc3339();
        let previous = self.versions.get(name).map(|r| r.version.clone());

        self.versions.insert(
            name.to_string(),
            InstalledSkillVersion {
                name: name.to_string(),
                version: new_version,
                installed_at: now,
                previous_version: previous,
                has_rollback: true,
            },
        );
        self.persist();
    }

    /// Remove a skill from the registry.
    pub fn remove(&mut self, name: &str) -> bool {
        let removed = self.versions.remove(name).is_some();
        if removed {
            self.persist();
        }
        removed
    }

    /// Get the installed version of a skill.
    pub fn get(&self, name: &str) -> Option<&InstalledSkillVersion> {
        self.versions.get(name)
    }

    /// Get the version of a skill.
    pub fn get_version(&self, name: &str) -> Option<&SkillVersion> {
        self.versions.get(name).map(|r| &r.version)
    }

    /// Check if a skill is installed with a version satisfying a requirement.
    pub fn satisfies(&self, name: &str, req: &VersionReq) -> bool {
        self.versions
            .get(name)
            .map(|r| req.matches(&r.version))
            .unwrap_or(false)
    }

    /// List all installed versions.
    pub fn list(&self) -> Vec<&InstalledSkillVersion> {
        let mut list: Vec<_> = self.versions.values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Find skills that can be upgraded given a map of available versions.
    pub fn find_upgrades(&self, available: &HashMap<String, SkillVersion>) -> Vec<UpgradeInfo> {
        let mut upgrades = Vec::new();
        for (name, record) in &self.versions {
            if let Some(latest) = available.get(name)
                && latest > &record.version
            {
                upgrades.push(UpgradeInfo {
                    name: name.clone(),
                    current: record.version.clone(),
                    available: latest.clone(),
                });
            }
        }
        upgrades.sort_by(|a, b| a.name.cmp(&b.name));
        upgrades
    }

    /// Total number of tracked skills.
    pub fn count(&self) -> usize {
        self.versions.len()
    }
}

impl Default for SkillVersionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Info about an available upgrade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeInfo {
    pub name: String,
    pub current: SkillVersion,
    pub available: SkillVersion,
}

// ============================================================================
// Rollback support
// ============================================================================

/// Create a rollback snapshot before upgrading.
pub fn create_rollback_snapshot(skills_dir: &Path, skill_name: &str) -> std::io::Result<PathBuf> {
    let skill_dir = skills_dir.join(skill_name);
    let rollback_dir = skills_dir.join(format!(".rollback-{skill_name}"));

    if !skill_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Skill directory not found: {}", skill_dir.display()),
        ));
    }

    // Remove old rollback if exists
    if rollback_dir.exists() {
        std::fs::remove_dir_all(&rollback_dir)?;
    }

    // Copy current to rollback
    copy_dir(&skill_dir, &rollback_dir)?;

    Ok(rollback_dir)
}

/// Restore a skill from its rollback snapshot.
pub fn restore_rollback(skills_dir: &Path, skill_name: &str) -> std::io::Result<()> {
    let skill_dir = skills_dir.join(skill_name);
    let rollback_dir = skills_dir.join(format!(".rollback-{skill_name}"));

    if !rollback_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No rollback snapshot found",
        ));
    }

    // Remove current
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)?;
    }

    // Move rollback to current
    std::fs::rename(&rollback_dir, &skill_dir)?;

    Ok(())
}

/// Recursive directory copy.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

// ============================================================================
// SKILL.md version parser
// ============================================================================

/// Parse the version from a SKILL.md content string.
/// Looks for `## Version: X.Y.Z` or `## Version: vX.Y.Z`.
pub fn parse_version_from_skill_md(content: &str) -> Option<SkillVersion> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("## Version:") {
            return SkillVersion::parse(rest.trim());
        }
        if let Some(rest) = line.strip_prefix("## Version :") {
            return SkillVersion::parse(rest.trim());
        }
    }
    None
}

/// Parse dependency declarations from SKILL.md.
/// Looks for a `## Dependencies` section with lines like:
/// - `skill-name ^1.2.0`
/// - `other-skill >=2.0`
/// - `base-skill *`
pub fn parse_dependencies_from_skill_md(content: &str) -> Vec<SkillDependency> {
    let mut deps = Vec::new();
    let mut in_deps_section = false;

    for line in content.lines() {
        if line.starts_with("## Dependencies") {
            in_deps_section = true;
            continue;
        }
        if line.starts_with("## ") && in_deps_section {
            break; // Next section
        }
        if in_deps_section && line.starts_with("- ") {
            let entry = line[2..].trim();
            if let Some((name, req_str)) = entry.split_once(' ') {
                if let Some(req) = VersionReq::parse(req_str.trim()) {
                    deps.push(SkillDependency::new(name.trim(), req));
                }
            } else {
                // No version constraint = any
                deps.push(SkillDependency::new(entry, VersionReq::Any));
            }
        }
    }
    deps
}

// ============================================================================
// Dependency resolver
// ============================================================================

/// Result of dependency resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyResolution {
    /// Skills that are satisfied (already installed with correct version).
    pub satisfied: Vec<String>,
    /// Skills that need to be installed.
    pub missing: Vec<SkillDependency>,
    /// Skills that are installed but wrong version.
    pub version_mismatch: Vec<VersionMismatch>,
    /// Whether all dependencies are satisfied.
    pub all_satisfied: bool,
}

/// A version mismatch between installed and required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMismatch {
    pub name: String,
    pub installed: SkillVersion,
    pub required: String,
}

/// Resolve dependencies for a skill against the version registry.
pub fn resolve_dependencies(
    deps: &[SkillDependency],
    registry: &SkillVersionRegistry,
) -> DependencyResolution {
    let mut satisfied = Vec::new();
    let mut missing = Vec::new();
    let mut version_mismatch = Vec::new();

    for dep in deps {
        match registry.get(&dep.name) {
            None => {
                missing.push(dep.clone());
            }
            Some(record) => {
                if dep.version_req.matches(&record.version) {
                    satisfied.push(dep.name.clone());
                } else {
                    version_mismatch.push(VersionMismatch {
                        name: dep.name.clone(),
                        installed: record.version.clone(),
                        required: dep.version_req.to_string(),
                    });
                }
            }
        }
    }

    let all_satisfied = missing.is_empty() && version_mismatch.is_empty();
    DependencyResolution {
        satisfied,
        missing,
        version_mismatch,
        all_satisfied,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- SkillVersion parsing --

    #[test]
    fn test_parse_version_basic() {
        let v = SkillVersion::parse("1.2.3").unwrap();
        assert_eq!(v, SkillVersion::new(1, 2, 3));
    }

    #[test]
    fn test_parse_version_with_v_prefix() {
        let v = SkillVersion::parse("v2.0.1").unwrap();
        assert_eq!(v, SkillVersion::new(2, 0, 1));
    }

    #[test]
    fn test_parse_version_two_parts() {
        let v = SkillVersion::parse("1.5").unwrap();
        assert_eq!(v, SkillVersion::new(1, 5, 0));
    }

    #[test]
    fn test_parse_version_prerelease() {
        let v = SkillVersion::parse("1.0.0-alpha.1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.pre, Some("alpha.1".to_string()));
        assert!(v.is_prerelease());
    }

    #[test]
    fn test_parse_version_invalid() {
        assert!(SkillVersion::parse("").is_none());
        assert!(SkillVersion::parse("abc").is_none());
        assert!(SkillVersion::parse("1").is_none());
        assert!(SkillVersion::parse("1.2.3.4").is_none());
    }

    #[test]
    fn test_version_display() {
        assert_eq!(SkillVersion::new(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(
            SkillVersion::with_pre(1, 0, 0, "beta.2").to_string(),
            "1.0.0-beta.2"
        );
    }

    #[test]
    fn test_version_ordering() {
        let v1 = SkillVersion::new(1, 0, 0);
        let v2 = SkillVersion::new(1, 1, 0);
        let v3 = SkillVersion::new(2, 0, 0);
        let pre = SkillVersion::with_pre(1, 0, 0, "alpha");

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(pre < v1); // pre-release < release
    }

    // -- VersionReq --

    #[test]
    fn test_version_req_exact() {
        let req = VersionReq::parse("=1.2.3").unwrap();
        assert!(req.matches(&SkillVersion::new(1, 2, 3)));
        assert!(!req.matches(&SkillVersion::new(1, 2, 4)));
    }

    #[test]
    fn test_version_req_min_inclusive() {
        let req = VersionReq::parse(">=1.0.0").unwrap();
        assert!(req.matches(&SkillVersion::new(1, 0, 0)));
        assert!(req.matches(&SkillVersion::new(2, 0, 0)));
        assert!(!req.matches(&SkillVersion::new(0, 9, 9)));
    }

    #[test]
    fn test_version_req_compatible() {
        let req = VersionReq::parse("^1.2.0").unwrap();
        assert!(req.matches(&SkillVersion::new(1, 2, 0)));
        assert!(req.matches(&SkillVersion::new(1, 9, 9)));
        assert!(!req.matches(&SkillVersion::new(2, 0, 0)));
        assert!(!req.matches(&SkillVersion::new(1, 1, 9)));
    }

    #[test]
    fn test_version_req_tilde() {
        let req = VersionReq::parse("~1.2.3").unwrap();
        assert!(req.matches(&SkillVersion::new(1, 2, 3)));
        assert!(req.matches(&SkillVersion::new(1, 2, 9)));
        assert!(!req.matches(&SkillVersion::new(1, 3, 0)));
    }

    #[test]
    fn test_version_req_any() {
        let req = VersionReq::parse("*").unwrap();
        assert!(req.matches(&SkillVersion::new(0, 0, 1)));
        assert!(req.matches(&SkillVersion::new(99, 99, 99)));
    }

    #[test]
    fn test_version_req_bare_is_exact() {
        let req = VersionReq::parse("1.2.3").unwrap();
        assert!(req.matches(&SkillVersion::new(1, 2, 3)));
        assert!(!req.matches(&SkillVersion::new(1, 2, 4)));
    }

    // -- SKILL.md parsing --

    #[test]
    fn test_parse_version_from_skill_md() {
        let content = "# My Skill\n\n## Version: 2.1.0\n\n## Tools\n- x: y\n";
        let v = parse_version_from_skill_md(content).unwrap();
        assert_eq!(v, SkillVersion::new(2, 1, 0));
    }

    #[test]
    fn test_parse_version_from_skill_md_missing() {
        let content = "# My Skill\n\n## Tools\n- x: y\n";
        assert!(parse_version_from_skill_md(content).is_none());
    }

    #[test]
    fn test_parse_dependencies_from_skill_md() {
        let content = "# My Skill\n\n## Dependencies\n- base-tools ^1.0.0\n- data-utils >=2.0\n- optional-lib *\n\n## Tools\n";
        let deps = parse_dependencies_from_skill_md(content);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "base-tools");
        assert_eq!(deps[1].name, "data-utils");
        assert_eq!(deps[2].name, "optional-lib");
    }

    #[test]
    fn test_parse_dependencies_no_version() {
        let content = "# Skill\n\n## Dependencies\n- simple-dep\n\n## Tools\n";
        let deps = parse_dependencies_from_skill_md(content);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].version_req.matches(&SkillVersion::new(99, 0, 0)));
    }

    // -- Registry --

    #[test]
    fn test_registry_register_and_get() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("my-skill", SkillVersion::new(1, 0, 0));

        let record = reg.get("my-skill").unwrap();
        assert_eq!(record.version, SkillVersion::new(1, 0, 0));
        assert!(!record.has_rollback);
        assert!(record.previous_version.is_none());
    }

    #[test]
    fn test_registry_upgrade() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("my-skill", SkillVersion::new(1, 0, 0));
        reg.register_upgrade("my-skill", SkillVersion::new(2, 0, 0));

        let record = reg.get("my-skill").unwrap();
        assert_eq!(record.version, SkillVersion::new(2, 0, 0));
        assert_eq!(record.previous_version, Some(SkillVersion::new(1, 0, 0)));
        assert!(record.has_rollback);
    }

    #[test]
    fn test_registry_satisfies() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("my-skill", SkillVersion::new(1, 5, 0));

        assert!(reg.satisfies("my-skill", &VersionReq::parse("^1.0.0").unwrap()));
        assert!(!reg.satisfies("my-skill", &VersionReq::parse("^2.0.0").unwrap()));
        assert!(!reg.satisfies("missing", &VersionReq::parse("*").unwrap()));
    }

    #[test]
    fn test_registry_find_upgrades() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("skill-a", SkillVersion::new(1, 0, 0));
        reg.register("skill-b", SkillVersion::new(2, 0, 0));

        let available: HashMap<String, SkillVersion> = [
            ("skill-a".to_string(), SkillVersion::new(1, 1, 0)),
            ("skill-b".to_string(), SkillVersion::new(2, 0, 0)), // same, no upgrade
            ("skill-c".to_string(), SkillVersion::new(1, 0, 0)), // not installed
        ]
        .into_iter()
        .collect();

        let upgrades = reg.find_upgrades(&available);
        assert_eq!(upgrades.len(), 1);
        assert_eq!(upgrades[0].name, "skill-a");
        assert_eq!(upgrades[0].available, SkillVersion::new(1, 1, 0));
    }

    #[test]
    fn test_registry_remove() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("my-skill", SkillVersion::new(1, 0, 0));
        assert!(reg.remove("my-skill"));
        assert!(reg.get("my-skill").is_none());
        assert!(!reg.remove("my-skill"));
    }

    #[test]
    fn test_registry_persistence() {
        let tmp = std::env::temp_dir().join("zeus_test_version_registry.json");
        let _ = std::fs::remove_file(&tmp);

        // Write
        let mut reg = SkillVersionRegistry::with_persistence(tmp.clone());
        reg.register("persisted-skill", SkillVersion::new(3, 1, 4));
        assert!(tmp.exists());

        // Read in new registry
        let mut reg2 = SkillVersionRegistry::with_persistence(tmp.clone());
        let count = reg2.load();
        assert_eq!(count, 1);
        let record = reg2.get("persisted-skill").unwrap();
        assert_eq!(record.version, SkillVersion::new(3, 1, 4));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_registry_list_sorted() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("zebra", SkillVersion::new(1, 0, 0));
        reg.register("alpha", SkillVersion::new(1, 0, 0));
        reg.register("middle", SkillVersion::new(1, 0, 0));

        let list = reg.list();
        let names: Vec<_> = list.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    // -- Dependency resolution --

    #[test]
    fn test_resolve_all_satisfied() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("base", SkillVersion::new(1, 0, 0));
        reg.register("utils", SkillVersion::new(2, 3, 0));

        let deps = vec![
            SkillDependency::new("base", VersionReq::parse("^1.0.0").unwrap()),
            SkillDependency::new("utils", VersionReq::parse(">=2.0.0").unwrap()),
        ];

        let result = resolve_dependencies(&deps, &reg);
        assert!(result.all_satisfied);
        assert_eq!(result.satisfied.len(), 2);
        assert!(result.missing.is_empty());
        assert!(result.version_mismatch.is_empty());
    }

    #[test]
    fn test_resolve_missing_dependency() {
        let reg = SkillVersionRegistry::new();
        let deps = vec![SkillDependency::new(
            "missing-skill",
            VersionReq::parse("^1.0.0").unwrap(),
        )];

        let result = resolve_dependencies(&deps, &reg);
        assert!(!result.all_satisfied);
        assert_eq!(result.missing.len(), 1);
        assert_eq!(result.missing[0].name, "missing-skill");
    }

    #[test]
    fn test_resolve_version_mismatch() {
        let mut reg = SkillVersionRegistry::new();
        reg.register("old-skill", SkillVersion::new(1, 0, 0));

        let deps = vec![SkillDependency::new(
            "old-skill",
            VersionReq::parse("^2.0.0").unwrap(),
        )];

        let result = resolve_dependencies(&deps, &reg);
        assert!(!result.all_satisfied);
        assert_eq!(result.version_mismatch.len(), 1);
        assert_eq!(
            result.version_mismatch[0].installed,
            SkillVersion::new(1, 0, 0)
        );
    }

    // -- Rollback --

    #[test]
    fn test_create_and_restore_rollback() {
        let tmp = std::env::temp_dir().join("zeus_test_rollback");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).unwrap();
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# V1\nOriginal.").unwrap();

        // Create rollback
        let rollback_path = create_rollback_snapshot(&tmp, "my-skill").unwrap();
        assert!(rollback_path.exists());

        // Overwrite current
        std::fs::write(tmp.join("my-skill/SKILL.md"), "# V2\nUpgraded.").unwrap();
        let content = std::fs::read_to_string(tmp.join("my-skill/SKILL.md")).unwrap();
        assert!(content.contains("V2"));

        // Restore
        restore_rollback(&tmp, "my-skill").unwrap();
        let restored = std::fs::read_to_string(tmp.join("my-skill/SKILL.md")).unwrap();
        assert!(restored.contains("V1"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_missing_skill() {
        let tmp = std::env::temp_dir().join("zeus_test_rollback_missing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let result = create_rollback_snapshot(&tmp, "nonexistent");
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_no_snapshot() {
        let tmp = std::env::temp_dir().join("zeus_test_rollback_nosnapshot");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-skill")).unwrap();

        let result = restore_rollback(&tmp, "my-skill");
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
