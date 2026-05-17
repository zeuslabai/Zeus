//! Zeus Extensions - Deno extension runtime and OpenClaw compatibility
//!
//! Provides a runtime for loading and managing extensions:
//! - **Deno bridge** - subprocess management with JSON-RPC over stdin/stdout
//! - **Extension registry** - discovery, versioning, load/unload
//! - **OpenClaw compatibility** - adapter for OpenClaw extension interface
//! - **Permission system** - fine-grained network/fs/env access control

pub mod openclaw;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("extension not found: {0}")]
    NotFound(String),

    #[error("extension already exists: {0}")]
    AlreadyExists(String),

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("start failed: {0}")]
    StartFailed(String),

    #[error("stop failed: {0}")]
    StopFailed(String),

    #[error("JSON-RPC error: {0}")]
    JsonRpcError(String),

    #[error("import failed: {0}")]
    ImportFailed(String),

    #[error("invalid extension: {0}")]
    InvalidExtension(String),
}

// ---------------------------------------------------------------------------
// Extension source & status
// ---------------------------------------------------------------------------

/// Where an extension's code comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionSource {
    /// Local filesystem path to a .ts or .js file.
    Local(String),
    /// URL to download from.
    Url(String),
    /// Name in the OpenClaw extension registry.
    OpenClaw(String),
}

/// Runtime status of an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionStatus {
    Running,
    Stopped,
    Error(String),
    Starting,
    Stopping,
}

impl std::fmt::Display for ExtensionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionStatus::Running => write!(f, "running"),
            ExtensionStatus::Stopped => write!(f, "stopped"),
            ExtensionStatus::Error(e) => write!(f, "error: {e}"),
            ExtensionStatus::Starting => write!(f, "starting"),
            ExtensionStatus::Stopping => write!(f, "stopping"),
        }
    }
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// Permissions granted to an extension.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionPermissions {
    /// Hosts the extension can connect to.
    pub allow_net: Vec<String>,
    /// Paths the extension can read from.
    pub allow_read: Vec<String>,
    /// Paths the extension can write to.
    pub allow_write: Vec<String>,
    /// Environment variables the extension can access.
    pub allow_env: Vec<String>,
}

impl ExtensionPermissions {
    /// Create permissions that allow everything (for trusted extensions).
    pub fn allow_all() -> Self {
        Self {
            allow_net: vec!["*".to_string()],
            allow_read: vec!["/".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec!["*".to_string()],
        }
    }

    /// Build Deno CLI permission flags from this config.
    pub fn to_deno_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();

        if !self.allow_net.is_empty() {
            if self.allow_net.iter().any(|h| h == "*") {
                flags.push("--allow-net".to_string());
            } else {
                flags.push(format!("--allow-net={}", self.allow_net.join(",")));
            }
        }

        if !self.allow_read.is_empty() {
            if self.allow_read.iter().any(|p| p == "/") {
                flags.push("--allow-read".to_string());
            } else {
                flags.push(format!("--allow-read={}", self.allow_read.join(",")));
            }
        }

        if !self.allow_write.is_empty() {
            flags.push(format!("--allow-write={}", self.allow_write.join(",")));
        }

        if !self.allow_env.is_empty() {
            if self.allow_env.iter().any(|e| e == "*") {
                flags.push("--allow-env".to_string());
            } else {
                flags.push(format!("--allow-env={}", self.allow_env.join(",")));
            }
        }

        flags
    }
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

/// A registered extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: ExtensionSource,
    pub status: ExtensionStatus,
    pub permissions: ExtensionPermissions,
    pub created_at: DateTime<Utc>,
    pub logs: Vec<LogEntry>,
}

impl Extension {
    pub fn new(name: impl Into<String>, source: ExtensionSource) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            version: "0.1.0".to_string(),
            source,
            status: ExtensionStatus::Stopped,
            permissions: ExtensionPermissions::default(),
            created_at: Utc::now(),
            logs: Vec::new(),
        }
    }

    pub fn with_permissions(mut self, permissions: ExtensionPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn add_log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push(LogEntry {
            timestamp: Utc::now(),
            level,
            message: message.into(),
        });
        // Keep last 500 log entries
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }
}

/// A log entry from an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
}

/// Log severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC types (for Deno bridge communication)
// ---------------------------------------------------------------------------

