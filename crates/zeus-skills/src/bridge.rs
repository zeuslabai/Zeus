//! Process bridges for communicating with external plugin runtimes.
//!
//! The bridge protocol uses newline-delimited JSON over stdin/stdout:
//!
//! **Request:** `{"method": "execute_tool", "params": {"name": "...", "args": {...}}}`
//! **Response:** `{"result": "...", "error": null}` or `{"result": null, "error": "..."}`

use crate::loader::PluginManifest;
use crate::plugin::Plugin;
use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use zeus_core::{Error, Result, ToolSchema};

/// Default timeout for a single bridge call (30 seconds).
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bridge for communicating with an external process over stdin/stdout.
///
/// The child process is expected to read newline-delimited JSON requests from
/// stdin and write newline-delimited JSON responses to stdout.
pub struct ProcessBridge {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl std::fmt::Debug for ProcessBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessBridge")
            .field("child_id", &self.child.id())
            .finish()
    }
}

impl ProcessBridge {
    /// Spawn a child process and establish the bridge, enforcing the skill's
    /// recorded sandbox level (GAP#3 Cut-2).
    ///
    /// At `SandboxLevel::None` (full-trust) — and on every non-macOS platform,
    /// where `Sandbox::sandbox_command` is a graceful passthrough — the process
    /// is spawned directly via `Command::new`, exactly as before (no behavior
    /// change). At any restrictive level on macOS, the command + args are
    /// composed into a single line, wrapped by aegis Seatbelt via
    /// `sandbox_command`, and executed through `/bin/sh -c`.
    ///
    /// Sandbox-by-default: callers that have no recorded policy must pass the
    /// most-restrictive level (`Paranoid`), so an unknown skill never escapes
    /// unsandboxed.
    pub async fn spawn(
        command: &str,
        args: &[&str],
        cwd: Option<&Path>,
        sandbox_level: zeus_aegis::SandboxLevel,
    ) -> Result<Self> {
        let sandbox = zeus_aegis::Sandbox::new(sandbox_level);

        // Compose the raw command line, then ask aegis to wrap it. On `None` or
        // non-macOS, `sandbox_command` returns the input unchanged.
        let raw_line = if args.is_empty() {
            command.to_string()
        } else {
            format!("{} {}", command, args.join(" "))
        };
        let wrapped = sandbox.sandbox_command(&raw_line);

        let mut cmd = if wrapped == raw_line {
            // Passthrough (full-trust or non-macOS): exec directly, as before.
            let mut c = Command::new(command);
            c.args(args);
            c
        } else {
            // Sandboxed (macOS, restrictive level): exec the wrapped line via sh.
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(&wrapped);
            c
        };
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Skill(format!("Failed to spawn {}: {}", command, e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Skill("Failed to capture child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Skill("Failed to capture child stdout".to_string()))?;

        Ok(Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        })
    }

    /// Send a request and read the response.
    ///
    /// Times out after 30 seconds. The request is serialized as
    /// `{"method": "<method>", "params": <params>}\n`.
    pub async fn call(&mut self, method: &str, params: &Value) -> Result<Value> {
        let request = serde_json::json!({
            "method": method,
            "params": params,
        });

        let mut request_line = serde_json::to_string(&request)
            .map_err(|e| Error::Skill(format!("Failed to serialize request: {}", e)))?;
        request_line.push('\n');

        // Write the request
        self.stdin
            .write_all(request_line.as_bytes())
            .await
            .map_err(|e| Error::Skill(format!("Failed to write to child stdin: {}", e)))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| Error::Skill(format!("Failed to flush child stdin: {}", e)))?;

        // Read the response with timeout
        let mut response_line = String::new();
        let read_result = tokio::time::timeout(BRIDGE_TIMEOUT, async {
            self.stdout
                .read_line(&mut response_line)
                .await
                .map_err(|e| Error::Skill(format!("Failed to read from child stdout: {}", e)))
        })
        .await;

        match read_result {
            Ok(Ok(0)) => Err(Error::Skill(
                "Child process closed stdout unexpectedly".to_string(),
            )),
            Ok(Ok(_)) => {
                let response: Value = serde_json::from_str(response_line.trim()).map_err(|e| {
                    Error::Skill(format!(
                        "Malformed JSON from child process: {} (raw: {:?})",
                        e,
                        response_line.trim()
                    ))
                })?;

                // Check for error field
                if let Some(err) = response.get("error")
                    && !err.is_null()
                {
                    let msg = err.as_str().unwrap_or("Unknown plugin error");
                    return Err(Error::Skill(msg.to_string()));
                }

                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Timeout(
                "Plugin call timed out after 30 seconds".to_string(),
            )),
        }
    }

    /// Gracefully shut down the child process.
    pub async fn shutdown(&mut self) -> Result<()> {
        // Try to send a shutdown command first
        let shutdown_req = serde_json::json!({"method": "shutdown", "params": {}});
        let mut line = serde_json::to_string(&shutdown_req).unwrap_or_default();
        line.push('\n');
        let _ = self.stdin.write_all(line.as_bytes()).await;
        let _ = self.stdin.flush().await;

        // Give the process a moment to exit gracefully, then kill if needed
        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.child
                    .kill()
                    .await
                    .map_err(|e| Error::Skill(format!("Failed to kill child process: {}", e)))?;
                Ok(())
            }
        }
    }
}

