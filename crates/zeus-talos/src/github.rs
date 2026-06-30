//! GitHub CLI tools — wraps `gh` for PRs, issues, and Actions

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

async fn run_gh(args: Vec<String>) -> Result<String> {
    let output = tokio::process::Command::new("gh")
        .args(&args)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to run gh: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Err(Error::Tool(format!(
            "gh failed ({}): {}",
            output.status.code().unwrap_or(-1),
            if stderr.is_empty() { stdout } else { stderr }
        )));
    }

    Ok(if stdout.is_empty() { stderr } else { stdout })
}

fn get_repo(args: &Value) -> Result<String> {
    args.get("repo")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::Tool("Missing 'repo' parameter (owner/repo)".to_string()))
}

// ---------------------------------------------------------------------------
// T9.1 — gh_pr_review
// ---------------------------------------------------------------------------

/// Review a GitHub pull request (approve / request-changes / comment)
pub struct GhPrReviewTool;

#[async_trait]
impl TalosTool for GhPrReviewTool {
    fn name(&self) -> &'static str {
        "gh_pr_review"
    }
    fn description(&self) -> &'static str {
        "Review a GitHub PR: approve, request changes, or leave a comment"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("repo", "string", "Repository (owner/repo)", true)
            .with_param("number", "integer", "PR number", true)
            .with_param("action", "string", "approve | request-changes | comment", true)
            .with_param("body", "string", "Review body text", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let repo = get_repo(&args)?;
        let number = args
            .get("number")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing 'number'".to_string()))?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'action'".to_string()))?;

        let num_str = number.to_string();
        let mut cmd_args = vec![
            "pr".to_string(),
            "review".to_string(),
            num_str,
            "--repo".to_string(),
            repo,
        ];

        match action {
            "approve" => cmd_args.push("--approve".to_string()),
            "request-changes" => cmd_args.push("--request-changes".to_string()),
            "comment" => cmd_args.push("--comment".to_string()),
            _ => return Err(Error::Tool(format!("Invalid action: {}", action))),
        }

        if let Some(body) = args.get("body").and_then(|v| v.as_str()) {
            cmd_args.push("--body".to_string());
            cmd_args.push(body.to_string());
        }

        run_gh(cmd_args).await
    }
}

// ---------------------------------------------------------------------------
// T9.2 — gh_pr_comment
// ---------------------------------------------------------------------------

/// Add a comment to a GitHub pull request
pub struct GhPrCommentTool;

#[async_trait]
impl TalosTool for GhPrCommentTool {
    fn name(&self) -> &'static str {
        "gh_pr_comment"
    }
    fn description(&self) -> &'static str {
        "Add a comment to a GitHub PR"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("repo", "string", "Repository (owner/repo)", true)
            .with_param("number", "integer", "PR number", true)
            .with_param("body", "string", "Comment body", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let repo = get_repo(&args)?;
        let number = args
            .get("number")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing 'number'".to_string()))?;
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'body'".to_string()))?;

        run_gh(vec![
            "pr".to_string(),
            "comment".to_string(),
            number.to_string(),
            "--repo".to_string(),
            repo,
            "--body".to_string(),
            body.to_string(),
        ])
        .await
    }
}

// ---------------------------------------------------------------------------
// T9.3 — gh_issue_create
// ---------------------------------------------------------------------------

/// Create a GitHub issue
pub struct GhIssueCreateTool;

#[async_trait]
impl TalosTool for GhIssueCreateTool {
    fn name(&self) -> &'static str {
        "gh_issue_create"
    }
    fn description(&self) -> &'static str {
        "Create a GitHub issue"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("repo", "string", "Repository (owner/repo)", true)
            .with_param("title", "string", "Issue title", true)
            .with_param("body", "string", "Issue body (markdown)", false)
            .with_param("label", "string", "Comma-separated labels", false)
            .with_param("assignee", "string", "Comma-separated assignees", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let repo = get_repo(&args)?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'title'".to_string()))?;

        let mut cmd_args = vec![
            "issue".to_string(),
            "create".to_string(),
            "--repo".to_string(),
            repo,
            "--title".to_string(),
            title.to_string(),
        ];

        if let Some(body) = args.get("body").and_then(|v| v.as_str()) {
            cmd_args.push("--body".to_string());
            cmd_args.push(body.to_string());
        }
        if let Some(labels) = args.get("label").and_then(|v| v.as_str()) {
            for label in labels.split(',') {
                cmd_args.push("--label".to_string());
                cmd_args.push(label.trim().to_string());
            }
        }
        if let Some(assignees) = args.get("assignee").and_then(|v| v.as_str()) {
            for a in assignees.split(',') {
                cmd_args.push("--assignee".to_string());
                cmd_args.push(a.trim().to_string());
            }
        }

        run_gh(cmd_args).await
    }
}

// ---------------------------------------------------------------------------
// T9.4 — gh_actions_status
// ---------------------------------------------------------------------------

/// Get GitHub Actions workflow run status
pub struct GhActionsStatusTool;

#[async_trait]
impl TalosTool for GhActionsStatusTool {
    fn name(&self) -> &'static str {
        "gh_actions_status"
    }
    fn description(&self) -> &'static str {
        "Get recent GitHub Actions workflow run status for a repo"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("repo", "string", "Repository (owner/repo)", true)
            .with_param("limit", "integer", "Number of runs to show (default 5)", false)
            .with_param("workflow", "string", "Filter by workflow file name", false)
            .with_param("branch", "string", "Filter by branch", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let repo = get_repo(&args)?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5);

        let mut cmd_args = vec![
            "run".to_string(),
            "list".to_string(),
            "--repo".to_string(),
            repo,
            "--limit".to_string(),
            limit.to_string(),
        ];

        if let Some(wf) = args.get("workflow").and_then(|v| v.as_str()) {
            cmd_args.push("--workflow".to_string());
            cmd_args.push(wf.to_string());
        }
        if let Some(branch) = args.get("branch").and_then(|v| v.as_str()) {
            cmd_args.push("--branch".to_string());
            cmd_args.push(branch.to_string());
        }

        run_gh(cmd_args).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gh_pr_review_schema() {
        let tool = GhPrReviewTool;
        assert_eq!(tool.name(), "gh_pr_review");
        let schema = tool.schema();
        let params = schema.parameters.as_object().unwrap();
        let props = params["properties"].as_object().unwrap();
        assert!(props.contains_key("repo"));
        assert!(props.contains_key("number"));
        assert!(props.contains_key("action"));
        assert!(props.contains_key("body"));
    }

    #[test]
    fn test_gh_pr_comment_schema() {
        let tool = GhPrCommentTool;
        assert_eq!(tool.name(), "gh_pr_comment");
        let schema = tool.schema();
        let params = schema.parameters.as_object().unwrap();
        assert!(params["required"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn test_gh_issue_create_schema() {
        let tool = GhIssueCreateTool;
        assert_eq!(tool.name(), "gh_issue_create");
        let schema = tool.schema();
        let params = schema.parameters.as_object().unwrap();
        let props = params["properties"].as_object().unwrap();
        assert!(props.contains_key("title"));
        assert!(props.contains_key("label"));
        assert!(props.contains_key("assignee"));
    }

    #[test]
    fn test_gh_actions_status_schema() {
        let tool = GhActionsStatusTool;
        assert_eq!(tool.name(), "gh_actions_status");
        let schema = tool.schema();
        let params = schema.parameters.as_object().unwrap();
        let props = params["properties"].as_object().unwrap();
        assert!(props.contains_key("limit"));
        assert!(props.contains_key("workflow"));
        assert!(props.contains_key("branch"));
    }
}
