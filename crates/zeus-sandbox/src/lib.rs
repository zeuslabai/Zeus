//! Zeus Sandbox - WASM sandbox with capability-based security
//!
//! Provides sandboxed code execution using Wasmtime + WASI with:
//! - **Capability-based policies** - fine-grained fs/net/env permissions
//! - **Resource limits** - memory ceiling, CPU time, wall-clock timeout
//! - **Execution tracking** - history with stdout/stderr capture
//! - **Policy management** - CRUD for reusable sandbox policies
//! - **Exec approval flow** - command authorization with auto-approve patterns

pub mod approvals;
pub mod path_validator;
pub mod resource_monitor;

pub use approvals::{
    ApprovalOutcome, ApprovalPolicy, ApprovalRequest, ApprovalStatus, ExecApprovalManager,
};
pub use path_validator::{PathValidationError, validate_path_in_root, validate_write_path};
pub use resource_monitor::{
    BudgetViolation, ResourceBudget, ResourceMonitor, ResourceSnapshot, ResourceType, UsageReport,
};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::preview1::{self, WasiP1Ctx};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during sandbox operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("policy not found: {0}")]
    PolicyNotFound(String),

    #[error("policy already exists: {0}")]
    PolicyAlreadyExists(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("memory limit exceeded: {limit_mb}MB")]
    MemoryLimitExceeded { limit_mb: u64 },

    #[error("CPU time limit exceeded: {limit_seconds}s")]
    CpuTimeLimitExceeded { limit_seconds: u64 },

    #[error("wall clock timeout: {limit_seconds}s")]
    WallClockTimeout { limit_seconds: u64 },

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid WASM module: {0}")]
    InvalidModule(String),

    #[error("compilation error: {0}")]
    CompilationError(String),

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("approval request not found: {0}")]
    RequestNotFound(String),
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// Fine-grained capability grants for a sandbox policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    /// Paths the WASM module can read from.
    pub fs_read: Vec<String>,
    /// Paths the WASM module can write to.
    pub fs_write: Vec<String>,
    /// Hosts the WASM module can connect to (e.g. "api.example.com:443").
    pub net: Vec<String>,
    /// Environment variables the WASM module can access.
    pub env: Vec<String>,
    /// Environment variable passthrough rules (alternative to exact `env` list)
    #[serde(default)]
    pub env_passthrough: Option<EnvPassthrough>,
}

/// Controls which environment variables are passed into the sandbox.
///
/// Uses allowlist/blocklist pattern matching. Blocklist takes precedence.
/// Supports glob-style patterns: `*` matches everything, `AWS_*` matches
/// any var starting with `AWS_`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvPassthrough {
    /// Patterns for env vars to allow (e.g. ["HOME", "PATH", "AWS_*"])
    #[serde(default)]
    pub allow: Vec<String>,
    /// Patterns for env vars to block (e.g. ["*_SECRET", "*_KEY", "*_TOKEN"])
    /// Blocklist takes precedence over allowlist.
    #[serde(default)]
    pub block: Vec<String>,
}

impl EnvPassthrough {
    /// Check if a variable name passes through the allowlist/blocklist filter.
    ///
    /// Rules:
    /// - If `allow` is empty, nothing passes through.
    /// - A var must match at least one allow pattern.
    /// - If it matches any block pattern, it is rejected (block wins).
    pub fn is_allowed(&self, var_name: &str) -> bool {
        if self.allow.is_empty() {
            return false;
        }

        // Check blocklist first -- block always wins
        for pattern in &self.block {
            if Self::matches_pattern(pattern, var_name) {
                return false;
            }
        }

        // Check allowlist
        for pattern in &self.allow {
            if Self::matches_pattern(pattern, var_name) {
                return true;
            }
        }

        false
    }

    /// Collect all matching env vars from the current process environment.
    pub fn resolve(&self) -> HashMap<String, String> {
        std::env::vars()
            .filter(|(key, _)| self.is_allowed(key))
            .collect()
    }

    /// Simple glob-style pattern matching.
    ///
    /// Supports:
    /// - `*` matches everything
    /// - `PREFIX_*` matches any name starting with `PREFIX_`
    /// - `*_SUFFIX` matches any name ending with `_SUFFIX`
    /// - `PREFIX_*_SUFFIX` matches names starting with `PREFIX_` and ending with `_SUFFIX`
    /// - Exact match (no wildcards)
    fn matches_pattern(pattern: &str, name: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        match pattern.find('*') {
            None => {
                // Exact match
                pattern == name
            }
            Some(star_pos) => {
                let prefix = &pattern[..star_pos];
                let suffix = &pattern[star_pos + 1..];

                if suffix.contains('*') {
                    // Multiple wildcards: fall back to checking prefix of first *
                    // and suffix of last * (simple approach)
                    name.starts_with(prefix) && name.ends_with(suffix.trim_matches('*'))
                } else if suffix.is_empty() {
                    // Prefix wildcard: `AWS_*`
                    name.starts_with(prefix)
                } else if prefix.is_empty() {
                    // Suffix wildcard: `*_KEY`
                    name.ends_with(suffix)
                } else {
                    // Middle wildcard: `PREFIX_*_SUFFIX`
                    name.starts_with(prefix) && name.ends_with(suffix)
                }
            }
        }
    }
}