/// A JSON-RPC request sent to the Deno subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC response from the Deno subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcErrorData>,
}

/// Error data in a JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorData {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Extension Registry
// ---------------------------------------------------------------------------

/// Handle for a running extension subprocess.
struct ProcessHandle {
    child: Child,
    /// Tokio task that reads stdout/stderr
    _log_task: tokio::task::JoinHandle<()>,
}

/// Manages the lifecycle of extensions.
pub struct ExtensionRegistry {
    extensions: Arc<Mutex<HashMap<String, Extension>>>,
    /// Running subprocess handles (id -> process)
    processes: Arc<Mutex<HashMap<String, ProcessHandle>>>,
    deno_path: String,
    openclaw_base: PathBuf,
    /// Path to the OpenClaw bridge script (openclaw_bridge.ts).
    bridge_path: Option<PathBuf>,
}

impl ExtensionRegistry {
    /// Create a new registry with the default Deno path.
    pub fn new() -> Self {
        let openclaw_base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("openclaw")
            .join("extensions");

        // Look for the bridge script relative to the binary or in known locations
        let bridge_path = Self::find_bridge_script();

        Self {
            extensions: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            deno_path: "deno".to_string(),
            openclaw_base,
            bridge_path,
        }
    }

    /// Locate the OpenClaw bridge script.
    fn find_bridge_script() -> Option<PathBuf> {
        // Check next to the binary
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let candidate = dir.join("openclaw_bridge.ts");
            if candidate.exists() {
                return Some(candidate);
            }
            // Check in share/zeus/
            let candidate = dir.join("share").join("zeus").join("openclaw_bridge.ts");
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // Check ~/.zeus/bridge/
        if let Some(home) = dirs::home_dir() {
            let candidate = home.join(".zeus").join("bridge").join("openclaw_bridge.ts");
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }

    /// Set the bridge script path explicitly.
    pub fn with_bridge_path(mut self, path: PathBuf) -> Self {
        self.bridge_path = Some(path);
        self
    }

    /// Create with a custom Deno binary path.
    pub fn with_deno_path(mut self, path: impl Into<String>) -> Self {
        self.deno_path = path.into();
        self
    }

    /// Create with a custom OpenClaw extensions directory.
    pub fn with_openclaw_base(mut self, path: PathBuf) -> Self {
        self.openclaw_base = path;
        self
    }

    // -- CRUD operations ----------------------------------------------------

    /// Register a new extension.
    pub async fn register(&self, extension: Extension) -> Result<Extension, ExtensionError> {
        let mut extensions = self.extensions.lock().await;
        if extensions.values().any(|e| e.name == extension.name) {
            return Err(ExtensionError::AlreadyExists(extension.name.clone()));
        }
        extensions.insert(extension.id.clone(), extension.clone());
        Ok(extension)
    }

    /// Get an extension by ID.
    pub async fn get(&self, id: &str) -> Result<Extension, ExtensionError> {
        self.extensions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ExtensionError::NotFound(id.to_string()))
    }

    /// List all extensions.
    pub async fn list(&self) -> Vec<Extension> {
        self.extensions.lock().await.values().cloned().collect()
    }

    /// Update an extension's config/permissions.
    pub async fn update(&self, extension: Extension) -> Result<(), ExtensionError> {
        let mut extensions = self.extensions.lock().await;
        if !extensions.contains_key(&extension.id) {
            return Err(ExtensionError::NotFound(extension.id.clone()));
        }
        extensions.insert(extension.id.clone(), extension);
        Ok(())
    }

    /// Uninstall (remove) an extension.
    pub async fn uninstall(&self, id: &str) -> Result<(), ExtensionError> {
        let mut extensions = self.extensions.lock().await;
        let ext = extensions
            .get(id)
            .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?;
        if ext.status == ExtensionStatus::Running {
            return Err(ExtensionError::RuntimeError(
                "cannot uninstall running extension; stop it first".to_string(),
            ));
        }
        extensions.remove(id);
        Ok(())
    }

    /// Return count of registered extensions.
    pub async fn count(&self) -> usize {
        self.extensions.lock().await.len()
    }

    // -- Lifecycle ----------------------------------------------------------