/// Node.js plugin that communicates via ProcessBridge.
pub struct NodePlugin {
    manifest: PluginManifest,
    bridge: Mutex<ProcessBridge>,
}

impl std::fmt::Debug for NodePlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodePlugin")
            .field("name", &self.manifest.name)
            .finish()
    }
}

impl NodePlugin {
    /// Create a new Node.js plugin by spawning a `node` process.
    pub async fn new(manifest: PluginManifest, plugin_dir: &Path) -> Result<Self> {
        let entry = manifest.entry.as_deref().unwrap_or("index.js");
        let level = resolve_sandbox_level(&manifest.name, plugin_dir);
        let bridge = ProcessBridge::spawn("node", &[entry], Some(plugin_dir), level).await?;
        Ok(Self {
            manifest,
            bridge: Mutex::new(bridge),
        })
    }
}

/// Resolve the recorded sandbox level for a skill at execution time.
///
/// Reads `permissions.json` from the skills dir (the parent of `plugin_dir`)
/// and maps the recorded `trust_level` to a [`zeus_aegis::SandboxLevel`].
///
/// Sandbox-by-default: if no policy file exists, the file can't be read, or no
/// entry matches `skill_name`, this returns the most-restrictive `Paranoid`,
/// so an un-recorded skill is never spawned unsandboxed.
fn resolve_sandbox_level(skill_name: &str, plugin_dir: &Path) -> zeus_aegis::SandboxLevel {
    use crate::skill_permissions::{trust_level_to_sandbox, SkillPermissionPolicy};

    let perms_path = match plugin_dir.parent() {
        Some(skills_dir) => skills_dir.join("permissions.json"),
        None => return zeus_aegis::SandboxLevel::Paranoid,
    };

    let policies: Vec<SkillPermissionPolicy> = match std::fs::read_to_string(&perms_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => return zeus_aegis::SandboxLevel::Paranoid,
    };

    match policies.iter().find(|p| p.skill_name == skill_name) {
        Some(policy) => trust_level_to_sandbox(policy.trust_level),
        // Unknown skill → strictest, never unsandboxed.
        None => zeus_aegis::SandboxLevel::Paranoid,
    }
}

#[async_trait]
impl Plugin for NodePlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn version(&self) -> &str {
        &self.manifest.version
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn tools(&self) -> Vec<ToolSchema> {
        self.manifest
            .tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect()
    }

    async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
        let mut bridge = self.bridge.lock().await;
        let result = bridge
            .call(
                "execute_tool",
                &serde_json::json!({"name": name, "args": args}),
            )
            .await?;
        match result {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    fn hook_events(&self) -> Vec<String> {
        self.manifest.hooks.clone()
    }

    async fn on_hook_event(&self, event: &str, context: &Value) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge
            .call(
                "on_hook_event",
                &serde_json::json!({"event": event, "context": context}),
            )
            .await?;
        Ok(())
    }

    async fn init(&self) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge.call("init", &serde_json::json!({})).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge.shutdown().await
    }
}

/// Python plugin that communicates via ProcessBridge.
pub struct PythonPlugin {
    manifest: PluginManifest,
    bridge: Mutex<ProcessBridge>,
}

impl std::fmt::Debug for PythonPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PythonPlugin")
            .field("name", &self.manifest.name)
            .finish()
    }
}

impl PythonPlugin {
    /// Create a new Python plugin by spawning a `python3` process.
    pub async fn new(manifest: PluginManifest, plugin_dir: &Path) -> Result<Self> {
        let entry = manifest.entry.as_deref().unwrap_or("main.py");
        let level = resolve_sandbox_level(&manifest.name, plugin_dir);
        let bridge = ProcessBridge::spawn("python3", &[entry], Some(plugin_dir), level).await?;
        Ok(Self {
            manifest,
            bridge: Mutex::new(bridge),
        })
    }
}

#[async_trait]
impl Plugin for PythonPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn version(&self) -> &str {
        &self.manifest.version
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn tools(&self) -> Vec<ToolSchema> {
        self.manifest
            .tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect()
    }

    async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
        let mut bridge = self.bridge.lock().await;
        let result = bridge
            .call(
                "execute_tool",
                &serde_json::json!({"name": name, "args": args}),
            )
            .await?;
        match result {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    fn hook_events(&self) -> Vec<String> {
        self.manifest.hooks.clone()
    }

    async fn on_hook_event(&self, event: &str, context: &Value) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge
            .call(
                "on_hook_event",
                &serde_json::json!({"event": event, "context": context}),
            )
            .await?;
        Ok(())
    }

