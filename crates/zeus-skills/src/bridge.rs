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
    /// Spawn a child process and establish the bridge.
    pub async fn spawn(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
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
        let bridge = ProcessBridge::spawn("node", &[entry], Some(plugin_dir)).await?;
        Ok(Self {
            manifest,
            bridge: Mutex::new(bridge),
        })
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
        let bridge = ProcessBridge::spawn("python3", &[entry], Some(plugin_dir)).await?;
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
}
