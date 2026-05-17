//! MCP Client — connects to external MCP servers via JSON-RPC over stdio.
//!
//! Spawns an MCP server as a child process, communicates over stdin/stdout using
//! newline-delimited JSON-RPC 2.0 messages.
//!
//! Lifecycle: `McpClient::connect()` -> `initialize()` -> `list_tools()` / `call_tool()` -> `shutdown()`

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::{debug, warn};

use crate::protocol::{McpRequest, McpResponse, ToolDefinition};

/// Default timeout for MCP tool calls (30 seconds).
const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Default timeout for initialization (10 seconds).
const INIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration for connecting to an external MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientConfig {
    /// Unique identifier for this server connection.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Transport type (currently only "stdio" is supported).
    pub transport: String,
    /// Command to execute (e.g., "npx", "python3", "node").
    pub command: String,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the child process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Result of a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// Whether the call succeeded.
    pub success: bool,
    /// The tool output content (MCP content array).
    pub content: Value,
    /// Error message if the call failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
}

/// An active connection to an external MCP server process.
pub struct McpClient {
    config: McpClientConfig,
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    /// Cached tool list from the server (populated after `list_tools()`).
    tools: Vec<ToolDefinition>,
    /// Whether the server has been initialized.
    initialized: bool,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("id", &self.config.id)
            .field("name", &self.config.name)
            .field("initialized", &self.initialized)
            .field("tools_count", &self.tools.len())
            .finish()
    }
}

impl McpClient {
    /// Spawn the MCP server process and establish stdio communication.
    ///
    /// Does NOT initialize the MCP protocol — call `initialize()` after this.
    pub async fn connect(config: McpClientConfig) -> Result<Self, String> {
        if config.transport != "stdio" {
            return Err(format!(
                "Unsupported transport '{}'; only 'stdio' is supported",
                config.transport
            ));
        }

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn MCP server '{}' (command: {} {}): {}",
                config.name,
                config.command,
                config.args.join(" "),
                e
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to capture child stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture child stdout".to_string())?;

        Ok(Self {
            config,
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            tools: Vec::new(),
            initialized: false,
        })
    }

    /// Send the MCP `initialize` handshake.
    pub async fn initialize(&mut self) -> Result<Value, String> {
        let request = McpRequest::new(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "zeus",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        );

        let response = self.send_request(request, INIT_TIMEOUT).await?;

        if let Some(error) = response.error {
            return Err(format!(
                "MCP initialize failed: {} (code {})",
                error.message, error.code
            ));
        }

        self.initialized = true;

        // Send initialized notification (no id, no response expected)
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        // Best-effort send — ignore errors on the notification
        let _ = self.send_raw(&notification).await;

        debug!("MCP server '{}' initialized successfully", self.config.name);
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// List tools available on the MCP server. Caches the result.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolDefinition>, String> {
        if !self.initialized {
            return Err("MCP client not initialized; call initialize() first".to_string());
        }

        let request = McpRequest::new("tools/list", json!({}));
        let response = self.send_request(request, INIT_TIMEOUT).await?;

        if let Some(error) = response.error {
            return Err(format!(
                "tools/list failed: {} (code {})",
                error.message, error.code
            ));
        }

        let result = response.result.unwrap_or(json!({"tools": []}));
        let tools: Vec<ToolDefinition> = result
            .get("tools")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();

        self.tools = tools.clone();
        Ok(tools)
    }

    /// Call a tool on the MCP server by name with given arguments.
    ///
    /// Returns a `ToolCallResult` with timing information.
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, String> {
        if !self.initialized {
            return Err("MCP client not initialized; call initialize() first".to_string());
        }

        let start = Instant::now();

        let request = McpRequest::new(
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        );

        let response = self.send_request(request, TOOL_CALL_TIMEOUT).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match response {
            Ok(resp) => {
                if let Some(error) = resp.error {
                    Ok(ToolCallResult {
                        success: false,
                        content: Value::Null,
                        error: Some(format!("{} (code {})", error.message, error.code)),
                        duration_ms,
                    })
                } else {
                    let result = resp.result.unwrap_or(Value::Null);
                    Ok(ToolCallResult {
                        success: true,
                        content: result,
                        error: None,
                        duration_ms,
                    })
                }
            }
            Err(e) => Ok(ToolCallResult {
                success: false,
                content: Value::Null,
                error: Some(e),
                duration_ms,
            }),
        }
    }

