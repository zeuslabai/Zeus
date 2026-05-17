//! Git CLI tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

// ---------------------------------------------------------------------------
// 1. GitStatusTool
// ---------------------------------------------------------------------------

/// Show git working tree status
pub struct GitStatusTool;

#[async_trait]
impl TalosTool for GitStatusTool {
    fn name(&self) -> &'static str {
        "git_status"
    }
    fn description(&self) -> &'static str {
        "Show git working tree status"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Repository path (default: current directory)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("status").arg("--porcelain");

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.is_empty() {
                Ok("Working tree clean".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 2. GitAddTool
// ---------------------------------------------------------------------------

/// Stage files for commit
pub struct GitAddTool;

#[async_trait]
impl TalosTool for GitAddTool {
    fn name(&self) -> &'static str {
        "git_add"
    }
    fn description(&self) -> &'static str {
        "Stage files for commit"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "files",
                "string",
                "Files to stage (space-separated, or \".\" for all)",
                true,
            )
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let files = args
            .get("files")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: files".to_string()))?;

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("add");

        for file in files.split_whitespace() {
            cmd.arg(file);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(format!("Staged: {}", files))
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 3. GitCommitTool
// ---------------------------------------------------------------------------

/// Create a git commit
pub struct GitCommitTool;

#[async_trait]
impl TalosTool for GitCommitTool {
    fn name(&self) -> &'static str {
        "git_commit"
    }
    fn description(&self) -> &'static str {
        "Create a git commit"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("message", "string", "Commit message", true)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: message".to_string()))?;

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("commit").arg("-m").arg(message);

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 4. GitPushTool
// ---------------------------------------------------------------------------

/// Push commits to remote
pub struct GitPushTool;

#[async_trait]
impl TalosTool for GitPushTool {
    fn name(&self) -> &'static str {
        "git_push"
    }
    fn description(&self) -> &'static str {
        "Push commits to remote repository"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("remote", "string", "Remote name (default: origin)", false)
            .with_param("branch", "string", "Branch name", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let remote = args
            .get("remote")
            .and_then(|v| v.as_str())
            .unwrap_or("origin");

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("push").arg(remote);

        if let Some(branch) = args.get("branch").and_then(|v| v.as_str()) {
            cmd.arg(branch);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // git push often writes progress to stderr even on success
            Ok(if stdout.is_empty() { stderr } else { stdout })
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 5. GitPullTool
// ---------------------------------------------------------------------------

/// Pull changes from remote
pub struct GitPullTool;

#[async_trait]
impl TalosTool for GitPullTool {
    fn name(&self) -> &'static str {
        "git_pull"
    }
    fn description(&self) -> &'static str {
        "Pull changes from remote repository"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("remote", "string", "Remote name (default: origin)", false)
            .with_param("branch", "string", "Branch name", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let remote = args
            .get("remote")
            .and_then(|v| v.as_str())
            .unwrap_or("origin");

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("pull").arg(remote);

        if let Some(branch) = args.get("branch").and_then(|v| v.as_str()) {
            cmd.arg(branch);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 6. GitDiffTool
// ---------------------------------------------------------------------------

/// Show git diff
pub struct GitDiffTool;

#[async_trait]
impl TalosTool for GitDiffTool {
    fn name(&self) -> &'static str {
        "git_diff"
    }
    fn description(&self) -> &'static str {
        "Show changes in the working tree"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("staged", "boolean", "Show staged changes only", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let staged = args
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("diff");

        if staged {
            cmd.arg("--staged");
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.is_empty() {
                Ok("No changes".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 7. GitDiffStatTool
// ---------------------------------------------------------------------------

/// Show git diff statistics
pub struct GitDiffStatTool;

#[async_trait]
impl TalosTool for GitDiffStatTool {
    fn name(&self) -> &'static str {
        "git_diff_stat"
    }
    fn description(&self) -> &'static str {
        "Show diff statistics (files changed, insertions, deletions)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("staged", "boolean", "Show staged changes only", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let staged = args
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("diff");

        if staged {
            cmd.arg("--staged");
        }

        cmd.arg("--stat");

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.is_empty() {
                Ok("No changes".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 8. GitLogTool
// ---------------------------------------------------------------------------

/// Show commit history
pub struct GitLogTool;

#[async_trait]
impl TalosTool for GitLogTool {
    fn name(&self) -> &'static str {
        "git_log"
    }
    fn description(&self) -> &'static str {
        "Show git commit history"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "count",
                "integer",
                "Number of commits to show (default: 10)",
                false,
            )
            .with_param(
                "oneline",
                "boolean",
                "Use one-line format (default: true)",
                false,
            )
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10);

        let oneline = args
            .get("oneline")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("log").arg(format!("-{}", count));

        if oneline {
            cmd.arg("--oneline");
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.is_empty() {
                Ok("No commits".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 9. GitBranchListTool
// ---------------------------------------------------------------------------

/// List git branches
pub struct GitBranchListTool;

#[async_trait]
impl TalosTool for GitBranchListTool {
    fn name(&self) -> &'static str {
        "git_branch_list"
    }
    fn description(&self) -> &'static str {
        "List git branches"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("all", "boolean", "Include remote branches", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("branch");

        if all {
            cmd.arg("-a");
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.is_empty() {
                Ok("No branches".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 10. GitBranchCreateTool
// ---------------------------------------------------------------------------

/// Create a new git branch
pub struct GitBranchCreateTool;

#[async_trait]
impl TalosTool for GitBranchCreateTool {
    fn name(&self) -> &'static str {
        "git_branch_create"
    }
    fn description(&self) -> &'static str {
        "Create a new git branch"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Branch name", true)
            .with_param(
                "checkout",
                "boolean",
                "Switch to new branch after creation (default: false)",
                false,
            )
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: name".to_string()))?;

        let checkout = args
            .get("checkout")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = tokio::process::Command::new("git");

        if checkout {
            cmd.arg("checkout").arg("-b").arg(name);
        } else {
            cmd.arg("branch").arg(name);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            if checkout {
                Ok(format!("Created and switched to branch '{}'", name))
            } else {
                Ok(format!("Created branch '{}'", name))
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 11. GitBranchDeleteTool
// ---------------------------------------------------------------------------

/// Delete a git branch
pub struct GitBranchDeleteTool;

#[async_trait]
impl TalosTool for GitBranchDeleteTool {
    fn name(&self) -> &'static str {
        "git_branch_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a git branch"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Branch name to delete", true)
            .with_param(
                "force",
                "boolean",
                "Force delete even if not merged (default: false)",
                false,
            )
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: name".to_string()))?;

        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("branch");

        if force {
            cmd.arg("-D");
        } else {
            cmd.arg("-d");
        }

        cmd.arg(name);

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(format!("Deleted branch '{}'", name))
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 12. GitCheckoutTool
// ---------------------------------------------------------------------------

/// Switch branches or restore files
pub struct GitCheckoutTool;

#[async_trait]
impl TalosTool for GitCheckoutTool {
    fn name(&self) -> &'static str {
        "git_checkout"
    }
    fn description(&self) -> &'static str {
        "Switch branches or restore working tree files"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "target",
                "string",
                "Branch name or file path to checkout",
                true,
            )
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: target".to_string()))?;

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("checkout").arg(target);

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // git checkout writes to stderr on success (e.g. "Switched to branch ...")
            if stderr.is_empty() {
                Ok(format!("Checked out '{}'", target))
            } else {
                Ok(stderr)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 13. GitCloneTool
// ---------------------------------------------------------------------------

/// Clone a git repository
pub struct GitCloneTool;

#[async_trait]
impl TalosTool for GitCloneTool {
    fn name(&self) -> &'static str {
        "git_clone"
    }
    fn description(&self) -> &'static str {
        "Clone a git repository"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("url", "string", "Repository URL to clone", true)
            .with_param("destination", "string", "Destination directory", false)
            .with_param(
                "path",
                "string",
                "Working directory for clone command",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: url".to_string()))?;

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("clone").arg(url);

        if let Some(dest) = args.get("destination").and_then(|v| v.as_str()) {
            cmd.arg(dest);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // git clone writes progress to stderr
            if stderr.is_empty() {
                Ok(format!("Cloned {}", url))
            } else {
                Ok(stderr)
            }
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 14. GitStashTool
// ---------------------------------------------------------------------------

/// Stash changes
pub struct GitStashTool;

#[async_trait]
impl TalosTool for GitStashTool {
    fn name(&self) -> &'static str {
        "git_stash"
    }
    fn description(&self) -> &'static str {
        "Stash changes in the working directory"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("message", "string", "Stash message", false)
            .with_param(
                "path",
                "string",
                "Repository path (default: current directory)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("stash");

        if let Some(message) = args.get("message").and_then(|v| v.as_str()) {
            cmd.arg("push").arg("-m").arg(message);
        }

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 15. GitStashPopTool
// ---------------------------------------------------------------------------

/// Pop stashed changes
pub struct GitStashPopTool;

#[async_trait]
impl TalosTool for GitStashPopTool {
    fn name(&self) -> &'static str {
        "git_stash_pop"
    }
    fn description(&self) -> &'static str {
        "Pop the most recent stashed changes"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Repository path (default: current directory)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("stash").arg("pop");

        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.current_dir(path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run git: {}", e)))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "git error: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: check that a parameter name is listed in the "required" array of the schema
    fn is_required(schema: &ToolSchema, param_name: &str) -> bool {
        schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().any(|v| v.as_str() == Some(param_name)))
            .unwrap_or(false)
    }

    /// Helper: check that a parameter exists in the schema properties
    fn has_param(schema: &ToolSchema, param_name: &str) -> bool {
        schema
            .parameters
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| props.contains_key(param_name))
            .unwrap_or(false)
    }

    // --- Schema validation tests ---

    #[test]
    fn test_git_status_schema() {
        let tool = GitStatusTool;
        assert_eq!(tool.name(), "git_status");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_status");
        assert!(has_param(&schema, "path"));
    }

    #[test]
    fn test_git_add_schema() {
        let tool = GitAddTool;
        assert_eq!(tool.name(), "git_add");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_add");
        assert!(has_param(&schema, "files"));
        assert!(is_required(&schema, "files"));
    }

    #[test]
    fn test_git_commit_schema() {
        let tool = GitCommitTool;
        assert_eq!(tool.name(), "git_commit");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_commit");
        assert!(has_param(&schema, "message"));
        assert!(is_required(&schema, "message"));
    }

    #[test]
    fn test_git_push_schema() {
        let tool = GitPushTool;
        assert_eq!(tool.name(), "git_push");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_push");
        assert!(has_param(&schema, "remote"));
        assert!(has_param(&schema, "branch"));
    }

    #[test]
    fn test_git_pull_schema() {
        let tool = GitPullTool;
        assert_eq!(tool.name(), "git_pull");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_pull");
        assert!(has_param(&schema, "remote"));
        assert!(has_param(&schema, "branch"));
    }

    #[test]
    fn test_git_diff_schema() {
        let tool = GitDiffTool;
        assert_eq!(tool.name(), "git_diff");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_diff");
        assert!(has_param(&schema, "staged"));
    }

    #[test]
    fn test_git_diff_stat_schema() {
        let tool = GitDiffStatTool;
        assert_eq!(tool.name(), "git_diff_stat");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_diff_stat");
        assert!(has_param(&schema, "staged"));
    }

    #[test]
    fn test_git_log_schema() {
        let tool = GitLogTool;
        assert_eq!(tool.name(), "git_log");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_log");
        assert!(has_param(&schema, "count"));
        assert!(has_param(&schema, "oneline"));
    }

    #[test]
    fn test_git_branch_list_schema() {
        let tool = GitBranchListTool;
        assert_eq!(tool.name(), "git_branch_list");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_branch_list");
        assert!(has_param(&schema, "all"));
    }

    #[test]
    fn test_git_branch_create_schema() {
        let tool = GitBranchCreateTool;
        assert_eq!(tool.name(), "git_branch_create");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_branch_create");
        assert!(has_param(&schema, "name"));
        assert!(is_required(&schema, "name"));
        assert!(has_param(&schema, "checkout"));
    }

    #[test]
    fn test_git_branch_delete_schema() {
        let tool = GitBranchDeleteTool;
        assert_eq!(tool.name(), "git_branch_delete");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_branch_delete");
        assert!(has_param(&schema, "name"));
        assert!(is_required(&schema, "name"));
        assert!(has_param(&schema, "force"));
    }

    #[test]
    fn test_git_checkout_schema() {
        let tool = GitCheckoutTool;
        assert_eq!(tool.name(), "git_checkout");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_checkout");
        assert!(has_param(&schema, "target"));
        assert!(is_required(&schema, "target"));
    }

    #[test]
    fn test_git_clone_schema() {
        let tool = GitCloneTool;
        assert_eq!(tool.name(), "git_clone");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_clone");
        assert!(has_param(&schema, "url"));
        assert!(is_required(&schema, "url"));
        assert!(has_param(&schema, "destination"));
    }

    #[test]
    fn test_git_stash_schema() {
        let tool = GitStashTool;
        assert_eq!(tool.name(), "git_stash");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_stash");
        assert!(has_param(&schema, "message"));
    }

    #[test]
    fn test_git_stash_pop_schema() {
        let tool = GitStashPopTool;
        assert_eq!(tool.name(), "git_stash_pop");
        let schema = tool.schema();
        assert_eq!(schema.name, "git_stash_pop");
        assert!(has_param(&schema, "path"));
    }

    // --- Integration tests ---

    #[tokio::test]
    async fn test_git_status_outside_repo() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let tool = GitStatusTool;
        let result = tool
            .execute(json!({ "path": dir.path().to_str().expect("SQL should execute") }))
            .await;
        assert!(
            result.is_err(),
            "Expected error when running git status outside a repo"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("git error") || err_msg.contains("not a git repository"),
            "Error should mention git: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_git_clone_and_status() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let repo_path = dir.path().join("test-repo");

        // Initialize a bare repo to clone from
        let init_output = tokio::process::Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(repo_path.to_str().expect("Command::new should succeed"))
            .output()
            .await
            .expect("Failed to init test repo");
        assert!(init_output.status.success(), "git init --bare failed");

        // Clone it
        let clone_tool = GitCloneTool;
        let clone_dest = dir.path().join("cloned");
        let result = clone_tool
            .execute(json!({
                "url": repo_path.to_str().expect("to_str should succeed"),
                "destination": clone_dest.to_str().expect("to_str should succeed"),
            }))
            .await;
        assert!(result.is_ok(), "git clone failed: {:?}", result.err());

        // Run status in the cloned repo
        let status_tool = GitStatusTool;
        let result = status_tool
            .execute(json!({
                "path": clone_dest.to_str().expect("to_str should succeed"),
            }))
            .await;
        assert!(result.is_ok(), "git status failed: {:?}", result.err());
        assert_eq!(
            result.expect("operation should succeed"),
            "Working tree clean"
        );
    }
}
