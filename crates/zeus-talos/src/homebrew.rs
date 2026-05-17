//! Homebrew package manager tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// Install a Homebrew package
pub struct BrewInstallTool;

#[async_trait]
impl TalosTool for BrewInstallTool {
    fn name(&self) -> &'static str {
        "brew_install"
    }
    fn description(&self) -> &'static str {
        "Install a package using Homebrew"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("package", "string", "Package name to install", true)
            .with_param(
                "cask",
                "boolean",
                "Install as a cask (default false)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let package = args
            .get("package")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing package".to_string()))?;

        let cask = args.get("cask").and_then(|v| v.as_bool()).unwrap_or(false);

        let sanitized = crate::sanitize_shell_arg(package);

        let mut cmd = tokio::process::Command::new("brew");
        cmd.arg("install");
        if cask {
            cmd.arg("--cask");
        }
        cmd.arg(&sanitized);

        let output = cmd.output().await.map_err(|e| {
            Error::Tool(format!(
                "Failed to run brew (is Homebrew installed?): {}",
                e
            ))
        })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(format!(
                "Installed {}{}\n{}",
                package,
                if cask { " (cask)" } else { "" },
                stdout
            ))
        } else {
            Err(Error::Tool(format!(
                "brew install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// List installed Homebrew packages
pub struct BrewListTool;

#[async_trait]
impl TalosTool for BrewListTool {
    fn name(&self) -> &'static str {
        "brew_list"
    }
    fn description(&self) -> &'static str {
        "List installed Homebrew packages"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "cask",
            "boolean",
            "List casks only (default false)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let cask = args.get("cask").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut cmd = tokio::process::Command::new("brew");
        cmd.arg("list");
        if cask {
            cmd.arg("--cask");
        }

        let output = cmd.output().await.map_err(|e| {
            Error::Tool(format!(
                "Failed to run brew (is Homebrew installed?): {}",
                e
            ))
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "brew list failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// Search Homebrew packages
pub struct BrewSearchTool;

#[async_trait]
impl TalosTool for BrewSearchTool {
    fn name(&self) -> &'static str {
        "brew_search"
    }
    fn description(&self) -> &'static str {
        "Search for Homebrew packages"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "query",
            "string",
            "Search query",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

        let sanitized = crate::sanitize_shell_arg(query);

        let output = tokio::process::Command::new("brew")
            .args(["search", &sanitized])
            .output()
            .await
            .map_err(|e| {
                Error::Tool(format!(
                    "Failed to run brew (is Homebrew installed?): {}",
                    e
                ))
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "brew search failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// Uninstall a Homebrew package
pub struct BrewUninstallTool;

#[async_trait]
impl TalosTool for BrewUninstallTool {
    fn name(&self) -> &'static str {
        "brew_uninstall"
    }
    fn description(&self) -> &'static str {
        "Uninstall a Homebrew package"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("package", "string", "Package name to uninstall", true)
            .with_param("cask", "boolean", "Uninstall a cask (default false)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let package = args
            .get("package")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing package".to_string()))?;

        let cask = args.get("cask").and_then(|v| v.as_bool()).unwrap_or(false);

        let sanitized = crate::sanitize_shell_arg(package);

        let mut cmd = tokio::process::Command::new("brew");
        cmd.arg("uninstall");
        if cask {
            cmd.arg("--cask");
        }
        cmd.arg(&sanitized);

        let output = cmd.output().await.map_err(|e| {
            Error::Tool(format!(
                "Failed to run brew (is Homebrew installed?): {}",
                e
            ))
        })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(format!(
                "Uninstalled {}{}\n{}",
                package,
                if cask { " (cask)" } else { "" },
                stdout
            ))
        } else {
            Err(Error::Tool(format!(
                "brew uninstall failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brew_install_schema() {
        let tool = BrewInstallTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "brew_install");
        // package is required
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("package")));
        // cask is optional (not in required)
        assert!(!required.iter().any(|v| v.as_str() == Some("cask")));
    }

    #[test]
    fn test_brew_list_schema() {
        let tool = BrewListTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "brew_list");
        // cask is optional
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!required.iter().any(|v| v.as_str() == Some("cask")));
    }

    #[test]
    fn test_brew_search_schema() {
        let tool = BrewSearchTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "brew_search");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn test_brew_uninstall_schema() {
        let tool = BrewUninstallTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "brew_uninstall");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("package")));
        assert!(!required.iter().any(|v| v.as_str() == Some("cask")));
    }
}