/// Resource limits for sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxLimits {
    /// Maximum memory in megabytes (default: 256).
    pub memory_mb: u64,
    /// Maximum CPU time in seconds (default: 30).
    pub cpu_seconds: u64,
    /// Maximum wall-clock time in seconds (default: 60).
    pub wall_clock_seconds: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            memory_mb: 256,
            cpu_seconds: 30,
            wall_clock_seconds: 60,
        }
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// A reusable sandbox execution policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub id: String,
    pub name: String,
    pub capabilities: SandboxCapabilities,
    pub limits: SandboxLimits,
    pub created_at: DateTime<Utc>,
}

impl SandboxPolicy {
    /// Create a new policy with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            capabilities: SandboxCapabilities::default(),
            limits: SandboxLimits::default(),
            created_at: Utc::now(),
        }
    }

    /// Create a restrictive policy with no capabilities.
    pub fn restrictive(name: impl Into<String>) -> Self {
        Self::new(name)
    }

    /// Create a permissive policy with broad capabilities.
    pub fn permissive(name: impl Into<String>) -> Self {
        let mut policy = Self::new(name);
        policy.capabilities.fs_read = vec!["/tmp".to_string()];
        policy.capabilities.fs_write = vec!["/tmp".to_string()];
        policy.capabilities.net = vec!["*".to_string()];
        policy
    }
}

// ---------------------------------------------------------------------------
// Execution language & status
// ---------------------------------------------------------------------------

/// Language of the code to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionLanguage {
    Wasm,
    TypeScript,
    JavaScript,
}

impl std::fmt::Display for ExecutionLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionLanguage::Wasm => write!(f, "wasm"),
            ExecutionLanguage::TypeScript => write!(f, "typescript"),
            ExecutionLanguage::JavaScript => write!(f, "javascript"),
        }
    }
}

/// Status of an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    TimedOut,
}

// ---------------------------------------------------------------------------
// Execution request / result
// ---------------------------------------------------------------------------

/// A request to execute code in the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    /// The code (or WASM bytes as base64) to execute.
    pub code: String,
    /// Language of the code.
    pub language: ExecutionLanguage,
    /// Optional policy ID to apply. Uses default restrictive policy if None.
    pub policy_id: Option<String>,
    /// Optional stdin data.
    pub stdin: Option<String>,
}

/// The result of a sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub id: String,
    pub policy_id: Option<String>,
    pub language: ExecutionLanguage,
    pub status: ExecutionStatus,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub memory_used_bytes: u64,
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Store data (holds WASI context + resource limits)
// ---------------------------------------------------------------------------

struct SandboxStoreData {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

// ---------------------------------------------------------------------------
// Sandbox engine
// ---------------------------------------------------------------------------

/// The main sandbox engine that manages policies and executes code.
pub struct SandboxEngine {
    engine: Engine,
    policies: Arc<Mutex<HashMap<String, SandboxPolicy>>>,
    executions: Arc<Mutex<Vec<ExecutionResult>>>,
}

impl SandboxEngine {
    /// Create a new sandbox engine.
    pub fn new() -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.wasm_component_model(false);

        let engine = Engine::new(&config)
            .map_err(|e| SandboxError::RuntimeError(format!("failed to create engine: {e}")))?;

        Ok(Self {
            engine,
            policies: Arc::new(Mutex::new(HashMap::new())),
            executions: Arc::new(Mutex::new(Vec::new())),
        })
    }

    // -- Policy management --------------------------------------------------

