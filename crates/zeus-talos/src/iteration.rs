//! Iteration, loop, and flow control tools
//!
//! These tools enable autonomous multi-step workflows by providing
//! loop primitives, batch execution, and flow control.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

// ============================================================================
// for_each_file - Run a command on every file matching a glob
// ============================================================================

pub struct ForEachFileTool;

#[async_trait]
impl TalosTool for ForEachFileTool {
    fn name(&self) -> &'static str {
        "for_each_file"
    }

    fn description(&self) -> &'static str {
        "Run a shell command on every file matching a glob pattern. Use {} as file placeholder in the command."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "pattern",
                "string",
                "Glob pattern (e.g. '*.txt', 'src/**/*.rs')",
                true,
            )
            .with_param(
                "command",
                "string",
                "Command to run. Use {} as file path placeholder",
                true,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'pattern' parameter".to_string()))?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let glob_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", directory, pattern)
        };

        let paths: Vec<_> = glob::glob(&glob_pattern)
            .map_err(|e| Error::Tool(format!("Invalid glob pattern: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        if paths.is_empty() {
            return Ok(format!("No files matched pattern: {}", pattern));
        }

        let mut results = Vec::new();
        let mut success_count = 0;
        let mut fail_count = 0;

        for path in &paths {
            let path_str = path.display().to_string();
            // Safe: {} substituted as a distinct argument — no shell interpolation of filenames
            let mut fe_argv: Vec<String> = command
                .split_whitespace()
                .map(|tok| {
                    if tok == "{}" {
                        path_str.clone()
                    } else {
                        tok.to_string()
                    }
                })
                .collect();
            if fe_argv.is_empty() {
                return Err(Error::Tool("Empty command template".to_string()));
            }
            let fe_prog = fe_argv.remove(0);
            let output = tokio::process::Command::new(&fe_prog)
                .args(&fe_argv)
                .current_dir(directory)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to execute: {}", e)))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if output.status.success() {
                success_count += 1;
                if !stdout.trim().is_empty() {
                    results.push(format!("[OK] {} -> {}", path_str, stdout.trim()));
                } else {
                    results.push(format!("[OK] {}", path_str));
                }
            } else {
                fail_count += 1;
                results.push(format!("[FAIL] {} -> {}", path_str, stderr.trim()));
            }
        }

        Ok(format!(
            "Processed {} files ({} ok, {} failed):\n{}",
            paths.len(),
            success_count,
            fail_count,
            results.join("\n")
        ))
    }
}

// ============================================================================
// batch_execute - Run a list of commands sequentially
// ============================================================================

pub struct BatchExecuteTool;

#[async_trait]
impl TalosTool for BatchExecuteTool {
    fn name(&self) -> &'static str {
        "batch_execute"
    }

    fn description(&self) -> &'static str {
        "Run a list of shell commands sequentially, collecting all results"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "commands",
                "array",
                "List of shell commands to execute sequentially",
                true,
            )
            .with_param(
                "stop_on_error",
                "boolean",
                "Stop on first error (default: false)",
                false,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let commands = args
            .get("commands")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing 'commands' array parameter".to_string()))?;
        let stop_on_error = args
            .get("stop_on_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let mut results = Vec::new();
        let mut success_count = 0;
        let mut fail_count = 0;

        for (i, cmd_val) in commands.iter().enumerate() {
            let cmd = cmd_val
                .as_str()
                .ok_or_else(|| Error::Tool(format!("Command {} is not a string", i)))?;

            let mut be_argv: Vec<&str> = cmd.split_whitespace().collect();
            if be_argv.is_empty() {
                return Err(Error::Tool(format!("Command {} is empty", i)));
            }
            let be_prog = be_argv.remove(0);
            let output = tokio::process::Command::new(be_prog)
                .args(&be_argv)
                .current_dir(directory)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to execute command {}: {}", i, e)))?;

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if output.status.success() {
                success_count += 1;
                let out = if stdout.is_empty() {
                    "(no output)".to_string()
                } else {
                    stdout
                };
                results.push(format!("[{}] OK: {} -> {}", i + 1, cmd, out));
            } else {
                fail_count += 1;
                results.push(format!("[{}] FAIL: {} -> {}", i + 1, cmd, stderr));
                if stop_on_error {
                    results.push(format!(
                        "Stopped: stop_on_error=true after command {}",
                        i + 1
                    ));
                    break;
                }
            }
        }

        Ok(format!(
            "Executed {} commands ({} ok, {} failed):\n{}",
            success_count + fail_count,
            success_count,
            fail_count,
            results.join("\n")
        ))
    }
}

