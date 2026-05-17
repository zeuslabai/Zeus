//! Docker tools — shell-based wrappers around the `docker` CLI.
//!
//! Provides:
//! - `docker_ps`     — list containers
//! - `docker_exec`   — exec a command inside a running container
//! - `docker_logs`   — fetch container logs
//! - `docker_start`  — start one or more containers
//! - `docker_stop`   — stop one or more containers
//! - `docker_compose` — run a docker compose subcommand
//!
//! Args are passed directly to `tokio::process::Command` (no shell interpretation),
//! so injection is structurally impossible — but we still validate identifiers
//! to fail loudly on obviously bad input.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

const MAX_OUTPUT_BYTES: usize = 256 * 1024; // 256 KB cap for logs/exec output

/// Validate that a string looks like a docker container/image/service name or ID.
/// Allowed: alphanumerics, `_`, `-`, `.`, `:`, `/`. No spaces, no shell metachars.
fn validate_identifier(s: &str, field: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::Tool(format!("{} must not be empty", field)));
    }
    if s.len() > 256 {
        return Err(Error::Tool(format!("{} too long (>256 chars)", field)));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'))
    {
        return Err(Error::Tool(format!(
            "{} contains invalid characters (allowed: a-z A-Z 0-9 _ - . : /)",
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

async fn run_docker(args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("docker")
        .args(args)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to run docker (is Docker installed?): {}", e)))?;

    if output.status.success() {
        Ok(truncate_output(
            String::from_utf8_lossy(&output.stdout).to_string(),
        ))
    } else {
        Err(Error::Tool(format!(
            "docker {} failed: {}",
            args.first().copied().unwrap_or("?"),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

// ---------- docker_ps ----------

/// List Docker containers.
pub struct DockerPsTool;

#[async_trait]
impl TalosTool for DockerPsTool {
    fn name(&self) -> &'static str {
        "docker_ps"
    }
    fn description(&self) -> &'static str {
        "List Docker containers (running by default, all with all=true)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("all", "boolean", "Show all containers (default false)", false)
            .with_param(
                "filter",
                "string",
                "Optional filter expression, e.g. 'status=exited'",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let filter = args.get("filter").and_then(|v| v.as_str());

        let mut argv: Vec<&str> = vec!["ps", "--format", "table {{.ID}}\t{{.Image}}\t{{.Status}}\t{{.Names}}\t{{.Ports}}"];
        if all {
            argv.push("-a");
        }
        if let Some(f) = filter {
            // Filters are key=value; permit '=' here in addition to ident chars.
            if !f
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/' | '='))
            {
                return Err(Error::Tool(
                    "filter contains invalid characters".to_string(),
                ));
            }
            argv.push("--filter");
            argv.push(f);
        }
        run_docker(&argv).await
    }
}

// ---------- docker_exec ----------

/// Execute a command inside a running container.
pub struct DockerExecTool;

#[async_trait]
impl TalosTool for DockerExecTool {
    fn name(&self) -> &'static str {
        "docker_exec"
    }
    fn description(&self) -> &'static str {
        "Run a command inside a running container (non-interactive)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("container", "string", "Container name or ID", true)
            .with_param(
                "command",
                "array",
                "Command and arguments, e.g. [\"ls\", \"-la\", \"/app\"]",
                true,
            )
            .with_param(
                "user",
                "string",
                "Optional user to run as (e.g. 'root' or 'uid:gid')",
                false,
            )
            .with_param(
                "workdir",
                "string",
                "Optional working directory inside container",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let container = args
            .get("container")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing container".to_string()))?;
        validate_identifier(container, "container")?;

        let command_arr = args
            .get("command")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing command array".to_string()))?;
        if command_arr.is_empty() {
            return Err(Error::Tool("command array must not be empty".to_string()));
        }
        let command_strs: Vec<String> = command_arr
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| Error::Tool("command entries must be strings".to_string()))
                    .map(|s| s.to_string())
            })
            .collect::<Result<Vec<_>>>()?;

        let mut argv: Vec<String> = vec!["exec".to_string()];
        if let Some(u) = args.get("user").and_then(|v| v.as_str()) {
            validate_identifier(u, "user")?;
            argv.push("--user".to_string());
            argv.push(u.to_string());
        }
        if let Some(w) = args.get("workdir").and_then(|v| v.as_str()) {
            argv.push("--workdir".to_string());
            argv.push(w.to_string());
        }
        argv.push(container.to_string());
        argv.extend(command_strs);

        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        run_docker(&argv_refs).await
    }
}

// ---------- docker_logs ----------

/// Fetch logs for a container.
pub struct DockerLogsTool;

#[async_trait]
impl TalosTool for DockerLogsTool {
    fn name(&self) -> &'static str {
        "docker_logs"
    }
    fn description(&self) -> &'static str {
        "Fetch logs from a Docker container"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("container", "string", "Container name or ID", true)
            .with_param(
                "tail",
                "integer",
                "Number of recent lines to return (default 100, max 5000)",
                false,
            )
            .with_param(
                "since",
                "string",
                "Show logs since timestamp or relative (e.g. '10m', '2024-01-01T00:00:00Z')",
                false,
            )
            .with_param("timestamps", "boolean", "Prepend timestamps", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let container = args
            .get("container")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing container".to_string()))?;
        validate_identifier(container, "container")?;

        let tail = args
            .get("tail")
            .and_then(|v| v.as_i64())
            .unwrap_or(100)
            .clamp(1, 5000);
        let timestamps = args
            .get("timestamps")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let tail_str = tail.to_string();
        let mut argv: Vec<&str> = vec!["logs", "--tail", &tail_str];
        if timestamps {
            argv.push("--timestamps");
        }
        let since_owned;
        if let Some(s) = args.get("since").and_then(|v| v.as_str()) {
            // since accepts duration like "10m" or RFC3339 — restrict to printable ascii.
            if !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '.' | 'T' | 'Z' | '+'))
            {
                return Err(Error::Tool(
                    "since contains invalid characters".to_string(),
                ));
            }
            since_owned = s.to_string();
            argv.push("--since");
            argv.push(&since_owned);
        }
        argv.push(container);
        run_docker(&argv).await
    }
}

// ---------- docker_start ----------

/// Start one or more containers.
pub struct DockerStartTool;

#[async_trait]
impl TalosTool for DockerStartTool {
    fn name(&self) -> &'static str {
        "docker_start"
    }
    fn description(&self) -> &'static str {
        "Start one or more stopped Docker containers"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "containers",
            "array",
            "Container names or IDs to start",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let arr = args
            .get("containers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing containers array".to_string()))?;
        if arr.is_empty() {
            return Err(Error::Tool("containers array must not be empty".to_string()));
        }
        let names: Vec<String> = arr
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| Error::Tool("containers entries must be strings".to_string()))?;
                validate_identifier(s, "container")?;
                Ok(s.to_string())
            })
            .collect::<Result<Vec<_>>>()?;

        let mut argv: Vec<&str> = vec!["start"];
        for n in &names {
            argv.push(n);
        }
        run_docker(&argv).await
    }
}