    /// Add a policy. Returns error if ID already exists.
    pub async fn add_policy(&self, policy: SandboxPolicy) -> Result<(), SandboxError> {
        let mut policies = self.policies.lock().await;
        if policies.contains_key(&policy.id) {
            return Err(SandboxError::PolicyAlreadyExists(policy.id.clone()));
        }
        policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    /// Get a policy by ID.
    pub async fn get_policy(&self, id: &str) -> Result<SandboxPolicy, SandboxError> {
        let policies = self.policies.lock().await;
        policies
            .get(id)
            .cloned()
            .ok_or_else(|| SandboxError::PolicyNotFound(id.to_string()))
    }

    /// List all policies.
    pub async fn list_policies(&self) -> Vec<SandboxPolicy> {
        let policies = self.policies.lock().await;
        policies.values().cloned().collect()
    }

    /// Update a policy. Returns error if not found.
    pub async fn update_policy(&self, policy: SandboxPolicy) -> Result<(), SandboxError> {
        let mut policies = self.policies.lock().await;
        if !policies.contains_key(&policy.id) {
            return Err(SandboxError::PolicyNotFound(policy.id.clone()));
        }
        policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    /// Delete a policy by ID.
    pub async fn delete_policy(&self, id: &str) -> Result<(), SandboxError> {
        let mut policies = self.policies.lock().await;
        policies
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| SandboxError::PolicyNotFound(id.to_string()))
    }

    /// Return count of policies.
    pub async fn policy_count(&self) -> usize {
        self.policies.lock().await.len()
    }

    // -- Execution ----------------------------------------------------------

    /// Execute WASM bytecode in the sandbox.
    pub async fn execute_wasm(
        &self,
        wasm_bytes: &[u8],
        policy: &SandboxPolicy,
        stdin_data: Option<&str>,
    ) -> Result<ExecutionResult, SandboxError> {
        let started_at = Utc::now();
        let start_instant = Instant::now();
        let exec_id = Uuid::new_v4().to_string();

        // Compile module
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::CompilationError(e.to_string()))?;

        // Build WASI context
        let mut wasi_builder = WasiCtxBuilder::new();

        // Apply capabilities: preopened dirs
        for path in &policy.capabilities.fs_read {
            if std::path::Path::new(path).exists() {
                let _ = wasi_builder.preopened_dir(
                    path,
                    path,
                    wasmtime_wasi::DirPerms::READ,
                    wasmtime_wasi::FilePerms::READ,
                );
            }
        }
        for path in &policy.capabilities.fs_write {
            if std::path::Path::new(path).exists() {
                let _ = wasi_builder.preopened_dir(
                    path,
                    path,
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                );
            }
        }

        // Env vars - use passthrough rules if set, otherwise fall back to exact list
        if let Some(ref passthrough) = policy.capabilities.env_passthrough {
            for (key, val) in passthrough.resolve() {
                wasi_builder.env(&key, &val);
            }
        } else {
            for var_name in &policy.capabilities.env {
                if let Ok(val) = std::env::var(var_name) {
                    wasi_builder.env(var_name, &val);
                }
            }
        }

        // Stdin
        if let Some(data) = stdin_data {
            wasi_builder.stdin(wasmtime_wasi::pipe::MemoryInputPipe::new(
                data.as_bytes().to_vec(),
            ));
        }

        // Capture stdout/stderr
        let stdout_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);
        let stderr_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);
        wasi_builder.stdout(stdout_pipe.clone());
        wasi_builder.stderr(stderr_pipe.clone());

        let wasi_ctx = wasi_builder.build_p1();

        // Build resource limits
        let limits = StoreLimitsBuilder::new()
            .memory_size(policy.limits.memory_mb as usize * 1024 * 1024)
            .table_elements(10000)
            .build();

        // Create store with fuel (CPU limiting) + epoch deadline (cooperative interruption)
        let fuel_amount = policy.limits.cpu_seconds * 1_000_000_000; // ~1B instructions per second
        let store_data = SandboxStoreData {
            wasi: wasi_ctx,
            limits,
        };
        let mut store = Store::new(&self.engine, store_data);
        store.limiter(|data| &mut data.limits);
        store
            .set_fuel(fuel_amount)
            .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

        // Epoch-based interruption: trap after 1 epoch tick from the watchdog thread
        store.set_epoch_deadline(1);

        // Spawn epoch watchdog — increments the engine epoch every second.
        // When the epoch advances past the store's deadline, Wasmtime traps
        // the running WASM, providing cooperative interruption even if the
        // module is stuck in a CPU-bound loop that doesn't consume fuel
        // quickly enough for the fuel limit to trigger.
        let watchdog_engine = self.engine.clone();
        let watchdog_seconds = policy.limits.wall_clock_seconds;
        let watchdog_handle = tokio::task::spawn_blocking(move || {
            for _ in 0..watchdog_seconds {
                std::thread::sleep(Duration::from_secs(1));
                watchdog_engine.increment_epoch();
            }
        });

        // Link WASI
        let mut linker: Linker<SandboxStoreData> = Linker::new(&self.engine);
        preview1::add_to_linker_sync(&mut linker, |data| &mut data.wasi)
            .map_err(|e| SandboxError::RuntimeError(format!("WASI link error: {e}")))?;

        // Instantiate with timeout
        let wall_timeout = Duration::from_secs(policy.limits.wall_clock_seconds);
        let (exit_code, status) = match tokio::time::timeout(wall_timeout, async move {
            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

            // Call _start (WASI convention)
            let start_fn = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .map_err(|e| SandboxError::ExecutionFailed(format!("no _start function: {e}")))?;

            match start_fn.call(&mut store, ()) {
                Ok(()) => Ok((0i32, ExecutionStatus::Completed)),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("fuel") {
                        Err(SandboxError::CpuTimeLimitExceeded {
                            limit_seconds: policy.limits.cpu_seconds,
                        })
                    } else if msg.contains("epoch") {
                        Err(SandboxError::WallClockTimeout {
                            limit_seconds: policy.limits.wall_clock_seconds,
                        })
                    } else {
                        Ok((1, ExecutionStatus::Failed))
                    }
                }
            }
        })
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(SandboxError::WallClockTimeout {
                    limit_seconds: policy.limits.wall_clock_seconds,
                });
            }
        };

        // Abort epoch watchdog — execution is done, no need to keep ticking
        watchdog_handle.abort();

        let duration_ms = start_instant.elapsed().as_millis() as u64;

        // Collect output
        let stdout_bytes: Vec<u8> = stdout_pipe.try_into_inner().unwrap_or_default().into();
        let stderr_bytes: Vec<u8> = stderr_pipe.try_into_inner().unwrap_or_default().into();

        let result = ExecutionResult {
            id: exec_id,
            policy_id: Some(policy.id.clone()),
            language: ExecutionLanguage::Wasm,
            status,
            stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
            stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
            exit_code,
            duration_ms,
            memory_used_bytes: 0,
            started_at,
        };

        // Store in history
        self.executions.lock().await.push(result.clone());

        Ok(result)
    }

    /// Execute code by dispatching to the appropriate runtime.
    pub async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, SandboxError> {
        // Resolve policy
        let policy = if let Some(ref pid) = request.policy_id {
            self.get_policy(pid).await?
        } else {
            SandboxPolicy::restrictive("default")
        };

        match request.language {
            ExecutionLanguage::Wasm => {
                // Decode base64 or use raw bytes
                let wasm_bytes = if request.code.starts_with("AGFzbQ") {
                    base64_decode(&request.code)
                        .map_err(|e| SandboxError::InvalidModule(e.to_string()))?
                } else {
                    request.code.as_bytes().to_vec()
                };
                self.execute_wasm(&wasm_bytes, &policy, request.stdin.as_deref())
                    .await
            }
            ExecutionLanguage::TypeScript | ExecutionLanguage::JavaScript => {
                // TypeScript/JavaScript: delegate to Deno subprocess (zeus-extensions handles this)
                Err(SandboxError::ExecutionFailed(
                    "TypeScript/JavaScript execution requires zeus-extensions Deno runtime"
                        .to_string(),
                ))
            }
        }
    }

    /// List execution history.
    pub async fn list_executions(&self) -> Vec<ExecutionResult> {
        self.executions.lock().await.clone()
    }

    /// Get a specific execution by ID.
    pub async fn get_execution(&self, id: &str) -> Option<ExecutionResult> {
        self.executions
            .lock()
            .await
            .iter()
            .find(|e| e.id == id)
            .cloned()
    }

    /// Return count of executions.
    pub async fn execution_count(&self) -> usize {
        self.executions.lock().await.len()
    }
}