// ============================================================================
// parallel_execute - Run multiple commands in parallel
// ============================================================================

pub struct ParallelExecuteTool;

#[async_trait]
impl TalosTool for ParallelExecuteTool {
    fn name(&self) -> &'static str {
        "parallel_execute"
    }

    fn description(&self) -> &'static str {
        "Run multiple shell commands in parallel, return when all complete"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "commands",
                "array",
                "List of shell commands to execute in parallel",
                true,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let commands = args
            .get("commands")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing 'commands' array parameter".to_string()))?;
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        let mut handles = Vec::new();

        for cmd_val in commands {
            let cmd = cmd_val
                .as_str()
                .ok_or_else(|| Error::Tool("Command is not a string".to_string()))?
                .to_string();
            let dir = directory.clone();

            handles.push(tokio::spawn(async move {
                let mut pe_argv: Vec<String> =
                    cmd.split_whitespace().map(|s| s.to_string()).collect();
                let output = if pe_argv.is_empty() {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "empty command",
                    ))
                } else {
                    let pe_prog = pe_argv.remove(0);
                    tokio::process::Command::new(&pe_prog)
                        .args(&pe_argv)
                        .current_dir(&dir)
                        .output()
                        .await
                };
                (cmd, output)
            }));
        }

        let mut results = Vec::new();
        let mut success_count = 0;
        let mut fail_count = 0;

        for (i, handle) in handles.into_iter().enumerate() {
            match handle.await {
                Ok((cmd, Ok(output))) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if output.status.success() {
                        success_count += 1;
                        let out = if stdout.is_empty() {
                            "(no output)".to_string()
                        } else {
                            stdout
                        };
                        results.push(format!("[{}] OK: {} -> {}", i + 1, cmd, out));
                    } else {
                        fail_count += 1;
                        results.push(format!("[{}] FAIL: {} -> {}", i + 1, cmd, stderr));
                    }
                }
                Ok((cmd, Err(e))) => {
                    fail_count += 1;
                    results.push(format!("[{}] ERROR: {} -> {}", i + 1, cmd, e));
                }
                Err(e) => {
                    fail_count += 1;
                    results.push(format!("[{}] PANIC: {}", i + 1, e));
                }
            }
        }

        Ok(format!(
            "Parallel: {} commands ({} ok, {} failed):\n{}",
            success_count + fail_count,
            success_count,
            fail_count,
            results.join("\n")
        ))
    }
}

// ============================================================================
// repeat - Run a command N times
// ============================================================================

pub struct RepeatTool;

#[async_trait]
impl TalosTool for RepeatTool {
    fn name(&self) -> &'static str {
        "repeat"
    }

    fn description(&self) -> &'static str {
        "Run a shell command N times with optional delay between runs"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("command", "string", "Shell command to repeat", true)
            .with_param("count", "integer", "Number of times to run", true)
            .with_param(
                "delay_ms",
                "integer",
                "Delay between runs in milliseconds (default: 0)",
                false,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing 'count' parameter".to_string()))?
            as usize;
        let delay_ms = args.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        if count > 1000 {
            return Err(Error::Tool("Count must be <= 1000".to_string()));
        }

        let mut results = Vec::new();
        let mut success_count = 0;

        for i in 0..count {
            if i > 0 && delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let mut rp_argv: Vec<&str> = command.split_whitespace().collect();
            if rp_argv.is_empty() {
                return Err(Error::Tool("Empty command".to_string()));
            }
            let rp_prog = rp_argv.remove(0);
            let output = tokio::process::Command::new(rp_prog)
                .args(&rp_argv)
                .current_dir(directory)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed at iteration {}: {}", i + 1, e)))?;

            if output.status.success() {
                success_count += 1;
            }

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stdout.is_empty() {
                results.push(format!("[{}] {}", i + 1, stdout));
            }
        }

        Ok(format!(
            "Repeated {} times ({} succeeded):\n{}",
            count,
            success_count,
            if results.is_empty() {
                "(no output)".to_string()
            } else {
                results.join("\n")
            }
        ))
    }
}

