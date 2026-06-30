//! Tmux tools — shell-based wrappers around the `tmux` CLI.
//!
//! Provides:
//! - `tmux_list`     — list sessions / windows / panes
//! - `tmux_send`     — send keys/text to a target pane
//! - `tmux_capture`  — capture visible pane contents
//! - `tmux_new`      — create a new detached session
//! - `tmux_kill`     — kill a session
//!
//! Args are passed directly to `tokio::process::Command` (no shell interpretation),
//! so injection is structurally impossible — but we still validate identifiers
//! to fail loudly on obviously bad input.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

const MAX_OUTPUT_BYTES: usize = 256 * 1024; // 256 KB cap on capture output

/// Validate session / window / pane / command names.
/// Allowed: alphanumerics, `_`, `-`, `.`, `:`, `/`, `@`, `%`. (`:`/`@`/`%` are needed
/// for tmux target syntax like `session:0.1`, `@1`, `%2`.)
fn validate_target(s: &str, field: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::Tool(format!("{} must not be empty", field)));
    }
    if s.len() > 256 {
        return Err(Error::Tool(format!("{} too long (>256 chars)", field)));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/' | '@' | '%'))
    {
        return Err(Error::Tool(format!(
            "{} contains invalid characters (allowed: a-z A-Z 0-9 _ - . : / @ %)",
            field
        )));
    }
    Ok(())
}

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s;
    }
    let mut out = s.into_bytes();
    out.truncate(MAX_OUTPUT_BYTES);
    let mut s = String::from_utf8_lossy(&out).into_owned();
    s.push_str("\n... [truncated]");
    s
}

async fn run_tmux(args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("tmux")
        .args(args)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to run tmux (is tmux installed?): {}", e)))?;

    if output.status.success() {
        Ok(truncate_output(
            String::from_utf8_lossy(&output.stdout).to_string(),
        ))
    } else {
        Err(Error::Tool(format!(
            "tmux {} failed: {}",
            args.first().copied().unwrap_or("?"),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

// ---------- tmux_list ----------

/// List tmux sessions, windows, or panes.
pub struct TmuxListTool;

#[async_trait]
impl TalosTool for TmuxListTool {
    fn name(&self) -> &'static str {
        "tmux_list"
    }
    fn description(&self) -> &'static str {
        "List tmux sessions (default), windows, or panes. scope: 'sessions' | 'windows' | 'panes'"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "scope",
                "string",
                "What to list: 'sessions' (default), 'windows', or 'panes'",
                false,
            )
            .with_param(
                "target",
                "string",
                "Optional target session/window when scope is 'windows' or 'panes'",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let scope = args
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("sessions");
        let target = args.get("target").and_then(|v| v.as_str());

        let subcmd = match scope {
            "sessions" => "list-sessions",
            "windows" => "list-windows",
            "panes" => "list-panes",
            other => {
                return Err(Error::Tool(format!(
                    "scope must be sessions|windows|panes (got '{}')",
                    other
                )))
            }
        };

        let mut argv: Vec<&str> = vec![subcmd];
        if let Some(t) = target {
            validate_target(t, "target")?;
            argv.push("-t");
            argv.push(t);
        }
        // tmux exits non-zero with "no server running" when nothing is up — surface as empty.
        match run_tmux(&argv).await {
            Ok(s) => Ok(s),
            Err(Error::Tool(msg)) if msg.contains("no server running") => Ok(String::new()),
            Err(e) => Err(e),
        }
    }
}

// ---------- tmux_send ----------

/// Send keys (text + optional Enter) to a tmux pane.
pub struct TmuxSendTool;

#[async_trait]
impl TalosTool for TmuxSendTool {
    fn name(&self) -> &'static str {
        "tmux_send"
    }
    fn description(&self) -> &'static str {
        "Send keys/text to a tmux target pane (e.g. 'mysession:0.0'). enter=true appends Enter."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "target",
                "string",
                "Target pane (e.g. 'session', 'session:window', 'session:window.pane')",
                true,
            )
            .with_param("keys", "string", "Text or key sequence to send", true)
            .with_param(
                "enter",
                "boolean",
                "Append Enter after sending (default true)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("target required".to_string()))?;
        let keys = args
            .get("keys")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("keys required".to_string()))?;
        let enter = args.get("enter").and_then(|v| v.as_bool()).unwrap_or(true);
        validate_target(target, "target")?;
        // keys may contain arbitrary text — tmux treats it as a literal arg, not a shell string,
        // so we don't restrict its contents. We do cap length to avoid pathological input.
        if keys.len() > 64 * 1024 {
            return Err(Error::Tool("keys too long (>64KB)".to_string()));
        }

        let mut argv: Vec<&str> = vec!["send-keys", "-t", target, keys];
        if enter {
            argv.push("Enter");
        }
        run_tmux(&argv).await?;
        Ok(format!("sent {} bytes to {}", keys.len(), target))
    }
}

// ---------- tmux_capture ----------

/// Capture the visible contents of a tmux pane.
pub struct TmuxCaptureTool;