impl Default for SandboxEngine {
    fn default() -> Self {
        Self::new().expect("failed to create default SandboxEngine")
    }
}

// ---------------------------------------------------------------------------
// Base64 helper
// ---------------------------------------------------------------------------

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    // Simple base64 decode without pulling in the base64 crate
    // We use a minimal implementation for WASM byte decoding
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.trim().as_bytes();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in input {
        if byte == b'=' {
            break;
        }
        if byte == b'\n' || byte == b'\r' || byte == b' ' {
            continue;
        }
        let val = table.iter().position(|&b| b == byte);
        match val {
            Some(v) => {
                buf = (buf << 6) | v as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    output.push((buf >> bits) as u8);
                    buf &= (1 << bits) - 1;
                }
            }
            None => return Err(format!("invalid base64 character: {}", byte as char)),
        }
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SandboxCapabilities tests ------------------------------------------

    #[test]
    fn test_capabilities_default() {
        let caps = SandboxCapabilities::default();
        assert!(caps.fs_read.is_empty());
        assert!(caps.fs_write.is_empty());
        assert!(caps.net.is_empty());
        assert!(caps.env.is_empty());
    }

    #[test]
    fn test_capabilities_serialization() {
        let caps = SandboxCapabilities {
            fs_read: vec!["/tmp".to_string()],
            fs_write: vec!["/tmp/out".to_string()],
            net: vec!["api.example.com:443".to_string()],
            env: vec!["HOME".to_string()],
            env_passthrough: None,
        };
        let json = serde_json::to_string(&caps).expect("should serialize to JSON");
        let de: SandboxCapabilities =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.fs_read.len(), 1);
        assert_eq!(de.fs_write.len(), 1);
        assert_eq!(de.net.len(), 1);
        assert_eq!(de.env.len(), 1);
    }

    // -- SandboxLimits tests ------------------------------------------------

    #[test]
    fn test_limits_default() {
        let limits = SandboxLimits::default();
        assert_eq!(limits.memory_mb, 256);
        assert_eq!(limits.cpu_seconds, 30);
        assert_eq!(limits.wall_clock_seconds, 60);
    }

    #[test]
    fn test_limits_serialization() {
        let limits = SandboxLimits {
            memory_mb: 128,
            cpu_seconds: 10,
            wall_clock_seconds: 30,
        };
        let json = serde_json::to_string(&limits).expect("should serialize to JSON");
        let de: SandboxLimits = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.memory_mb, 128);
        assert_eq!(de.cpu_seconds, 10);
        assert_eq!(de.wall_clock_seconds, 30);
    }

    #[test]
    fn test_limits_custom() {
        let limits = SandboxLimits {
            memory_mb: 512,
            cpu_seconds: 60,
            wall_clock_seconds: 120,
        };
        assert_eq!(limits.memory_mb, 512);
    }

    // -- SandboxPolicy tests ------------------------------------------------

    #[test]
    fn test_policy_new() {
        let policy = SandboxPolicy::new("test-policy");
        assert_eq!(policy.name, "test-policy");
        assert!(!policy.id.is_empty());
        assert!(policy.capabilities.fs_read.is_empty());
        assert_eq!(policy.limits.memory_mb, 256);
    }

    #[test]
    fn test_policy_restrictive() {
        let policy = SandboxPolicy::restrictive("strict");
        assert_eq!(policy.name, "strict");
        assert!(policy.capabilities.fs_read.is_empty());
        assert!(policy.capabilities.net.is_empty());
    }

    #[test]
    fn test_policy_permissive() {
        let policy = SandboxPolicy::permissive("open");
        assert_eq!(policy.name, "open");
        assert!(!policy.capabilities.fs_read.is_empty());
        assert!(!policy.capabilities.net.is_empty());
        assert_eq!(policy.capabilities.net[0], "*");
    }

    #[test]
    fn test_policy_serialization() {
        let policy = SandboxPolicy::new("ser-test");
        let json = serde_json::to_string(&policy).expect("should serialize to JSON");
        let de: SandboxPolicy = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "ser-test");
        assert_eq!(de.id, policy.id);
    }

    #[test]
    fn test_policy_unique_ids() {
        let p1 = SandboxPolicy::new("a");
        let p2 = SandboxPolicy::new("b");
        assert_ne!(p1.id, p2.id);
    }

    // -- ExecutionLanguage tests --------------------------------------------

    #[test]
    fn test_language_display() {
        assert_eq!(ExecutionLanguage::Wasm.to_string(), "wasm");
        assert_eq!(ExecutionLanguage::TypeScript.to_string(), "typescript");
        assert_eq!(ExecutionLanguage::JavaScript.to_string(), "javascript");
    }

    #[test]
    fn test_language_serialization() {
        let json =
            serde_json::to_string(&ExecutionLanguage::Wasm).expect("should serialize to JSON");
        assert_eq!(json, "\"wasm\"");
        let de: ExecutionLanguage =
            serde_json::from_str("\"typescript\"").expect("should parse successfully");
        assert_eq!(de, ExecutionLanguage::TypeScript);
    }

    // -- ExecutionStatus tests ----------------------------------------------

    #[test]
    fn test_status_serialization() {
        let json =
            serde_json::to_string(&ExecutionStatus::Completed).expect("should serialize to JSON");
        assert_eq!(json, "\"completed\"");
        let de: ExecutionStatus =
            serde_json::from_str("\"failed\"").expect("should parse successfully");
        assert_eq!(de, ExecutionStatus::Failed);
    }

    #[test]
    fn test_status_equality() {
        assert_eq!(ExecutionStatus::Running, ExecutionStatus::Running);
        assert_ne!(ExecutionStatus::Running, ExecutionStatus::Completed);
    }

    // -- ExecutionRequest tests ---------------------------------------------

    #[test]
    fn test_execution_request_serialization() {
        let req = ExecutionRequest {
            code: "print('hello')".to_string(),
            language: ExecutionLanguage::JavaScript,
            policy_id: Some("pol-1".to_string()),
            stdin: Some("input data".to_string()),
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: ExecutionRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.code, "print('hello')");
        assert_eq!(de.language, ExecutionLanguage::JavaScript);
        assert_eq!(de.policy_id.as_deref(), Some("pol-1"));
    }

    #[test]
    fn test_execution_request_minimal() {
        let json = r#"{"code":"test","language":"wasm"}"#;
        let req: ExecutionRequest = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(req.code, "test");
        assert!(req.policy_id.is_none());
        assert!(req.stdin.is_none());
    }

    // -- ExecutionResult tests ----------------------------------------------

    #[test]
    fn test_execution_result_serialization() {
        let result = ExecutionResult {
            id: "exec-1".to_string(),
            policy_id: Some("pol-1".to_string()),
            language: ExecutionLanguage::Wasm,
            status: ExecutionStatus::Completed,
            stdout: "hello world\n".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 42,
            memory_used_bytes: 1024,
            started_at: Utc::now(),
        };
        let json = serde_json::to_string(&result).expect("should serialize to JSON");
        let de: ExecutionResult = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "exec-1");
        assert_eq!(de.exit_code, 0);
        assert_eq!(de.duration_ms, 42);
    }

    // -- SandboxEngine tests ------------------------------------------------

    #[tokio::test]
    async fn test_engine_creation() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        assert_eq!(engine.policy_count().await, 0);
        assert_eq!(engine.execution_count().await, 0);
    }

    #[tokio::test]
    async fn test_engine_default() {
        let engine = SandboxEngine::default();
        assert_eq!(engine.policy_count().await, 0);
    }

    #[tokio::test]
    async fn test_add_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::new("test");
        engine
            .add_policy(policy)
            .await
            .expect("async operation should succeed");
        assert_eq!(engine.policy_count().await, 1);
    }

    #[tokio::test]
    async fn test_add_duplicate_policy_error() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let mut p1 = SandboxPolicy::new("test");
        let p2 = p1.clone();
        p1.name = "first".to_string();
        engine
            .add_policy(p1)
            .await
            .expect("async operation should succeed");
        let err = engine.add_policy(p2).await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyAlreadyExists(_)));
    }

    #[tokio::test]
    async fn test_get_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::new("my-policy");
        let id = policy.id.clone();
        engine
            .add_policy(policy)
            .await
            .expect("async operation should succeed");

        let retrieved = engine
            .get_policy(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.name, "my-policy");
    }

    #[tokio::test]
    async fn test_get_missing_policy_error() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let err = engine.get_policy("nonexistent").await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_list_policies() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("a"))
            .await
            .expect("SandboxPolicy::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("b"))
            .await
            .expect("SandboxPolicy::new should succeed");

        let policies = engine.list_policies().await;
        assert_eq!(policies.len(), 2);
    }

    #[tokio::test]
    async fn test_update_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let mut policy = SandboxPolicy::new("original");
        let id = policy.id.clone();
        engine
            .add_policy(policy.clone())
            .await
            .expect("async operation should succeed");

        policy.name = "updated".to_string();
        engine
            .update_policy(policy)
            .await
            .expect("async operation should succeed");

        let retrieved = engine
            .get_policy(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(retrieved.name, "updated");
    }

    #[tokio::test]
    async fn test_update_missing_policy_error() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::new("ghost");
        let err = engine.update_policy(policy).await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::new("doomed");
        let id = policy.id.clone();
        engine
            .add_policy(policy)
            .await
            .expect("async operation should succeed");
        assert_eq!(engine.policy_count().await, 1);

        engine
            .delete_policy(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(engine.policy_count().await, 0);
    }

    #[tokio::test]
    async fn test_delete_missing_policy_error() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let err = engine.delete_policy("nope").await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_js_execution_unsupported() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let req = ExecutionRequest {
            code: "console.log('hi')".to_string(),
            language: ExecutionLanguage::JavaScript,
            policy_id: None,
            stdin: None,
        };
        let err = engine.execute(req).await.unwrap_err();
        assert!(matches!(err, SandboxError::ExecutionFailed(_)));
    }

    #[tokio::test]
    async fn test_ts_execution_unsupported() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let req = ExecutionRequest {
            code: "const x: number = 1;".to_string(),
            language: ExecutionLanguage::TypeScript,
            policy_id: None,
            stdin: None,
        };
        let err = engine.execute(req).await.unwrap_err();
        assert!(matches!(err, SandboxError::ExecutionFailed(_)));
    }

    #[tokio::test]
    async fn test_execute_invalid_wasm() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let req = ExecutionRequest {
            code: "not valid wasm bytes".to_string(),
            language: ExecutionLanguage::Wasm,
            policy_id: None,
            stdin: None,
        };
        let err = engine.execute(req).await.unwrap_err();
        assert!(matches!(err, SandboxError::CompilationError(_)));
    }

    #[tokio::test]
    async fn test_execution_history_empty() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        assert!(engine.list_executions().await.is_empty());
        assert!(engine.get_execution("none").await.is_none());
    }

    // -- base64 decode tests -----------------------------------------------

    #[test]
    fn test_base64_decode_simple() {
        let decoded = base64_decode("SGVsbG8=").expect("operation should succeed");
        assert_eq!(
            String::from_utf8(decoded).expect("operation should succeed"),
            "Hello"
        );
    }

    #[test]
    fn test_base64_decode_empty() {
        let decoded = base64_decode("").expect("operation should succeed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_base64_decode_padding() {
        let decoded = base64_decode("YQ==").expect("operation should succeed");
        assert_eq!(decoded, vec![b'a']);
    }

    #[test]
    fn test_base64_decode_no_padding() {
        let decoded = base64_decode("YWJj").expect("operation should succeed");
        assert_eq!(
            String::from_utf8(decoded).expect("operation should succeed"),
            "abc"
        );
    }

    #[test]
    fn test_base64_decode_whitespace() {
        let decoded = base64_decode("SGVs\nbG8=").expect("operation should succeed");
        assert_eq!(
            String::from_utf8(decoded).expect("operation should succeed"),
            "Hello"
        );
    }

    // -- Error display tests ------------------------------------------------

    #[test]
    fn test_error_display() {
        assert_eq!(
            SandboxError::PolicyNotFound("x".into()).to_string(),
            "policy not found: x"
        );
        assert_eq!(
            SandboxError::MemoryLimitExceeded { limit_mb: 256 }.to_string(),
            "memory limit exceeded: 256MB"
        );
        assert_eq!(
            SandboxError::CpuTimeLimitExceeded { limit_seconds: 30 }.to_string(),
            "CPU time limit exceeded: 30s"
        );
        assert_eq!(
            SandboxError::WallClockTimeout { limit_seconds: 60 }.to_string(),
            "wall clock timeout: 60s"
        );
        assert_eq!(
            SandboxError::PermissionDenied("no net".into()).to_string(),
            "permission denied: no net"
        );
    }
}