    /// Check if the server process is still running.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,     // still running
            Ok(Some(_)) => false, // exited
            Err(_) => false,
        }
    }

    /// Get the cached tool list (empty if `list_tools()` hasn't been called).
    pub fn cached_tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Find a tool by name in the cached tool list.
    pub fn find_tool(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// Get the client configuration.
    pub fn config(&self) -> &McpClientConfig {
        &self.config
    }

    /// Gracefully shut down the MCP server process.
    pub async fn shutdown(&mut self) -> Result<(), String> {
        // Try graceful close — dropping stdin signals EOF to the child process.
        // We do a best-effort flush before the child wait.
        let _ = self.stdin.flush().await;

        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(status)) => {
                debug!(
                    "MCP server '{}' exited with status: {}",
                    self.config.name, status
                );
                Ok(())
            }
            Ok(Err(e)) => {
                warn!("Error waiting for MCP server '{}': {}", self.config.name, e);
                Err(e.to_string())
            }
            Err(_) => {
                warn!(
                    "MCP server '{}' did not exit gracefully, killing",
                    self.config.name
                );
                self.child
                    .kill()
                    .await
                    .map_err(|e| format!("Failed to kill MCP server: {}", e))?;
                Ok(())
            }
        }
    }

    /// Send a JSON-RPC request and read the response with a timeout.
    async fn send_request(
        &mut self,
        request: McpRequest,
        timeout: Duration,
    ) -> Result<McpResponse, String> {
        let request_value =
            serde_json::to_value(&request).map_err(|e| format!("Serialize error: {}", e))?;

        self.send_raw(&request_value).await?;

        // Read response with timeout
        let mut line = String::new();
        let read_result = tokio::time::timeout(timeout, async {
            loop {
                line.clear();
                let bytes_read = self
                    .stdout
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("Failed to read from MCP server stdout: {}", e))?;

                if bytes_read == 0 {
                    return Err(
                        "MCP server closed stdout unexpectedly (process may have crashed)"
                            .to_string(),
                    );
                }

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue; // skip empty lines
                }

                // Try to parse as JSON-RPC response; skip notifications
                match serde_json::from_str::<McpResponse>(trimmed) {
                    Ok(resp) => {
                        // Only return responses that have an id matching our request
                        // (skip notifications which have no id)
                        if resp.id.is_some() {
                            return Ok(resp);
                        }
                        // Otherwise it's a notification — skip and keep reading
                        debug!("Skipping MCP notification: {}", trimmed);
                    }
                    Err(_) => {
                        // Could be a notification or other message — skip
                        debug!("Skipping non-response MCP message: {}", trimmed);
                    }
                }
            }
        })
        .await;

        match read_result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "MCP server '{}' timed out after {}ms",
                self.config.name,
                timeout.as_millis()
            )),
        }
    }

    /// Write a raw JSON value as a newline-delimited message to stdin.
    async fn send_raw(&mut self, value: &Value) -> Result<(), String> {
        let mut line = serde_json::to_string(value)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;
        line.push('\n');

        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to MCP server stdin: {}", e))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush MCP server stdin: {}", e))?;

        Ok(())
    }
}