#[async_trait]
impl TalosTool for TmuxCaptureTool {
    fn name(&self) -> &'static str {
        "tmux_capture"
    }
    fn description(&self) -> &'static str {
        "Capture the contents of a tmux pane. Set history=true to include scrollback."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("target", "string", "Target pane", true)
            .with_param(
                "history",
                "boolean",
                "Include full scrollback history (default false)",
                false,
            )
            .with_param(
                "lines",
                "integer",
                "Number of trailing lines to keep (default 200, max 5000)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("target required".to_string()))?;
        let history = args.get("history").and_then(|v| v.as_bool()).unwrap_or(false);
        let lines = args
            .get("lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .min(5000) as usize;
        validate_target(target, "target")?;

        // -p prints to stdout; -J joins wrapped lines; -S - means start of history.
        let mut argv: Vec<&str> = vec!["capture-pane", "-p", "-J", "-t", target];
        if history {
            argv.push("-S");
            argv.push("-");
        }
        let out = run_tmux(&argv).await?;
        // Trim to the last `lines` lines.
        let kept: Vec<&str> = out.lines().rev().take(lines).collect();
        let mut result: Vec<&str> = kept.into_iter().rev().collect();
        // If we truncated, prepend a marker.
        let total_lines = out.lines().count();
        if total_lines > result.len() {
            result.insert(0, "... [earlier lines trimmed]");
        }
        Ok(result.join("\n"))
    }
}

// ---------- tmux_new ----------

/// Create a new detached tmux session.
pub struct TmuxNewTool;

#[async_trait]
impl TalosTool for TmuxNewTool {
    fn name(&self) -> &'static str {
        "tmux_new"
    }
    fn description(&self) -> &'static str {
        "Create a new detached tmux session. Optionally run an initial command in it."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Session name", true)
            .with_param(
                "command",
                "string",
                "Optional command to run in the session shell",
                false,
            )
            .with_param(
                "cwd",
                "string",
                "Optional working directory for the session",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("name required".to_string()))?;
        validate_target(name, "name")?;
        let command = args.get("command").and_then(|v| v.as_str());
        let cwd = args.get("cwd").and_then(|v| v.as_str());

        let mut argv: Vec<String> = vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-s".to_string(),
            name.to_string(),
        ];
        if let Some(c) = cwd {
            // cwd: allow more chars than identifiers (paths can contain spaces...)
            // but reject shell metacharacters that have special meaning unquoted.
            if c.chars().any(|ch| matches!(ch, ';' | '|' | '&' | '`' | '$' | '\n')) {
                return Err(Error::Tool(
                    "cwd contains forbidden characters".to_string(),
                ));
            }
            argv.push("-c".to_string());
            argv.push(c.to_string());
        }
        if let Some(cmd) = command {
            // tmux runs this via its shell; pass as a single arg.
            argv.push(cmd.to_string());
        }

        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        run_tmux(&argv_refs).await?;
        Ok(format!("created session '{}'", name))
    }
}

// ---------- tmux_kill ----------

/// Kill a tmux session (or the entire server).
pub struct TmuxKillTool;

#[async_trait]
impl TalosTool for TmuxKillTool {
    fn name(&self) -> &'static str {
        "tmux_kill"
    }
    fn description(&self) -> &'static str {
        "Kill a tmux session by name, or the entire server with all=true."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Session name to kill", false)
            .with_param(
                "all",
                "boolean",
                "Kill the entire tmux server (default false)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        if all {
            run_tmux(&["kill-server"]).await?;
            return Ok("tmux server killed".to_string());
        }
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("either name or all=true required".to_string()))?;
        validate_target(name, "name")?;
        run_tmux(&["kill-session", "-t", name]).await?;
        Ok(format!("killed session '{}'", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_targets() {
        assert!(validate_target("mysession", "x").is_ok());
        assert!(validate_target("session:0.1", "x").is_ok());
        assert!(validate_target("@1", "x").is_ok());
        assert!(validate_target("%2", "x").is_ok());
        assert!(validate_target("bad name", "x").is_err());
        assert!(validate_target("", "x").is_err());
        assert!(validate_target("a;rm -rf /", "x").is_err());
    }

    #[test]
    fn list_schema_has_optional_params() {
        let s = TmuxListTool.schema();
        let req = s
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        assert!(req.is_empty());
    }

    #[test]
    fn send_requires_target_and_keys() {
        let s = TmuxSendTool.schema();
        let req = s
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        let names: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"target"));
        assert!(names.contains(&"keys"));
    }

    #[test]
    fn list_rejects_unknown_scope() {
        let t = TmuxListTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({"scope": "everything"})));
        assert!(res.is_err());
    }

    #[test]
    fn send_rejects_bad_target() {
        let t = TmuxSendTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({
            "target": "; rm -rf /",
            "keys": "ls",
        })));
        assert!(res.is_err());
    }

    #[test]
    fn new_rejects_cwd_with_metachars() {
        let t = TmuxNewTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({
            "name": "test",
            "cwd": "/tmp; rm -rf /",
        })));
        assert!(res.is_err());
    }

    #[test]
    fn kill_requires_name_or_all() {
        let t = TmuxKillTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({})));
        assert!(res.is_err());
    }
}