// ============================================================================
// Additional comprehensive tests
// ============================================================================

#[cfg(test)]
mod additional_tests {
    use super::*;

    // ── Policy CRUD ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_delete_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::new("to-delete");
        let policy_id = policy.id.clone();
        engine
            .add_policy(policy)
            .await
            .expect("async operation should succeed");
        assert_eq!(engine.policy_count().await, 1);

        engine
            .delete_policy(&policy_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(engine.policy_count().await, 0);
    }

    #[tokio::test]
    async fn test_delete_missing_policy_error() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let err = engine.delete_policy("nonexistent").await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_policy_count() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        assert_eq!(engine.policy_count().await, 0);

        engine
            .add_policy(SandboxPolicy::new("p1"))
            .await
            .expect("SandboxPolicy::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("p2"))
            .await
            .expect("SandboxPolicy::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("p3"))
            .await
            .expect("SandboxPolicy::new should succeed");
        assert_eq!(engine.policy_count().await, 3);
    }

    #[tokio::test]
    async fn test_execution_count() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        assert_eq!(engine.execution_count().await, 0);
    }

    #[tokio::test]
    async fn test_list_policies_multiple() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("beta"))
            .await
            .expect("SandboxPolicy::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("alpha"))
            .await
            .expect("SandboxPolicy::new should succeed");
        engine
            .add_policy(SandboxPolicy::new("gamma"))
            .await
            .expect("SandboxPolicy::new should succeed");

        let policies = engine.list_policies().await;
        assert_eq!(policies.len(), 3);
    }

    #[tokio::test]
    async fn test_update_policy_fields() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let mut policy = SandboxPolicy::new("mutable");
        let policy_id = policy.id.clone();
        engine
            .add_policy(policy.clone())
            .await
            .expect("async operation should succeed");

        policy.capabilities.net = vec!["example.com:443".to_string()];
        policy.capabilities.fs_read = vec!["/opt".to_string()];
        engine
            .update_policy(policy)
            .await
            .expect("async operation should succeed");

        let updated = engine
            .get_policy(&policy_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(updated.capabilities.net, vec!["example.com:443"]);
        assert_eq!(updated.capabilities.fs_read, vec!["/opt"]);
    }

    // ── Execution with policy ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_with_nonexistent_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let req = ExecutionRequest {
            code: "console.log('hi')".to_string(),
            language: ExecutionLanguage::JavaScript,
            policy_id: Some("ghost".to_string()),
            stdin: None,
        };
        let err = engine.execute(req).await.unwrap_err();
        assert!(matches!(err, SandboxError::PolicyNotFound(_)));
    }

    #[tokio::test]
    async fn test_execute_with_valid_policy() {
        let engine = SandboxEngine::new().expect("SandboxEngine::new should succeed");
        let policy = SandboxPolicy::restrictive("strict");
        let policy_id = policy.id.clone();
        engine
            .add_policy(policy)
            .await
            .expect("async operation should succeed");

        let req = ExecutionRequest {
            code: "console.log('hi')".to_string(),
            language: ExecutionLanguage::JavaScript,
            policy_id: Some(policy_id),
            stdin: None,
        };
        // JS execution fails (no runtime) but shouldn't be PolicyNotFound
        let err = engine.execute(req).await.unwrap_err();
        assert!(!matches!(err, SandboxError::PolicyNotFound(_)));
    }

    // ── SandboxError variants ──────────────────────────────────────────────

    #[test]
    fn test_error_compilation_error() {
        let err = SandboxError::CompilationError("syntax error".into());
        assert_eq!(err.to_string(), "compilation error: syntax error");
    }

    #[test]
    fn test_error_execution_failed() {
        let err = SandboxError::ExecutionFailed("panic".into());
        assert_eq!(err.to_string(), "execution failed: panic");
    }

    #[test]
    fn test_error_runtime_error() {
        let err = SandboxError::RuntimeError("segfault".into());
        assert_eq!(err.to_string(), "runtime error: segfault");
    }

    #[test]
    fn test_error_invalid_module() {
        let err = SandboxError::InvalidModule("bad magic".into());
        assert_eq!(err.to_string(), "invalid WASM module: bad magic");
    }

    #[test]
    fn test_error_policy_already_exists() {
        let err = SandboxError::PolicyAlreadyExists("dup".into());
        assert_eq!(err.to_string(), "policy already exists: dup");
    }

    // ── Policy variants ────────────────────────────────────────────────────

    #[test]
    fn test_policy_restrictive_has_no_capabilities() {
        let policy = SandboxPolicy::restrictive("locked");
        assert!(policy.capabilities.fs_read.is_empty());
        assert!(policy.capabilities.fs_write.is_empty());
        assert!(policy.capabilities.net.is_empty());
        assert!(policy.capabilities.env.is_empty());
    }

    #[test]
    fn test_policy_permissive_has_capabilities() {
        let policy = SandboxPolicy::permissive("open");
        assert!(!policy.capabilities.fs_read.is_empty());
        assert!(!policy.capabilities.net.is_empty());
    }

    #[test]
    fn test_policy_unique_ids() {
        let p1 = SandboxPolicy::new("same-name");
        let p2 = SandboxPolicy::new("same-name");
        assert_ne!(p1.id, p2.id);
    }

    // ── Capabilities ───────────────────────────────────────────────────────

    #[test]
    fn test_capabilities_default_empty() {
        let caps = SandboxCapabilities::default();
        assert!(caps.fs_read.is_empty());
        assert!(caps.fs_write.is_empty());
        assert!(caps.net.is_empty());
        assert!(caps.env.is_empty());
        assert!(caps.env_passthrough.is_none());
    }

    #[test]
    fn test_capabilities_serde() {
        let mut caps = SandboxCapabilities::default();
        caps.fs_read = vec!["/tmp".to_string()];
        caps.net = vec!["example.com:443".to_string()];
        let json = serde_json::to_string(&caps).expect("should serialize to JSON");
        let parsed: SandboxCapabilities =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.fs_read, vec!["/tmp"]);
        assert_eq!(parsed.net, vec!["example.com:443"]);
    }

    // ── ExecutionLanguage ──────────────────────────────────────────────────

    #[test]
    fn test_language_all_variants_display() {
        assert_eq!(ExecutionLanguage::JavaScript.to_string(), "javascript");
        assert_eq!(ExecutionLanguage::TypeScript.to_string(), "typescript");
        assert_eq!(ExecutionLanguage::Wasm.to_string(), "wasm");
    }

    #[test]
    fn test_language_serde_roundtrip() {
        let langs = [
            ExecutionLanguage::JavaScript,
            ExecutionLanguage::TypeScript,
            ExecutionLanguage::Wasm,
        ];
        for lang in &langs {
            let json = serde_json::to_string(lang).expect("should serialize to JSON");
            let parsed: ExecutionLanguage =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(*lang, parsed);
        }
    }

    // ── ExecutionStatus ────────────────────────────────────────────────────

    #[test]
    fn test_status_all_variants() {
        let statuses = [
            ExecutionStatus::Running,
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).expect("should serialize to JSON");
            let parsed: ExecutionStatus =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(*status, parsed);
        }
    }

    // ── Limits ─────────────────────────────────────────────────────────────

    #[test]
    fn test_limits_default_values() {
        let limits = SandboxLimits::default();
        assert_eq!(limits.memory_mb, 256);
        assert_eq!(limits.cpu_seconds, 30);
        assert_eq!(limits.wall_clock_seconds, 60);
    }

    #[test]
    fn test_limits_custom_values() {
        let limits = SandboxLimits {
            memory_mb: 512,
            cpu_seconds: 60,
            wall_clock_seconds: 120,
        };
        let json = serde_json::to_string(&limits).expect("should serialize to JSON");
        let parsed: SandboxLimits = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.memory_mb, 512);
        assert_eq!(parsed.wall_clock_seconds, 120);
    }

    // ── ExecutionRequest ───────────────────────────────────────────────────

    #[test]
    fn test_execution_request_with_stdin() {
        let req = ExecutionRequest {
            code: "console.log('test')".to_string(),
            language: ExecutionLanguage::JavaScript,
            policy_id: Some("default".to_string()),
            stdin: Some("input data".to_string()),
        };
        assert_eq!(req.stdin.as_deref(), Some("input data"));
    }

    #[test]
    fn test_execution_request_serde() {
        let req = ExecutionRequest {
            code: "code".to_string(),
            language: ExecutionLanguage::Wasm,
            policy_id: None,
            stdin: None,
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let parsed: ExecutionRequest =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.language, ExecutionLanguage::Wasm);
        assert!(parsed.policy_id.is_none());
    }
}