// ---------- docker_stop ----------

/// Stop one or more containers.
pub struct DockerStopTool;

#[async_trait]
impl TalosTool for DockerStopTool {
    fn name(&self) -> &'static str {
        "docker_stop"
    }
    fn description(&self) -> &'static str {
        "Stop one or more running Docker containers"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "containers",
                "array",
                "Container names or IDs to stop",
                true,
            )
            .with_param(
                "timeout",
                "integer",
                "Seconds to wait before SIGKILL (default 10)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let arr = args
            .get("containers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing containers array".to_string()))?;
        if arr.is_empty() {
            return Err(Error::Tool("containers array must not be empty".to_string()));
        }
        let names: Vec<String> = arr
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| Error::Tool("containers entries must be strings".to_string()))?;
                validate_identifier(s, "container")?;
                Ok(s.to_string())
            })
            .collect::<Result<Vec<_>>>()?;

        let timeout = args
            .get("timeout")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .clamp(0, 3600);
        let timeout_str = timeout.to_string();

        let mut argv: Vec<&str> = vec!["stop", "--time", &timeout_str];
        for n in &names {
            argv.push(n);
        }
        run_docker(&argv).await
    }
}

// ---------- docker_compose ----------

/// Run a docker compose subcommand.
pub struct DockerComposeTool;

