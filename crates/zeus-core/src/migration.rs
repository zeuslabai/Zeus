use crate::Result;

/// Current config version
pub const CURRENT_VERSION: u32 = 1;

pub struct ConfigMigrator;

impl ConfigMigrator {
    /// Get current config version
    pub fn current_version() -> u32 {
        CURRENT_VERSION
    }

    /// Detect version from raw TOML content
    pub fn detect_version(raw_toml: &str) -> u32 {
        // Parse and look for config_version key
        // If missing, it's version 0
        if let Ok(table) = raw_toml.parse::<toml::Table>() {
            table
                .get("config_version")
                .and_then(|v| v.as_integer())
                .map(|v| v as u32)
                .unwrap_or(0)
        } else {
            0
        }
    }

    /// Migrate config from its current version to the latest
    pub fn migrate(raw_toml: &str) -> Result<String> {
        let mut version = Self::detect_version(raw_toml);
        let mut content = raw_toml.to_string();

        while version < CURRENT_VERSION {
            content = match version {
                0 => Self::migrate_v0_to_v1(&content)?,
                _ => break,
            };
            version += 1;
        }

        Ok(content)
    }

    /// v0 -> v1: Add config_version field and [search] section
    fn migrate_v0_to_v1(raw: &str) -> Result<String> {
        let mut content = raw.to_string();

        // Add config_version = 1 near the top
        if !content.contains("config_version") {
            content = format!("config_version = 1\n{}", content);
        }

        // Add [search] section if not present
        if !content.contains("[search]") {
            content.push_str(
                "\n\n# Web search configuration\n[search]\nprovider = \"duckduckgo\"\nmax_results = 5\n",
            );
        }

        Ok(content)
    }

    /// Check if migration is needed
    pub fn needs_migration(raw_toml: &str) -> bool {
        Self::detect_version(raw_toml) < CURRENT_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_version_missing() {
        let toml = r#"
model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
"#;
        assert_eq!(ConfigMigrator::detect_version(toml), 0);
    }

    #[test]
    fn test_detect_version_present() {
        let toml = r#"
config_version = 1
model = "anthropic/claude-sonnet-4-20250514"
"#;
        assert_eq!(ConfigMigrator::detect_version(toml), 1);
    }

    #[test]
    fn test_migrate_v0_to_v1_adds_version() {
        let toml = r#"model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
"#;
        let migrated = ConfigMigrator::migrate(toml).expect("operation should succeed");
        assert!(migrated.contains("config_version = 1"));
    }

    #[test]
    fn test_migrate_v0_to_v1_adds_search_section() {
        let toml = r#"model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
"#;
        let migrated = ConfigMigrator::migrate(toml).expect("operation should succeed");
        assert!(migrated.contains("[search]"));
        assert!(migrated.contains("provider = \"duckduckgo\""));
        assert!(migrated.contains("max_results = 5"));
    }

    #[test]
    fn test_migrate_idempotent() {
        let toml = r#"model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
"#;
        let migrated_once = ConfigMigrator::migrate(toml).expect("operation should succeed");
        let migrated_twice =
            ConfigMigrator::migrate(&migrated_once).expect("operation should succeed");
        assert_eq!(migrated_once, migrated_twice);
    }

    #[test]
    fn test_needs_migration_true_for_v0() {
        let toml = r#"
model = "anthropic/claude-sonnet-4-20250514"
"#;
        assert!(ConfigMigrator::needs_migration(toml));
    }

    #[test]
    fn test_needs_migration_false_for_current() {
        let toml = r#"
config_version = 1
model = "anthropic/claude-sonnet-4-20250514"
"#;
        assert!(!ConfigMigrator::needs_migration(toml));
    }
}