    async fn init(&self) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge.call("init", &serde_json::json!({})).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        bridge.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_request_serialization() {
        let request = serde_json::json!({
            "method": "execute_tool",
            "params": {
                "name": "greet",
                "args": {"name": "Zeus"}
            }
        });

        let serialized = serde_json::to_string(&request).expect("should serialize to JSON");
        assert!(serialized.contains("execute_tool"));
        assert!(serialized.contains("greet"));
        assert!(serialized.contains("Zeus"));

        // Round-trip
        let deserialized: Value =
            serde_json::from_str(&serialized).expect("should parse successfully");
        assert_eq!(deserialized["method"], "execute_tool");
        assert_eq!(deserialized["params"]["name"], "greet");
        assert_eq!(deserialized["params"]["args"]["name"], "Zeus");
    }

    #[test]
    fn test_bridge_response_parsing_success() {
        let response_str = r#"{"result": "Hello, Zeus!", "error": null}"#;
        let response: Value =
            serde_json::from_str(response_str).expect("should parse successfully");

        assert!(response["error"].is_null());
        assert_eq!(response["result"], "Hello, Zeus!");
    }

    #[test]
    fn test_bridge_response_parsing_error() {
        let response_str = r#"{"result": null, "error": "Something went wrong"}"#;
        let response: Value =
            serde_json::from_str(response_str).expect("should parse successfully");

        assert!(!response["error"].is_null());
        assert_eq!(response["error"], "Something went wrong");
    }

    #[test]
    fn test_bridge_response_parsing_malformed() {
        let response_str = "not valid json";
        let result: std::result::Result<Value, _> = serde_json::from_str(response_str);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bridge_spawn_and_call() {
        // Use a simple shell command that echoes JSON back
        // `cat` will read from stdin and echo to stdout
        let script = r#"
import sys, json
for line in sys.stdin:
    req = json.loads(line.strip())
    if req["method"] == "shutdown":
        break
    resp = {"result": "ok from " + req["method"], "error": None}
    print(json.dumps(resp), flush=True)
"#;

        // Write script to a temp file
        let tmp = tempfile::NamedTempFile::new().expect("NamedTempFile::new should succeed");
        std::fs::write(tmp.path(), script).expect("should write file");

        let result = ProcessBridge::spawn(
            "python3",
            &[tmp.path().to_str().expect("spawn should succeed")],
            None,
            zeus_aegis::SandboxLevel::None,
        )
        .await;

        // python3 might not be installed in all environments, so just check
        // that spawn works or fails gracefully
        match result {
            Ok(mut bridge) => {
                let response = bridge
                    .call("test_method", &serde_json::json!({}))
                    .await
                    .expect("async operation should succeed");
                assert_eq!(
                    response.as_str().expect("should be a string"),
                    "ok from test_method"
                );

                bridge
                    .shutdown()
                    .await
                    .expect("async operation should succeed");
            }
            Err(e) => {
                // python3 not available - this is acceptable in CI
                eprintln!("Skipping bridge test: {}", e);
            }
        }
    }

    // --- GAP#3 Cut-2: sandbox-by-default resolution ---

    #[test]
    fn test_resolve_sandbox_level_unknown_skill_is_strictest() {
        // No permissions.json on disk → most-restrictive, never unsandboxed.
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tmp.path().join("some-skill");
        std::fs::create_dir_all(&plugin_dir).expect("mkdir");
        let level = resolve_sandbox_level("some-skill", &plugin_dir);
        assert_eq!(level, zeus_aegis::SandboxLevel::Paranoid);
    }

    #[test]
    fn test_resolve_sandbox_level_reads_recorded_policy() {
        use crate::skill_permissions::{upsert_policy_file, SkillPermissionPolicy};
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tmp.path().join("trusted-skill");
        std::fs::create_dir_all(&plugin_dir).expect("mkdir");
        // skills_dir is the parent of plugin_dir.
        let perms = tmp.path().join("permissions.json");

        let mut p = SkillPermissionPolicy::for_source("trusted-skill", "clawhub");
        p.trust_level = 3; // full trust → None
        upsert_policy_file(&perms, &p).expect("write policy");

        let level = resolve_sandbox_level("trusted-skill", &plugin_dir);
        assert_eq!(level, zeus_aegis::SandboxLevel::None);

        // A skill present on disk but not in the policy file → still strictest.
        let other = resolve_sandbox_level("ghost-skill", &plugin_dir);
        assert_eq!(other, zeus_aegis::SandboxLevel::Paranoid);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_sandbox_command_passthrough_non_macos() {
        // On non-macOS, even a restrictive level must leave the command
        // unchanged (graceful no-op) — verified via the aegis seam directly.
        let sandbox = zeus_aegis::Sandbox::new(zeus_aegis::SandboxLevel::Paranoid);
        let line = "node index.js";
        assert_eq!(sandbox.sandbox_command(line), line);
    }
}