/// Convenience function: connect, initialize, and list tools in one call.
///
/// Returns `(McpClient, Vec<ToolDefinition>)` on success.
pub async fn connect_and_discover(
    config: McpClientConfig,
) -> Result<(McpClient, Vec<ToolDefinition>), String> {
    let mut client = McpClient::connect(config).await?;
    client.initialize().await?;
    let tools = client.list_tools().await?;
    Ok((client, tools))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_serialization() {
        let config = McpClientConfig {
            id: "test-server".to_string(),
            name: "Test Server".to_string(),
            transport: "stdio".to_string(),
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: HashMap::from([("FOO".to_string(), "bar".to_string())]),
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let de: McpClientConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "test-server");
        assert_eq!(de.command, "node");
        assert_eq!(de.args, vec!["server.js"]);
        assert_eq!(de.env.get("FOO").expect("key should exist"), "bar");
    }

    #[test]
    fn test_client_config_default_fields() {
        let json = r#"{"id":"x","name":"X","transport":"stdio","command":"echo"}"#;
        let config: McpClientConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(config.args.is_empty());
        assert!(config.env.is_empty());
    }

    #[test]
    fn test_tool_call_result_success() {
        let result = ToolCallResult {
            success: true,
            content: json!({"content": [{"type": "text", "text": "hello"}]}),
            error: None,
            duration_ms: 42,
        };
        let json = serde_json::to_string(&result).expect("should serialize to JSON");
        assert!(json.contains("\"success\":true"));
        assert!(!json.contains("error"));
        assert!(json.contains("\"duration_ms\":42"));
    }

    #[test]
    fn test_tool_call_result_failure() {
        let result = ToolCallResult {
            success: false,
            content: Value::Null,
            error: Some("tool exploded".to_string()),
            duration_ms: 100,
        };
        let json = serde_json::to_string(&result).expect("should serialize to JSON");
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("tool exploded"));
    }

    #[tokio::test]
    async fn test_connect_unsupported_transport() {
        let config = McpClientConfig {
            id: "bad".to_string(),
            name: "Bad".to_string(),
            transport: "http".to_string(),
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        let result = McpClient::connect(config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported transport"));
    }

    #[tokio::test]
    async fn test_connect_nonexistent_command() {
        let config = McpClientConfig {
            id: "missing".to_string(),
            name: "Missing".to_string(),
            transport: "stdio".to_string(),
            command: "/usr/bin/does-not-exist-zeus-test-12345".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        let result = McpClient::connect(config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to spawn"));
    }

    #[tokio::test]
    async fn test_connect_and_communicate() {
        // Use a Python script that acts as a minimal MCP server
        let script = r#"
import sys, json

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except:
        continue

    method = req.get("method", "")
    req_id = req.get("id")

    if method == "initialize":
        resp = {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "test-server", "version": "0.1.0"},
                "capabilities": {"tools": {}}
            }
        }
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        resp = {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echoes input",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": {"type": "string"}
                            }
                        }
                    }
                ]
            }
        }
    elif method == "tools/call":
        name = req.get("params", {}).get("name", "")
        args = req.get("params", {}).get("arguments", {})
        if name == "echo":
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": args.get("message", "")}]
                }
            }
        else:
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32000, "message": f"Unknown tool: {name}"}
            }
    else:
        resp = {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"Unknown method: {method}"}
        }

    print(json.dumps(resp), flush=True)
"#;

        // Write script to temp file
        let tmp = tempfile::NamedTempFile::new().expect("NamedTempFile::new should succeed");
        std::fs::write(tmp.path(), script).expect("should write file");

        let config = McpClientConfig {
            id: "test".to_string(),
            name: "Test MCP Server".to_string(),
            transport: "stdio".to_string(),
            command: "python3".to_string(),
            args: vec![
                tmp.path()
                    .to_str()
                    .expect("to_str should succeed")
                    .to_string(),
            ],
            env: HashMap::new(),
        };

        let result = McpClient::connect(config).await;

        match result {
            Ok(mut client) => {
                // Initialize
                let init_result = client.initialize().await;
                assert!(init_result.is_ok(), "Init failed: {:?}", init_result);

                // List tools
                let tools = client
                    .list_tools()
                    .await
                    .expect("async operation should succeed");
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "echo");

                // Find tool
                assert!(client.find_tool("echo").is_some());
                assert!(client.find_tool("nonexistent").is_none());

                // Call tool successfully
                let result = client
                    .call_tool("echo", json!({"message": "hello zeus"}))
                    .await
                    .expect("async operation should succeed");
                assert!(result.success);
                assert!(result.duration_ms < 10_000);
                let text = result.content["content"][0]["text"]
                    .as_str()
                    .expect("should be a string");
                assert_eq!(text, "hello zeus");

                // Call unknown tool
                let result = client
                    .call_tool("nonexistent", json!({}))
                    .await
                    .expect("async operation should succeed");
                assert!(!result.success);
                assert!(result.error.is_some());
                assert!(
                    result
                        .error
                        .expect("operation should succeed")
                        .contains("Unknown tool")
                );

                // Alive check
                assert!(client.is_alive());

                // Shutdown
                client
                    .shutdown()
                    .await
                    .expect("async operation should succeed");
            }
            Err(e) => {
                // python3 not available -- acceptable in CI
                eprintln!("Skipping MCP client test: {}", e);
            }
        }
    }
}