// ============================================================================
// EnvPassthrough tests
// ============================================================================

#[cfg(test)]
mod env_passthrough_tests {
    use super::*;

    #[test]
    fn test_env_passthrough_default_blocks_all() {
        let pt = EnvPassthrough::default();
        assert!(!pt.is_allowed("HOME"));
        assert!(!pt.is_allowed("PATH"));
        assert!(!pt.is_allowed("AWS_REGION"));
    }

    #[test]
    fn test_env_passthrough_exact_match() {
        let pt = EnvPassthrough {
            allow: vec!["HOME".to_string()],
            block: vec![],
        };
        assert!(pt.is_allowed("HOME"));
        assert!(!pt.is_allowed("PATH"));
        assert!(!pt.is_allowed("HOMEDIR"));
    }

    #[test]
    fn test_env_passthrough_wildcard_all() {
        let pt = EnvPassthrough {
            allow: vec!["*".to_string()],
            block: vec![],
        };
        assert!(pt.is_allowed("HOME"));
        assert!(pt.is_allowed("PATH"));
        assert!(pt.is_allowed("AWS_REGION"));
        assert!(pt.is_allowed("ANYTHING_AT_ALL"));
    }

    #[test]
    fn test_env_passthrough_prefix_wildcard() {
        let pt = EnvPassthrough {
            allow: vec!["AWS_*".to_string()],
            block: vec![],
        };
        assert!(pt.is_allowed("AWS_REGION"));
        assert!(pt.is_allowed("AWS_ACCESS_KEY_ID"));
        assert!(!pt.is_allowed("HOME"));
        assert!(!pt.is_allowed("GAWS_REGION"));
    }