#[async_trait]
impl TalosTool for DockerComposeTool {
    fn name(&self) -> &'static str {
        "docker_compose"
    }
    fn description(&self) -> &'static str {
        "Run a docker compose subcommand (up, down, ps, logs, build, restart)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "subcommand",
                "string",
                "Subcommand: up | down | ps | logs | build | restart | pull | config",
                true,
            )
            .with_param(
                "file",
                "string",
                "Optional path to a compose file (-f)",
                false,
            )
            .with_param(
                "project",
                "string",
                "Optional project name (-p)",
                false,
            )
            .with_param(
                "services",
                "array",
                "Optional list of services to scope the command to",
                false,
            )
            .with_param(
                "detach",
                "boolean",
                "For 'up': run in detached mode (-d). Default true.",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        const ALLOWED: &[&str] = &[
            "up", "down", "ps", "logs", "build", "restart", "pull", "config",
        ];
        let sub = args
            .get("subcommand")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing subcommand".to_string()))?;
        if !ALLOWED.contains(&sub) {
            return Err(Error::Tool(format!(
                "subcommand '{}' not allowed (allowed: {:?})",
                sub, ALLOWED
            )));
        }

        let mut argv: Vec<String> = vec!["compose".to_string()];
        if let Some(f) = args.get("file").and_then(|v| v.as_str()) {
            // path: allow / . _ - alphanumerics and digits
            if !f
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
            {
                return Err(Error::Tool("file path contains invalid characters".into()));
            }
            argv.push("-f".to_string());
            argv.push(f.to_string());
        }
        if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
            validate_identifier(p, "project")?;
            argv.push("-p".to_string());
            argv.push(p.to_string());
        }

        argv.push(sub.to_string());

        // Subcommand-specific defaults
        if sub == "up" {
            let detach = args
                .get("detach")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if detach {
                argv.push("-d".to_string());
            }
        }
        if sub == "down" {
            // safer default: don't remove volumes implicitly
        }

        if let Some(svc_arr) = args.get("services").and_then(|v| v.as_array()) {
            for v in svc_arr {
                let s = v
                    .as_str()
                    .ok_or_else(|| Error::Tool("services entries must be strings".to_string()))?;
                validate_identifier(s, "service")?;
                argv.push(s.to_string());
            }
        }

        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        run_docker(&argv_refs).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identifiers() {
        assert!(validate_identifier("nginx", "x").is_ok());
        assert!(validate_identifier("my_container-1", "x").is_ok());
        assert!(validate_identifier("registry.io/foo:bar", "x").is_ok());
        assert!(validate_identifier("", "x").is_err());
        assert!(validate_identifier("bad name", "x").is_err());
        assert!(validate_identifier("bad;rm", "x").is_err());
        assert!(validate_identifier("bad$(x)", "x").is_err());
    }

    #[test]
    fn truncate_caps_output() {
        let big = "x".repeat(MAX_OUTPUT_BYTES + 100);
        let out = truncate_output(big);
        assert!(out.ends_with("[truncated]"));
        assert!(out.len() <= MAX_OUTPUT_BYTES + 32);
    }

    #[test]
    fn ps_schema_has_no_required_args() {
        let t = DockerPsTool;
        let s = t.schema();
        assert_eq!(s.name, "docker_ps");
        let required = s
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        assert!(required.is_empty());
    }

    #[test]
    fn exec_schema_requires_container_and_command() {
        let s = DockerExecTool.schema();
        let required = s
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"container"));
        assert!(names.contains(&"command"));
    }

    #[test]
    fn compose_rejects_unknown_subcommand() {
        let t = DockerComposeTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({"subcommand": "rm-rf"})));
        assert!(res.is_err());
    }

    #[test]
    fn exec_rejects_bad_container_name() {
        let t = DockerExecTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({
            "container": "; rm -rf /",
            "command": ["ls"],
        })));
        assert!(res.is_err());
    }
}