    /// Start an extension (spawn Deno subprocess).
    pub async fn start(&self, id: &str) -> Result<(), ExtensionError> {
        // Check if already running
        {
            let procs = self.processes.lock().await;
            if procs.contains_key(id) {
                return Ok(());
            }
        }

        let (source_path, deno_flags, ext_name, use_bridge) = {
            let mut extensions = self.extensions.lock().await;
            let ext = extensions
                .get_mut(id)
                .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?;

            if ext.status == ExtensionStatus::Running {
                return Ok(());
            }

            // Resolve source path and whether to use the bridge
            let (source_path, use_bridge) = match &ext.source {
                ExtensionSource::Local(path) => (path.clone(), false),
                ExtensionSource::Url(url) => {
                    ext.add_log(LogLevel::Info, format!("fetching from {url}"));
                    return Err(ExtensionError::StartFailed(
                        "URL source not yet implemented; use local path".to_string(),
                    ));
                }
                ExtensionSource::OpenClaw(name) => {
                    let ext_dir = self.openclaw_base.join(name);
                    let index = ext_dir.join("index.ts");
                    if !index.exists() {
                        return Err(ExtensionError::StartFailed(format!(
                            "OpenClaw extension not found at {}",
                            index.display()
                        )));
                    }
                    // OpenClaw extensions use the bridge script
                    (ext_dir.to_string_lossy().to_string(), true)
                }
            };

            if !use_bridge && !std::path::Path::new(&source_path).exists() {
                return Err(ExtensionError::StartFailed(format!(
                    "source file not found: {source_path}"
                )));
            }

            ext.status = ExtensionStatus::Starting;
            ext.add_log(
                LogLevel::Info,
                format!("starting extension from {source_path}"),
            );

            let flags = ext.permissions.to_deno_flags();
            (source_path, flags, ext.name.clone(), use_bridge)
        };

        // Spawn Deno subprocess
        let mut cmd = Command::new(&self.deno_path);
        cmd.arg("run");
        for flag in &deno_flags {
            cmd.arg(flag);
        }

        if use_bridge {
            // OpenClaw extensions run through the bridge shim
            if let Some(ref bridge) = self.bridge_path {
                cmd.arg(bridge.to_string_lossy().as_ref());
                cmd.arg(&source_path); // extension directory as arg
            } else {
                return Err(ExtensionError::StartFailed(
                    "OpenClaw bridge script not found; set bridge_path or install openclaw_bridge.ts".to_string(),
                ));
            }
        } else {
            cmd.arg(&source_path);
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ExtensionError::StartFailed(format!("failed to spawn Deno ({}): {}", self.deno_path, e))
        })?;