// ============================================================================
// until_success - Retry until success
// ============================================================================

pub struct UntilSuccessTool;

#[async_trait]
impl TalosTool for UntilSuccessTool {
    fn name(&self) -> &'static str {
        "until_success"
    }

    fn description(&self) -> &'static str {
        "Retry a shell command until it succeeds (exit 0), with max attempts and exponential backoff"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("command", "string", "Shell command to retry", true)
            .with_param(
                "max_attempts",
                "integer",
                "Maximum retry attempts (default: 5)",
                false,
            )
            .with_param(
                "backoff_ms",
                "integer",
                "Initial backoff in milliseconds, doubles each retry (default: 1000)",
                false,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let max_attempts = args
            .get("max_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        let backoff_ms = args
            .get("backoff_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        if max_attempts > 50 {
            return Err(Error::Tool("max_attempts must be <= 50".to_string()));
        }

        let mut current_backoff = backoff_ms;

        for attempt in 1..=max_attempts {
            let mut us_argv: Vec<&str> = command.split_whitespace().collect();
            if us_argv.is_empty() {
                return Err(Error::Tool("Empty command".to_string()));
            }
            let us_prog = us_argv.remove(0);
            let output = tokio::process::Command::new(us_prog)
                .args(&us_argv)
                .current_dir(directory)
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!("Attempt {} failed to execute: {}", attempt, e))
                })?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok(format!(
                    "Succeeded on attempt {}/{}:\n{}",
                    attempt,
                    max_attempts,
                    if stdout.is_empty() {
                        "(no output)".to_string()
                    } else {
                        stdout
                    }
                ));
            }

            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if attempt < max_attempts {
                tokio::time::sleep(std::time::Duration::from_millis(current_backoff)).await;
                current_backoff = (current_backoff * 2).min(30000); // cap at 30s
            } else {
                return Ok(format!(
                    "Failed after {} attempts. Last error:\n{}",
                    max_attempts, stderr
                ));
            }
        }

        Ok("Unexpected: loop ended without result".to_string())
    }
}

// ============================================================================
// while_condition - Loop while condition is true
// ============================================================================

pub struct WhileConditionTool;