    #[test]
    fn test_env_passthrough_suffix_wildcard() {
        let pt = EnvPassthrough {
            allow: vec!["*".to_string()],
            block: vec!["*_SECRET".to_string()],
        };
        assert!(!pt.is_allowed("DB_SECRET"));
        assert!(!pt.is_allowed("APP_SECRET"));
        assert!(pt.is_allowed("DB_HOST"));
        assert!(pt.is_allowed("HOME"));
    }

    #[test]
    fn test_env_passthrough_block_precedence() {
        let pt = EnvPassthrough {
            allow: vec!["HOME".to_string(), "PATH".to_string()],
            block: vec!["HOME".to_string()],
        };
        assert!(!pt.is_allowed("HOME"));
        assert!(pt.is_allowed("PATH"));
    }

    #[test]
    fn test_env_passthrough_resolve() {
        // SAFETY: test-only; set_var/remove_var can race with other threads
        // reading env, but cargo test runs this in its own process.
        unsafe {
            std::env::set_var("ZEUS_TEST_PASSTHROUGH_A", "alpha");
            std::env::set_var("ZEUS_TEST_PASSTHROUGH_B", "beta");
        }

        let pt = EnvPassthrough {
            allow: vec!["ZEUS_TEST_PASSTHROUGH_*".to_string()],
            block: vec![],
        };

        let resolved = pt.resolve();
        assert_eq!(
            resolved.get("ZEUS_TEST_PASSTHROUGH_A").map(|s| s.as_str()),
            Some("alpha")
        );
        assert_eq!(
            resolved.get("ZEUS_TEST_PASSTHROUGH_B").map(|s| s.as_str()),
            Some("beta")
        );
        assert!(resolved.get("HOME").is_none());

        // Clean up
        unsafe {
            std::env::remove_var("ZEUS_TEST_PASSTHROUGH_A");
            std::env::remove_var("ZEUS_TEST_PASSTHROUGH_B");
        }
    }