        // Capture stderr for logging
        let stderr = child.stderr.take();
        let ext_id = id.to_string();
        let extensions_ref = self.extensions.clone();
        let log_task = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Log stderr lines to extension log
                    if let Ok(mut exts) = extensions_ref.try_lock()
                        && let Some(ext) = exts.get_mut(&ext_id)
                    {
                        let level = if line.contains("error") || line.contains("Error") {
                            LogLevel::Error
                        } else if line.contains("warn") {
                            LogLevel::Warn
                        } else {
                            LogLevel::Debug
                        };
                        ext.add_log(level, &line);
                    }
                }
            }
        });

        info!("Extension '{}' started (pid: {:?})", ext_name, child.id());

        // Store process handle
        {
            let mut procs = self.processes.lock().await;
            procs.insert(
                id.to_string(),
                ProcessHandle {
                    child,
                    _log_task: log_task,
                },
            );
        }

        // Mark as running
        {
            let mut extensions = self.extensions.lock().await;
            if let Some(ext) = extensions.get_mut(id) {
                ext.status = ExtensionStatus::Running;
                ext.add_log(LogLevel::Info, "extension started");
            }
        }

        Ok(())
    }

    /// Stop a running extension.
    pub async fn stop(&self, id: &str) -> Result<(), ExtensionError> {
        // Kill the subprocess
        {
            let mut procs = self.processes.lock().await;
            if let Some(mut handle) = procs.remove(id) {
                info!(
                    "Stopping extension subprocess (pid: {:?})",
                    handle.child.id()
                );
                // Try graceful shutdown first
                let _ = handle.child.kill().await;
                handle._log_task.abort();
            }
        }

        // Update status
        {
            let mut extensions = self.extensions.lock().await;
            if let Some(ext) = extensions.get_mut(id) {
                if ext.status == ExtensionStatus::Stopped {
                    return Ok(());
                }
                ext.status = ExtensionStatus::Stopped;
                ext.add_log(LogLevel::Info, "extension stopped");
            }
        }

        Ok(())
    }

    /// Send a JSON-RPC request to a running extension and get the response.
    pub async fn send_rpc(
        &self,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ExtensionError> {
        let mut procs = self.processes.lock().await;
        let handle = procs
            .get_mut(id)
            .ok_or_else(|| ExtensionError::RuntimeError("extension not running".to_string()))?;

        let stdin = handle
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| ExtensionError::RuntimeError("stdin not available".to_string()))?;

        let request = JsonRpcRequest::new(1, method, params);
        let mut request_bytes = serde_json::to_vec(&request)
            .map_err(|e| ExtensionError::JsonRpcError(e.to_string()))?;
        request_bytes.push(b'\n');

        stdin.write_all(&request_bytes).await.map_err(|e| {
            ExtensionError::JsonRpcError(format!("failed to write to stdin: {}", e))
        })?;
        stdin
            .flush()
            .await
            .map_err(|e| ExtensionError::JsonRpcError(format!("failed to flush stdin: {}", e)))?;

        // Read response from stdout
        let stdout = handle
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| ExtensionError::RuntimeError("stdout not available".to_string()))?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            reader.read_line(&mut line),
        )
        .await
        .map_err(|_| ExtensionError::JsonRpcError("RPC response timed out (30s)".to_string()))?
        .map_err(|e| ExtensionError::JsonRpcError(format!("failed to read response: {}", e)))?;

        let response: JsonRpcResponse = serde_json::from_str(&line).map_err(|e| {
            ExtensionError::JsonRpcError(format!("invalid JSON-RPC response: {}", e))
        })?;

        if let Some(err) = response.error {
            return Err(ExtensionError::JsonRpcError(err.message));
        }

        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }

    /// Check if an extension process is still alive.
    pub async fn is_alive(&self, id: &str) -> bool {
        let mut procs = self.processes.lock().await;
        if let Some(handle) = procs.get_mut(id) {
            match handle.child.try_wait() {
                Ok(None) => true, // Still running
                Ok(Some(status)) => {
                    warn!("Extension {} exited with status: {}", id, status);
                    false
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Get logs for an extension.
    pub async fn logs(&self, id: &str) -> Result<Vec<LogEntry>, ExtensionError> {
        let extensions = self.extensions.lock().await;
        let ext = extensions
            .get(id)
            .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?;
        Ok(ext.logs.clone())
    }

    // -- OpenClaw import ----------------------------------------------------

    /// Import an extension from the OpenClaw extensions directory.
    pub async fn import_openclaw(&self, extension_name: &str) -> Result<Extension, ExtensionError> {
        let ext_dir = self.openclaw_base.join(extension_name);
        if !ext_dir.exists() {
            return Err(ExtensionError::ImportFailed(format!(
                "OpenClaw extension directory not found: {}",
                ext_dir.display()
            )));
        }

        let index_path = ext_dir.join("index.ts");
        if !index_path.exists() {
            return Err(ExtensionError::ImportFailed(format!(
                "no index.ts found in {}",
                ext_dir.display()
            )));
        }

        let ext = Extension::new(
            extension_name,
            ExtensionSource::OpenClaw(extension_name.to_string()),
        );

        self.register(ext).await
    }

    /// Deno binary path.
    pub fn deno_path(&self) -> &str {
        &self.deno_path
    }

    /// OpenClaw extensions base directory.
    pub fn openclaw_base(&self) -> &PathBuf {
        &self.openclaw_base
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- ExtensionSource tests ----------------------------------------------

    #[test]
    fn test_source_local() {
        let src = ExtensionSource::Local("/path/to/ext.ts".to_string());
        let json = serde_json::to_string(&src).expect("should serialize to JSON");
        let de: ExtensionSource = serde_json::from_str(&json).expect("should parse successfully");
        assert!(matches!(de, ExtensionSource::Local(p) if p == "/path/to/ext.ts"));
    }

    #[test]
    fn test_source_url() {
        let src = ExtensionSource::Url("https://example.com/ext.ts".to_string());
        let json = serde_json::to_string(&src).expect("should serialize to JSON");
        assert!(json.contains("https://example.com/ext.ts"));
    }

    #[test]
    fn test_source_openclaw() {
        let src = ExtensionSource::OpenClaw("discord".to_string());
        let json = serde_json::to_string(&src).expect("should serialize to JSON");
        assert!(json.contains("discord"));
    }

    // -- ExtensionStatus tests ----------------------------------------------

    #[test]
    fn test_status_display() {
        assert_eq!(ExtensionStatus::Running.to_string(), "running");
        assert_eq!(ExtensionStatus::Stopped.to_string(), "stopped");
        assert_eq!(
            ExtensionStatus::Error("oops".into()).to_string(),
            "error: oops"
        );
        assert_eq!(ExtensionStatus::Starting.to_string(), "starting");
        assert_eq!(ExtensionStatus::Stopping.to_string(), "stopping");
    }

    #[test]
    fn test_status_equality() {
        assert_eq!(ExtensionStatus::Running, ExtensionStatus::Running);
        assert_ne!(ExtensionStatus::Running, ExtensionStatus::Stopped);
    }

    #[test]
    fn test_status_serialization() {
        let json =
            serde_json::to_string(&ExtensionStatus::Running).expect("should serialize to JSON");
        assert_eq!(json, "\"running\"");
    }

    // -- ExtensionPermissions tests -----------------------------------------

    #[test]
    fn test_permissions_default() {
        let perms = ExtensionPermissions::default();
        assert!(perms.allow_net.is_empty());
        assert!(perms.allow_read.is_empty());
        assert!(perms.allow_write.is_empty());
        assert!(perms.allow_env.is_empty());
    }

    #[test]
    fn test_permissions_allow_all() {
        let perms = ExtensionPermissions::allow_all();
        assert!(perms.allow_net.contains(&"*".to_string()));
        assert!(perms.allow_read.contains(&"/".to_string()));
        assert!(perms.allow_env.contains(&"*".to_string()));
    }

    #[test]
    fn test_permissions_to_deno_flags_empty() {
        let perms = ExtensionPermissions::default();
        let flags = perms.to_deno_flags();
        assert!(flags.is_empty());
    }

    #[test]
    fn test_permissions_to_deno_flags_wildcard() {
        let perms = ExtensionPermissions::allow_all();
        let flags = perms.to_deno_flags();
        assert!(flags.contains(&"--allow-net".to_string()));
        assert!(flags.contains(&"--allow-read".to_string()));
        assert!(flags.contains(&"--allow-env".to_string()));
    }

    #[test]
    fn test_permissions_to_deno_flags_specific() {
        let perms = ExtensionPermissions {
            allow_net: vec!["api.example.com".to_string(), "localhost:8080".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp/out".to_string()],
            allow_env: vec!["HOME".to_string(), "PATH".to_string()],
        };
        let flags = perms.to_deno_flags();
        assert!(flags.iter().any(|f| f.starts_with("--allow-net=")));
        assert!(flags.iter().any(|f| f.starts_with("--allow-read=")));
        assert!(flags.iter().any(|f| f.starts_with("--allow-write=")));
        assert!(flags.iter().any(|f| f.starts_with("--allow-env=")));
    }

    #[test]
    fn test_permissions_serialization() {
        let perms = ExtensionPermissions {
            allow_net: vec!["example.com".to_string()],
            allow_read: vec![],
            allow_write: vec![],
            allow_env: vec![],
        };
        let json = serde_json::to_string(&perms).expect("should serialize to JSON");
        let de: ExtensionPermissions =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.allow_net.len(), 1);
    }

    // -- Extension tests ----------------------------------------------------

    #[test]
    fn test_extension_new() {
        let ext = Extension::new("test-ext", ExtensionSource::Local("/tmp/test.ts".into()));
        assert_eq!(ext.name, "test-ext");
        assert_eq!(ext.version, "0.1.0");
        assert_eq!(ext.status, ExtensionStatus::Stopped);
        assert!(ext.logs.is_empty());
    }

    #[test]
    fn test_extension_builder() {
        let ext = Extension::new("builder", ExtensionSource::Url("https://x.com/e.ts".into()))
            .with_permissions(ExtensionPermissions::allow_all())
            .with_version("1.0.0");
        assert_eq!(ext.version, "1.0.0");
        assert!(ext.permissions.allow_net.contains(&"*".to_string()));
    }

    #[test]
    fn test_extension_add_log() {
        let mut ext = Extension::new("log-test", ExtensionSource::Local("/tmp/t.ts".into()));
        ext.add_log(LogLevel::Info, "started");
        ext.add_log(LogLevel::Error, "oops");
        assert_eq!(ext.logs.len(), 2);
        assert_eq!(ext.logs[0].level, LogLevel::Info);
        assert_eq!(ext.logs[1].message, "oops");
    }

    #[test]
    fn test_extension_serialization() {
        let ext = Extension::new("ser", ExtensionSource::Local("/tmp/s.ts".into()));
        let json = serde_json::to_string(&ext).expect("should serialize to JSON");
        let de: Extension = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "ser");
        assert_eq!(de.status, ExtensionStatus::Stopped);
    }

    // -- LogLevel tests -----------------------------------------------------

    #[test]
    fn test_log_level_display() {
        assert_eq!(LogLevel::Debug.to_string(), "debug");
        assert_eq!(LogLevel::Info.to_string(), "info");
        assert_eq!(LogLevel::Warn.to_string(), "warn");
        assert_eq!(LogLevel::Error.to_string(), "error");
    }

    #[test]
    fn test_log_level_serialization() {
        let json = serde_json::to_string(&LogLevel::Warn).expect("should serialize to JSON");
        assert_eq!(json, "\"warn\"");
        let de: LogLevel = serde_json::from_str("\"error\"").expect("should parse successfully");
        assert_eq!(de, LogLevel::Error);
    }

    // -- LogEntry tests -----------------------------------------------------

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            message: "test message".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("should serialize to JSON");
        let de: LogEntry = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.message, "test message");
    }

    // -- JSON-RPC tests -----------------------------------------------------

    #[test]
    fn test_jsonrpc_request() {
        let req = JsonRpcRequest::new(1, "ping", serde_json::json!({}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, 1);
        assert_eq!(req.method, "ping");
    }

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest::new(42, "hello", serde_json::json!({"name": "Zeus"}));
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: JsonRpcRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, 42);
        assert_eq!(de.method, "hello");
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: 1,
            result: Some(serde_json::json!({"status": "ok"})),
            error: None,
        };
        let json = serde_json::to_string(&resp).expect("should serialize to JSON");
        let de: JsonRpcResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert!(de.result.is_some());
        assert!(de.error.is_none());
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: 1,
            result: None,
            error: Some(JsonRpcErrorData {
                code: -32600,
                message: "invalid request".to_string(),
                data: None,
            }),
        };
        let json = serde_json::to_string(&resp).expect("should serialize to JSON");
        let de: JsonRpcResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert!(de.error.is_some());
        assert_eq!(de.error.expect("operation should succeed").code, -32600);
    }

    // -- ExtensionRegistry tests --------------------------------------------

    #[tokio::test]
    async fn test_registry_creation() {
        let reg = ExtensionRegistry::new();
        assert_eq!(reg.count().await, 0);
        assert_eq!(reg.deno_path(), "deno");
    }

    #[tokio::test]
    async fn test_registry_default() {
        let reg = ExtensionRegistry::default();
        assert_eq!(reg.count().await, 0);
    }

    #[tokio::test]
    async fn test_registry_custom_deno() {
        let reg = ExtensionRegistry::new().with_deno_path("/usr/local/bin/deno");
        assert_eq!(reg.deno_path(), "/usr/local/bin/deno");
    }

    #[tokio::test]
    async fn test_register_extension() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("test", ExtensionSource::Local("/tmp/t.ts".into()));
        let registered = reg
            .register(ext)
            .await
            .expect("async operation should succeed");
        assert_eq!(registered.name, "test");
        assert_eq!(reg.count().await, 1);
    }

    #[tokio::test]
    async fn test_register_duplicate_error() {
        let reg = ExtensionRegistry::new();
        let ext1 = Extension::new("dup", ExtensionSource::Local("/tmp/1.ts".into()));
        let ext2 = Extension::new("dup", ExtensionSource::Local("/tmp/2.ts".into()));
        reg.register(ext1)
            .await
            .expect("async operation should succeed");
        let err = reg.register(ext2).await.unwrap_err();
        assert!(matches!(err, ExtensionError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn test_get_extension() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("get-test", ExtensionSource::Local("/tmp/t.ts".into()));
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        let got = reg.get(&id).await.expect("async operation should succeed");
        assert_eq!(got.name, "get-test");
    }

    #[tokio::test]
    async fn test_get_missing_error() {
        let reg = ExtensionRegistry::new();
        let err = reg.get("nope").await.unwrap_err();
        assert!(matches!(err, ExtensionError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_list_extensions() {
        let reg = ExtensionRegistry::new();
        reg.register(Extension::new("a", ExtensionSource::Local("/a.ts".into())))
            .await
            .expect("Extension::new should succeed");
        reg.register(Extension::new("b", ExtensionSource::Local("/b.ts".into())))
            .await
            .expect("Extension::new should succeed");
        assert_eq!(reg.list().await.len(), 2);
    }

    #[tokio::test]
    async fn test_update_extension() {
        let reg = ExtensionRegistry::new();
        let mut ext = Extension::new("upd", ExtensionSource::Local("/tmp/u.ts".into()));
        let id = ext.id.clone();
        reg.register(ext.clone())
            .await
            .expect("async operation should succeed");

        ext.version = "2.0.0".to_string();
        reg.update(ext)
            .await
            .expect("async operation should succeed");

        let got = reg.get(&id).await.expect("async operation should succeed");
        assert_eq!(got.version, "2.0.0");
    }

    #[tokio::test]
    async fn test_update_missing_error() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("ghost", ExtensionSource::Local("/tmp/g.ts".into()));
        let err = reg.update(ext).await.unwrap_err();
        assert!(matches!(err, ExtensionError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_uninstall_extension() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("doomed", ExtensionSource::Local("/tmp/d.ts".into()));
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        reg.uninstall(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(reg.count().await, 0);
    }

    #[tokio::test]
    async fn test_uninstall_missing_error() {
        let reg = ExtensionRegistry::new();
        let err = reg.uninstall("nope").await.unwrap_err();
        assert!(matches!(err, ExtensionError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_start_missing_source() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new(
            "no-file",
            ExtensionSource::Local("/nonexistent/path/ext.ts".into()),
        );
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        let err = reg.start(&id).await.unwrap_err();
        assert!(matches!(err, ExtensionError::StartFailed(_)));
    }

    #[tokio::test]
    async fn test_start_url_not_implemented() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new(
            "url-ext",
            ExtensionSource::Url("https://example.com/ext.ts".into()),
        );
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        let err = reg.start(&id).await.unwrap_err();
        assert!(matches!(err, ExtensionError::StartFailed(_)));
    }

    #[tokio::test]
    async fn test_stop_already_stopped() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("stopped", ExtensionSource::Local("/tmp/s.ts".into()));
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        // Should be a no-op
        reg.stop(&id).await.expect("async operation should succeed");
    }

    #[tokio::test]
    async fn test_logs_empty() {
        let reg = ExtensionRegistry::new();
        let ext = Extension::new("log-test", ExtensionSource::Local("/tmp/l.ts".into()));
        let id = ext.id.clone();
        reg.register(ext)
            .await
            .expect("async operation should succeed");

        let logs = reg.logs(&id).await.expect("async operation should succeed");
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn test_import_openclaw_missing() {
        let reg = ExtensionRegistry::new().with_openclaw_base(PathBuf::from("/nonexistent/path"));
        let err = reg.import_openclaw("discord").await.unwrap_err();
        assert!(matches!(err, ExtensionError::ImportFailed(_)));
    }

    // -- Error display tests ------------------------------------------------

    #[test]
    fn test_error_display() {
        assert_eq!(
            ExtensionError::NotFound("x".into()).to_string(),
            "extension not found: x"
        );
        assert_eq!(
            ExtensionError::AlreadyExists("y".into()).to_string(),
            "extension already exists: y"
        );
        assert_eq!(
            ExtensionError::PermissionDenied("no net".into()).to_string(),
            "permission denied: no net"
        );
        assert_eq!(
            ExtensionError::JsonRpcError("parse error".into()).to_string(),
            "JSON-RPC error: parse error"
        );
    }
}