#[async_trait]
impl TalosTool for WhileConditionTool {
    fn name(&self) -> &'static str {
        "while_condition"
    }

    fn description(&self) -> &'static str {
        "Run a command repeatedly while a condition holds (file_exists, process_running, port_open, command_succeeds)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "command",
                "string",
                "Shell command to run each iteration",
                true,
            )
            .with_param(
                "condition",
                "string",
                "Condition type: file_exists, process_running, port_open, command_succeeds",
                true,
            )
            .with_param(
                "check_value",
                "string",
                "Value to check (file path, process name, port number, or shell command)",
                true,
            )
            .with_param(
                "max_iterations",
                "integer",
                "Maximum iterations (default: 100)",
                false,
            )
            .with_param(
                "interval_ms",
                "integer",
                "Delay between iterations in ms (default: 1000)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let condition = args
            .get("condition")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'condition' parameter".to_string()))?;
        let check_value = args
            .get("check_value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'check_value' parameter".to_string()))?;
        let max_iterations = args
            .get("max_iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;
        let interval_ms = args
            .get("interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        if max_iterations > 10000 {
            return Err(Error::Tool("max_iterations must be <= 10000".to_string()));
        }

        for iterations in 0..max_iterations {
            // Check condition
            let condition_met = match condition {
                "file_exists" => std::path::Path::new(check_value).exists(),
                "process_running" => tokio::process::Command::new("pgrep")
                    .arg("-x")
                    .arg(check_value)
                    .output()
                    .await
                    .map(|o| o.status.success())
                    .unwrap_or(false),
                "port_open" => {
                    // Safe: validate port is numeric, pass as distinct arg
                    let port_digits: String =
                        check_value.chars().filter(|c| c.is_ascii_digit()).collect();
                    if port_digits.is_empty() || port_digits != check_value {
                        false
                    } else {
                        tokio::process::Command::new("lsof")
                            .arg("-i")
                            .arg(format!(":{}", port_digits))
                            .arg("-sTCP:LISTEN")
                            .output()
                            .await
                            .map(|o| o.status.success())
                            .unwrap_or(false)
                    }
                }
                "command_succeeds" => {
                    // Safe: parse into argv, no shell interpolation
                    let wc_cs_argv: Vec<&str> = check_value.split_whitespace().collect();
                    if wc_cs_argv.is_empty() {
                        false
                    } else {
                        tokio::process::Command::new(wc_cs_argv[0])
                            .args(&wc_cs_argv[1..])
                            .output()
                            .await
                            .map(|o| o.status.success())
                            .unwrap_or(false)
                    }
                }
                _ => {
                    return Err(Error::Tool(format!(
                        "Unknown condition: {}. Use: file_exists, process_running, port_open, command_succeeds",
                        condition
                    )));
                }
            };

            if !condition_met {
                return Ok(format!(
                    "Condition '{}={}' became false after {} iterations",
                    condition, check_value, iterations
                ));
            }

            // Run the command — safe: split into argv, no shell
            if let Some(wc_prog) = command.split_whitespace().next() {
                let wc_body_args: Vec<&str> = command.split_whitespace().skip(1).collect();
                let _ = tokio::process::Command::new(wc_prog)
                    .args(&wc_body_args)
                    .output()
                    .await;
            }

            if interval_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
            }
        }

        Ok(format!(
            "Reached max_iterations ({}) while condition still true",
            max_iterations
        ))
    }
}

// ============================================================================
// search_replace_bulk - Find and replace across multiple files
// ============================================================================

pub struct SearchReplaceBulkTool;

#[async_trait]
impl TalosTool for SearchReplaceBulkTool {
    fn name(&self) -> &'static str {
        "search_replace_bulk"
    }

    fn description(&self) -> &'static str {
        "Find and replace text across multiple files matching a glob pattern"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("pattern", "string", "Text or regex pattern to find", true)
            .with_param("replacement", "string", "Replacement text", true)
            .with_param("glob", "string", "File glob pattern (e.g. '**/*.rs')", true)
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
            .with_param(
                "dry_run",
                "boolean",
                "Preview changes without modifying files (default: true)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'pattern' parameter".to_string()))?;
        let replacement = args
            .get("replacement")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'replacement' parameter".to_string()))?;
        let glob_pattern = args
            .get("glob")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'glob' parameter".to_string()))?;
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let re =
            regex::Regex::new(pattern).map_err(|e| Error::Tool(format!("Invalid regex: {}", e)))?;

        // Security: reject absolute glob patterns that could escape working directory
        if glob_pattern.starts_with('/') || glob_pattern.contains("../") {
            return Err(Error::Tool(
                "Glob pattern must be relative and must not escape the working directory"
                    .to_string(),
            ));
        }

        let canonical_dir = std::fs::canonicalize(directory)
            .map_err(|e| Error::Tool(format!("Invalid working directory: {}", e)))?;

        let full_glob = format!("{}/{}", directory, glob_pattern);

        let paths: Vec<_> = glob::glob(&full_glob)
            .map_err(|e| Error::Tool(format!("Invalid glob: {}", e)))?
            .filter_map(|r| r.ok())
            .filter(|p| p.is_file())
            // Security: verify every matched path is inside the working directory
            .filter(|p| {
                std::fs::canonicalize(p)
                    .map(|canon| canon.starts_with(&canonical_dir))
                    .unwrap_or(false)
            })
            .collect();

        let mut modified_files = Vec::new();
        let mut total_replacements = 0;

        for path in &paths {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // skip binary files
            };

            let match_count = re.find_iter(&content).count();
            if match_count == 0 {
                continue;
            }

            total_replacements += match_count;
            let new_content = re.replace_all(&content, replacement).to_string();
            modified_files.push(format!(
                "  {} ({} replacements)",
                path.display(),
                match_count
            ));

            if !dry_run {
                std::fs::write(path, new_content).map_err(|e| {
                    Error::Tool(format!("Failed to write {}: {}", path.display(), e))
                })?;
            }
        }

        let mode = if dry_run { "DRY RUN" } else { "APPLIED" };
        Ok(format!(
            "[{}] {} replacements in {} files (scanned {}):\n{}",
            mode,
            total_replacements,
            modified_files.len(),
            paths.len(),
            if modified_files.is_empty() {
                "  (no matches found)".to_string()
            } else {
                modified_files.join("\n")
            }
        ))
    }
}