    #[test]
    fn test_env_passthrough_serde_roundtrip() {
        let pt = EnvPassthrough {
            allow: vec!["HOME".to_string(), "AWS_*".to_string()],
            block: vec!["*_SECRET".to_string(), "*_TOKEN".to_string()],
        };

        let json = serde_json::to_string(&pt).expect("should serialize to JSON");
        let parsed: EnvPassthrough =
            serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(parsed.allow, vec!["HOME", "AWS_*"]);
        assert_eq!(parsed.block, vec!["*_SECRET", "*_TOKEN"]);
    }

    #[test]
    fn test_env_passthrough_capabilities_backward_compat() {
        // Old JSON without env_passthrough field should deserialize fine (None)
        let json = r#"{
            "fs_read": ["/tmp"],
            "fs_write": [],
            "net": [],
            "env": ["HOME"]
        }"#;
        let caps: SandboxCapabilities =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(caps.env_passthrough.is_none());
        assert_eq!(caps.env, vec!["HOME"]);
        assert_eq!(caps.fs_read, vec!["/tmp"]);
    }

    #[test]
    fn test_env_passthrough_mixed_patterns() {
        let pt = EnvPassthrough {
            allow: vec!["HOME".to_string(), "AWS_*".to_string()],
            block: vec!["AWS_SECRET*".to_string()],
        };
        // HOME: exact match in allow, not blocked -> allowed
        assert!(pt.is_allowed("HOME"));
        // AWS_REGION: matches AWS_* allow, not blocked -> allowed
        assert!(pt.is_allowed("AWS_REGION"));
        // AWS_SECRET_KEY: matches AWS_* allow, but also matches AWS_SECRET* block -> blocked
        assert!(!pt.is_allowed("AWS_SECRET_KEY"));
        // AWS_SECRET_ACCESS_KEY: matches AWS_* allow, matches AWS_SECRET* block -> blocked
        assert!(!pt.is_allowed("AWS_SECRET_ACCESS_KEY"));
        // PATH: not in allow -> blocked
        assert!(!pt.is_allowed("PATH"));
    }
}