// ============================================================================
// pipe - Chain commands (stdout -> stdin)
// ============================================================================

pub struct PipeTool;

#[async_trait]
impl TalosTool for PipeTool {
    fn name(&self) -> &'static str {
        "pipe"
    }

    fn description(&self) -> &'static str {
        "Chain shell commands: stdout of each feeds into stdin of the next"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "commands",
                "array",
                "List of commands to chain via pipes",
                true,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let commands = args
            .get("commands")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Tool("Missing 'commands' array parameter".to_string()))?;
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        // Build a single piped command
        let cmd_strs: Vec<&str> = commands.iter().filter_map(|v| v.as_str()).collect();

        if cmd_strs.is_empty() {
            return Err(Error::Tool("No valid commands provided".to_string()));
        }

        let piped = cmd_strs.join(" | ");

        // Safe pipe: spawn each command separately and connect stdout -> stdin
        let pipe_cmds: Vec<Vec<&str>> = cmd_strs
            .iter()
            .map(|c| c.split_whitespace().collect())
            .collect();
        if pipe_cmds.iter().any(|a| a.is_empty()) {
            return Err(Error::Tool("Empty command in pipe".to_string()));
        }
        use std::process::Stdio;
        let mut prev_stdout: Option<std::process::ChildStdout> = None;
        let mut children = Vec::new();
        for (pi, argv) in pipe_cmds.iter().enumerate() {
            let stdin_src = if let Some(prev) = prev_stdout.take() {
                Stdio::from(prev)
            } else {
                Stdio::null()
            };
            let is_last = pi == pipe_cmds.len() - 1;
            let mut child = std::process::Command::new(argv[0])
                .args(&argv[1..])
                .stdin(stdin_src)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(directory)
                .spawn()
                .map_err(|e| Error::Tool(format!("Pipe spawn failed at cmd {}: {}", pi, e)))?;
            if !is_last {
                prev_stdout = child.stdout.take();
            }
            children.push(child);
        }
        let output = children
            .into_iter()
            .last()
            .ok_or_else(|| Error::Tool("Pipe produced no children".to_string()))?
            .wait_with_output()
            .map_err(|e| Error::Tool(format!("Pipe execution failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(format!("Pipe: {}\n{}", piped, stdout))
        } else {
            Ok(format!(
                "Pipe failed: {}\nstderr: {}\nstdout: {}",
                piped, stderr, stdout
            ))
        }
    }
}

// ============================================================================
// conditional - If/else command execution
// ============================================================================

pub struct ConditionalTool;

#[async_trait]
impl TalosTool for ConditionalTool {
    fn name(&self) -> &'static str {
        "conditional"
    }

    fn description(&self) -> &'static str {
        "Run one command if a condition is true, another if false. Conditions: file_exists, process_running, port_open, command_succeeds"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "condition",
                "string",
                "Condition type: file_exists, process_running, port_open, command_succeeds",
                true,
            )
            .with_param("check_value", "string", "Value to check", true)
            .with_param(
                "if_true",
                "string",
                "Command to run if condition is true",
                true,
            )
            .with_param(
                "if_false",
                "string",
                "Command to run if condition is false (optional)",
                false,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let condition = args
            .get("condition")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'condition' parameter".to_string()))?;
        let check_value = args
            .get("check_value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'check_value' parameter".to_string()))?;
        let if_true = args
            .get("if_true")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'if_true' parameter".to_string()))?;
        let if_false = args.get("if_false").and_then(|v| v.as_str());
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let condition_met = match condition {
            "file_exists" => std::path::Path::new(check_value).exists(),
            "process_running" => tokio::process::Command::new("pgrep")
                .arg("-x")
                .arg(check_value)
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false),
            "port_open" => {
                let ct_port_digits: String =
                    check_value.chars().filter(|c| c.is_ascii_digit()).collect();
                if ct_port_digits.is_empty() || ct_port_digits != check_value {
                    false
                } else {
                    tokio::process::Command::new("lsof")
                        .arg("-i")
                        .arg(format!(":{}", ct_port_digits))
                        .arg("-sTCP:LISTEN")
                        .output()
                        .await
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                }
            }
            "command_succeeds" => {
                let ct_cs_argv: Vec<&str> = check_value.split_whitespace().collect();
                if ct_cs_argv.is_empty() {
                    false
                } else {
                    tokio::process::Command::new(ct_cs_argv[0])
                        .args(&ct_cs_argv[1..])
                        .output()
                        .await
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                }
            }
            _ => return Err(Error::Tool(format!("Unknown condition: {}", condition))),
        };

        let cmd_to_run = if condition_met {
            if_true
        } else if let Some(cmd) = if_false {
            cmd
        } else {
            return Ok(format!(
                "Condition '{}={}' is false, no if_false command provided",
                condition, check_value
            ));
        };

        let mut ct_argv: Vec<&str> = cmd_to_run.split_whitespace().collect();
        if ct_argv.is_empty() {
            return Err(Error::Tool("Empty command to run".to_string()));
        }
        let ct_prog = ct_argv.remove(0);
        let output = tokio::process::Command::new(ct_prog)
            .args(&ct_argv)
            .current_dir(directory)
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to execute: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let branch = if condition_met { "true" } else { "false" };

        Ok(format!(
            "Condition '{}={}' -> {} -> ran: {}\n{}",
            condition, check_value, branch, cmd_to_run, stdout
        ))
    }
}

// ============================================================================
// watch_path - Watch for file changes
// ============================================================================

pub struct WatchPathTool;

#[async_trait]
impl TalosTool for WatchPathTool {
    fn name(&self) -> &'static str {
        "watch_path"
    }

    fn description(&self) -> &'static str {
        "Watch a file or directory for changes. Runs a command when changes detected. Uses polling."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "File or directory path to watch", true)
            .with_param(
                "command",
                "string",
                "Command to run when changes detected",
                true,
            )
            .with_param(
                "timeout_seconds",
                "integer",
                "How long to watch (default: 60, max: 300)",
                false,
            )
            .with_param(
                "poll_interval_ms",
                "integer",
                "Polling interval in ms (default: 1000)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'path' parameter".to_string()))?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let timeout_secs = args
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60)
            .min(300);
        let poll_ms = args
            .get("poll_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        let target = std::path::Path::new(path);
        if !target.exists() {
            return Err(Error::Tool(format!("Path does not exist: {}", path)));
        }

        // Get initial modification time
        let initial_mtime = std::fs::metadata(target)
            .and_then(|m| m.modified())
            .map_err(|e| Error::Tool(format!("Cannot read metadata: {}", e)))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;

            let current_mtime = std::fs::metadata(target)
                .and_then(|m| m.modified())
                .unwrap_or(initial_mtime);

            if current_mtime != initial_mtime {
                if let Some(wp_prog) = command.split_whitespace().next() {
                    let wp_args: Vec<&str> = command.split_whitespace().skip(1).collect();
                    let _ = tokio::process::Command::new(wp_prog)
                        .args(&wp_args)
                        .output()
                        .await;
                }

                return Ok(format!(
                    "Change detected on '{}' after watching. Ran: {}",
                    path, command
                ));
            }
        }

        Ok(format!(
            "No changes detected on '{}' within {} seconds",
            path, timeout_secs
        ))
    }
}

// ============================================================================
// for_each_line - Run command on each line of a file
// ============================================================================

pub struct ForEachLineTool;

#[async_trait]
impl TalosTool for ForEachLineTool {
    fn name(&self) -> &'static str {
        "for_each_line"
    }

    fn description(&self) -> &'static str {
        "Run a shell command on each line of a file. Use {} as line content placeholder."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "file_path",
                "string",
                "Path to the file to read lines from",
                true,
            )
            .with_param(
                "command",
                "string",
                "Command to run per line. Use {} as placeholder for the line content",
                true,
            )
            .with_param(
                "directory",
                "string",
                "Working directory (default: current dir)",
                false,
            )
            .with_param(
                "skip_empty",
                "boolean",
                "Skip empty lines (default: true)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'file_path' parameter".to_string()))?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command' parameter".to_string()))?;
        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let skip_empty = args
            .get("skip_empty")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let content = std::fs::read_to_string(file_path)
            .map_err(|e| Error::Tool(format!("Failed to read file: {}", e)))?;

        let lines: Vec<&str> = content
            .lines()
            .filter(|l| !skip_empty || !l.trim().is_empty())
            .collect();

        if lines.len() > 10000 {
            return Err(Error::Tool(
                "File has > 10000 lines, too many to iterate".to_string(),
            ));
        }

        let mut results = Vec::new();
        let mut success_count = 0;

        for (i, line) in lines.iter().enumerate() {
            // Safe: {} substituted as a distinct argument — no shell interpolation of line content
            let mut fl_argv: Vec<String> = command
                .split_whitespace()
                .map(|tok| {
                    if tok == "{}" {
                        line.to_string()
                    } else {
                        tok.to_string()
                    }
                })
                .collect();
            if fl_argv.is_empty() {
                return Err(Error::Tool("Empty command template".to_string()));
            }
            let fl_prog = fl_argv.remove(0);
            let output = tokio::process::Command::new(&fl_prog)
                .args(&fl_argv)
                .current_dir(directory)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Line {} failed: {}", i + 1, e)))?;

            if output.status.success() {
                success_count += 1;
            }

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stdout.is_empty() && results.len() < 100 {
                results.push(format!("[{}] {}", i + 1, stdout));
            }
        }

        Ok(format!(
            "Processed {} lines ({} succeeded):\n{}",
            lines.len(),
            success_count,
            if results.is_empty() {
                "(no output)".to_string()
            } else {
                results.join("\n")
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_batch_execute() {
        let tool = BatchExecuteTool;
        let result = tool
            .execute(json!({
                "commands": ["echo hello", "echo world"],
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("2 ok"));
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[tokio::test]
    async fn test_parallel_execute() {
        let tool = ParallelExecuteTool;
        let result = tool
            .execute(json!({
                "commands": ["echo a", "echo b", "echo c"],
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("3 ok"));
    }

    #[tokio::test]
    async fn test_repeat() {
        let tool = RepeatTool;
        let result = tool
            .execute(json!({
                "command": "echo hi",
                "count": 3,
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("3 times"));
        assert!(result.contains("3 succeeded"));
    }

    #[tokio::test]
    async fn test_until_success() {
        let tool = UntilSuccessTool;
        let result = tool
            .execute(json!({
                "command": "echo ok",
                "max_attempts": 3,
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("Succeeded on attempt 1"));
    }

    #[tokio::test]
    async fn test_pipe() {
        let tool = PipeTool;
        let result = tool
            .execute(json!({
                "commands": ["echo hello world", "wc -w"],
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("2"));
    }

    #[tokio::test]
    async fn test_conditional_true() {
        let tool = ConditionalTool;
        let result = tool
            .execute(json!({
                "condition": "command_succeeds",
                "check_value": "true",
                "if_true": "echo yes",
                "if_false": "echo no",
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("true"));
        assert!(result.contains("yes"));
    }

    #[tokio::test]
    async fn test_conditional_false() {
        let tool = ConditionalTool;
        let result = tool
            .execute(json!({
                "condition": "command_succeeds",
                "check_value": "false",
                "if_true": "echo yes",
                "if_false": "echo no",
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("false"));
        assert!(result.contains("no"));
    }

    #[tokio::test]
    async fn test_search_replace_dry_run() {
        let tool = SearchReplaceBulkTool;
        let result = tool
            .execute(json!({
                "pattern": "nonexistent_string_xyz",
                "replacement": "replaced",
                "glob": "*.rs",
                "directory": "/tmp",
                "dry_run": true,
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("DRY RUN"));
    }

    #[tokio::test]
    async fn test_for_each_file() {
        let tool = ForEachFileTool;
        // Use /dev/null pattern that won't match much
        let result = tool
            .execute(json!({
                "pattern": "/nonexistent_dir_xyz/*",
                "command": "echo {}",
            }))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("No files matched"));
    }
    // ── Error-path tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_batch_execute_stop_on_error() {
        let tool = BatchExecuteTool;
        let result = tool
            .execute(json!({
                "commands": ["echo first", "false", "echo third"],
                "stop_on_error": true,
            }))
            .await
            .expect("batch_execute should not error at tool level");
        // Should stop after false — third should not appear
        assert!(result.contains("first"));
        assert!(!result.contains("third") || result.contains("1 ok"));
    }

    #[tokio::test]
    async fn test_batch_execute_empty_command_errors() {
        let tool = BatchExecuteTool;
        let result = tool
            .execute(json!({
                "commands": [""],
            }))
            .await;
        assert!(result.is_err(), "empty command should return Err");
    }

    #[tokio::test]
    async fn test_repeat_count_limit() {
        let tool = RepeatTool;
        let result = tool
            .execute(json!({
                "command": "echo x",
                "count": 1001,
            }))
            .await;
        assert!(result.is_err(), "count > 1000 should return Err");
    }

    #[tokio::test]
    async fn test_for_each_line_missing_file() {
        let tool = ForEachLineTool;
        let result = tool
            .execute(json!({
                "file": "/nonexistent_file_xyz_abc.txt",
                "command": "echo {}",
            }))
            .await;
        assert!(result.is_err(), "missing file should return Err");
    }

    #[tokio::test]
    async fn test_search_replace_absolute_glob_rejected() {
        let tool = SearchReplaceBulkTool;
        let result = tool
            .execute(json!({
                "pattern": "foo",
                "replacement": "bar",
                "glob": "/**/*.rs",
                "dry_run": true,
            }))
            .await;
        assert!(result.is_err(), "absolute glob should be rejected");
    }

    #[tokio::test]
    async fn test_search_replace_traversal_rejected() {
        let tool = SearchReplaceBulkTool;
        let result = tool
            .execute(json!({
                "pattern": "foo",
                "replacement": "bar",
                "glob": "../../../etc/passwd",
                "dry_run": true,
            }))
            .await;
        assert!(result.is_err(), "path traversal glob should be rejected");
    }

    #[tokio::test]
    async fn test_while_condition_unknown_condition() {
        let tool = WhileConditionTool;
        let result = tool
            .execute(json!({
                "command": "echo x",
                "condition": "invalid_condition_xyz",
                "check_value": "foo",
                "max_iterations": 1,
            }))
            .await;
        assert!(result.is_err(), "unknown condition should return Err");
    }

    #[tokio::test]
    async fn test_pipe_empty_command_rejected() {
        let tool = PipeTool;
        let result = tool
            .execute(json!({
                "commands": ["echo hello", ""],
            }))
            .await;
        assert!(result.is_err(), "empty command in pipe should return Err");
    }
}
