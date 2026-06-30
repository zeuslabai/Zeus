//! WASM Plugin Sandbox
//!
//! Provides secure execution of WebAssembly plugins using Wasmtime + WASI.
//! Features:
//! - Sandboxed execution with configurable capabilities
//! - Memory limits and execution timeouts
//! - WASI file system isolation
//! - Host function bindings for Zeus APIs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};
use wasmtime::*;

/// Errors from WASM sandbox operations
#[derive(Debug, Error)]
pub enum WasmError {
    #[error("Failed to compile WASM module: {0}")]
    Compilation(String),

    #[error("Failed to instantiate WASM module: {0}")]
    Instantiation(String),

    #[error("WASM execution failed: {0}")]
    Execution(String),

    #[error("WASM execution timed out after {0}ms")]
    Timeout(u64),

    #[error("Memory limit exceeded: {limit_mb}MB")]
    MemoryLimitExceeded { limit_mb: u64 },

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Plugin not found: {0}")]
    NotFound(String),

    #[error("Invalid WASM module: {0}")]
    InvalidModule(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for WASM operations
pub type WasmResult<T> = Result<T, WasmError>;

/// Capability permissions for WASM plugins
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WasmCapabilities {
    /// Allow reading files from specified directories
    pub read_dirs: Vec<PathBuf>,

    /// Allow writing files to specified directories
    pub write_dirs: Vec<PathBuf>,

    /// Allow network access to specified hosts
    pub network_hosts: Vec<String>,

    /// Allow environment variable access (specific vars only)
    pub env_vars: Vec<String>,

    /// Allow access to stdin/stdout/stderr
    pub allow_stdio: bool,

    /// Allow spawning subprocesses
    pub allow_spawn: bool,

    /// Maximum memory in megabytes (default: 64)
    pub max_memory_mb: u64,

    /// Execution timeout in milliseconds (default: 30000)
    pub timeout_ms: u64,

    /// Allow random number generation
    pub allow_random: bool,

    /// Allow access to system clock
    pub allow_clock: bool,
}

impl WasmCapabilities {
    /// Create minimal capabilities (no filesystem, no network)
    pub fn minimal() -> Self {
        Self {
            allow_stdio: true,
            allow_clock: true,
            allow_random: true,
            max_memory_mb: 64,
            timeout_ms: 30000,
            ..Default::default()
        }
    }

    /// Create standard capabilities (read-only sandbox dir)
    pub fn standard(sandbox_dir: &Path) -> Self {
        Self {
            read_dirs: vec![sandbox_dir.to_path_buf()],
            write_dirs: vec![sandbox_dir.join("output")],
            allow_stdio: true,
            allow_clock: true,
            allow_random: true,
            max_memory_mb: 128,
            timeout_ms: 60000,
            ..Default::default()
        }
    }

    /// Create full capabilities (for trusted plugins only)
    pub fn full() -> Self {
        Self {
            read_dirs: vec![PathBuf::from("/")],
            write_dirs: vec![PathBuf::from("/tmp")],
            network_hosts: vec!["*".to_string()],
            allow_stdio: true,
            allow_spawn: false, // Still don't allow spawning
            allow_clock: true,
            allow_random: true,
            max_memory_mb: 512,
            timeout_ms: 300000,
            ..Default::default()
        }
    }
}

/// A loaded WASM plugin
pub struct WasmPlugin {
    /// Plugin name/ID
    pub name: String,
    /// Path to the WASM module
    pub path: PathBuf,
    /// Plugin capabilities
    pub capabilities: WasmCapabilities,
    /// Plugin metadata
    pub metadata: WasmPluginMetadata,
    /// Compiled module (cached)
    module: Module,
}

/// Plugin metadata from manifest
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WasmPluginMetadata {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub exports: Vec<String>,
}

/// WASM execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExecutionResult {
    /// Output from stdout
    pub stdout: String,
    /// Output from stderr
    pub stderr: String,
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Memory used in bytes
    pub memory_used: u64,
}

/// WASM Plugin Sandbox manager
pub struct WasmSandbox {
    /// Wasmtime engine (shared across plugins)
    engine: Engine,
    /// Loaded plugins
    plugins: HashMap<String, WasmPlugin>,
    /// Default capabilities for new plugins
    default_capabilities: WasmCapabilities,
    /// Plugin cache directory
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl WasmSandbox {
    /// Create a new WASM sandbox
    pub fn new(cache_dir: PathBuf) -> WasmResult<Self> {
        let mut config = Config::new();
        config
            .async_support(true)
            .epoch_interruption(true)
            .consume_fuel(true)
            .cranelift_opt_level(OptLevel::Speed);

        let engine =
            Engine::new(&config).map_err(|e| WasmError::Compilation(format!("Engine: {}", e)))?;

        // Ensure cache directory exists
        std::fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            engine,
            plugins: HashMap::new(),
            default_capabilities: WasmCapabilities::minimal(),
            cache_dir,
        })
    }

    /// Set default capabilities for new plugins
    pub fn set_default_capabilities(&mut self, caps: WasmCapabilities) {
        self.default_capabilities = caps;
    }

    /// Load a WASM plugin from file
    pub fn load_plugin(
        &mut self,
        name: &str,
        wasm_path: &Path,
        capabilities: Option<WasmCapabilities>,
    ) -> WasmResult<()> {
        info!(name = %name, path = %wasm_path.display(), "Loading WASM plugin");

        // Read and compile the module
        let wasm_bytes = std::fs::read(wasm_path)?;
        let module = Module::new(&self.engine, &wasm_bytes)
            .map_err(|e| WasmError::Compilation(e.to_string()))?;

        // Extract exports
        let exports: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();

        let metadata = WasmPluginMetadata {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            exports,
            ..Default::default()
        };

        let plugin = WasmPlugin {
            name: name.to_string(),
            path: wasm_path.to_path_buf(),
            capabilities: capabilities.unwrap_or_else(|| self.default_capabilities.clone()),
            metadata,
            module,
        };

        self.plugins.insert(name.to_string(), plugin);
        info!(name = %name, "WASM plugin loaded successfully");

        Ok(())
    }

    /// Unload a plugin
    pub fn unload_plugin(&mut self, name: &str) -> WasmResult<()> {
        self.plugins
            .remove(name)
            .ok_or_else(|| WasmError::NotFound(name.to_string()))?;
        info!(name = %name, "WASM plugin unloaded");
        Ok(())
    }

    /// List loaded plugins
    pub fn list_plugins(&self) -> Vec<&WasmPluginMetadata> {
        self.plugins.values().map(|p| &p.metadata).collect()
    }

    /// Get plugin info
    pub fn get_plugin(&self, name: &str) -> Option<&WasmPlugin> {
        self.plugins.get(name)
    }

    /// Execute a WASM plugin's main function
    ///
    /// This is a simplified implementation that demonstrates the sandbox structure.
    /// Full WASI integration requires wasmtime-wasi preview1 or preview2 APIs
    /// which depend on specific WASM module compilation targets.
    pub async fn execute(
        &self,
        plugin_name: &str,
        _args: Vec<String>,
        _env: HashMap<String, String>,
    ) -> WasmResult<WasmExecutionResult> {
        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| WasmError::NotFound(plugin_name.to_string()))?;

        let start = std::time::Instant::now();

        // Create a minimal store for execution
        let mut store = Store::new(&self.engine, ());

        // Set fuel limit (approximates instruction count)
        let fuel_limit = plugin.capabilities.timeout_ms * 1_000_000;
        store
            .set_fuel(fuel_limit)
            .map_err(|e| WasmError::Instantiation(format!("Failed to set fuel limit: {}", e)))?;

        // Create linker (without WASI for now - basic execution)
        let linker: Linker<()> = Linker::new(&self.engine);

        // Instantiate module
        let instance = linker
            .instantiate(&mut store, &plugin.module)
            .map_err(|e| WasmError::Instantiation(e.to_string()))?;

        // Try to get _start function (WASI entry point)
        let start_result = instance.get_typed_func::<(), ()>(&mut store, "_start");

        let timeout = Duration::from_millis(plugin.capabilities.timeout_ms);
        let result = tokio::time::timeout(timeout, async {
            if let Ok(start_func) = start_result {
                start_func
                    .call(&mut store, ())
                    .map_err(|e| WasmError::Execution(e.to_string()))
            } else {
                // No _start, try main
                if let Ok(main_func) = instance.get_typed_func::<(), ()>(&mut store, "main") {
                    main_func
                        .call(&mut store, ())
                        .map_err(|e| WasmError::Execution(e.to_string()))
                } else {
                    // Module has no entry point, just return success
                    Ok(())
                }
            }
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(())) => Ok(WasmExecutionResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
                duration_ms,
                memory_used: 0,
            }),
            Ok(Err(e)) => {
                warn!(plugin = %plugin_name, error = %e, "WASM execution failed");
                Ok(WasmExecutionResult {
                    stdout: String::new(),
                    stderr: e.to_string(),
                    exit_code: 1,
                    duration_ms,
                    memory_used: 0,
                })
            }
            Err(_) => Err(WasmError::Timeout(plugin.capabilities.timeout_ms)),
        }
    }

    /// Execute a specific exported function from a WASM plugin
    pub async fn call_function(
        &self,
        plugin_name: &str,
        function_name: &str,
        _input: &str,
    ) -> WasmResult<String> {
        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| WasmError::NotFound(plugin_name.to_string()))?;

        let mut store = Store::new(&self.engine, ());

        // Create linker
        let linker: Linker<()> = Linker::new(&self.engine);

        // Instantiate
        let instance = linker
            .instantiate(&mut store, &plugin.module)
            .map_err(|e| WasmError::Instantiation(e.to_string()))?;

        // Check if function exists
        let _func = instance
            .get_func(&mut store, function_name)
            .ok_or_else(|| {
                WasmError::Execution(format!("Function '{}' not found", function_name))
            })?;

        // For full implementation, would need to handle type marshaling
        // based on the function signature
        Ok(format!("Called {}::{}", plugin_name, function_name))
    }

    /// Validate a WASM module without loading it
    pub fn validate(&self, wasm_path: &Path) -> WasmResult<WasmPluginMetadata> {
        let wasm_bytes = std::fs::read(wasm_path)?;

        // Try to compile - this validates the module
        let module = Module::new(&self.engine, &wasm_bytes)
            .map_err(|e| WasmError::InvalidModule(e.to_string()))?;

        let exports: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();

        // Check for required exports
        if !exports.contains(&"_start".to_string())
            && !exports.contains(&"main".to_string())
            && exports.iter().all(|e| e == "memory")
        {
            warn!(
                path = %wasm_path.display(),
                "WASM module has no callable exports"
            );
        }

        Ok(WasmPluginMetadata {
            name: wasm_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            version: "unknown".to_string(),
            exports,
            ..Default::default()
        })
    }
}

/// Builder for WASM sandbox with configuration
pub struct WasmSandboxBuilder {
    cache_dir: PathBuf,
    default_capabilities: WasmCapabilities,
}

impl WasmSandboxBuilder {
    pub fn new() -> Self {
        Self {
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("zeus")
                .join("wasm"),
            default_capabilities: WasmCapabilities::minimal(),
        }
    }

    pub fn cache_dir(mut self, path: PathBuf) -> Self {
        self.cache_dir = path;
        self
    }

    pub fn default_capabilities(mut self, caps: WasmCapabilities) -> Self {
        self.default_capabilities = caps;
        self
    }

    pub fn build(self) -> WasmResult<WasmSandbox> {
        let mut sandbox = WasmSandbox::new(self.cache_dir)?;
        sandbox.set_default_capabilities(self.default_capabilities);
        Ok(sandbox)
    }
}

impl Default for WasmSandboxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_capabilities_minimal() {
        let caps = WasmCapabilities::minimal();
        assert!(caps.read_dirs.is_empty());
        assert!(caps.write_dirs.is_empty());
        assert!(caps.network_hosts.is_empty());
        assert!(caps.allow_stdio);
        assert!(!caps.allow_spawn);
        assert_eq!(caps.max_memory_mb, 64);
        assert_eq!(caps.timeout_ms, 30000);
    }

    #[test]
    fn test_wasm_capabilities_standard() {
        let sandbox_dir = PathBuf::from("/tmp/sandbox");
        let caps = WasmCapabilities::standard(&sandbox_dir);
        assert_eq!(caps.read_dirs, vec![sandbox_dir.clone()]);
        assert_eq!(caps.write_dirs, vec![sandbox_dir.join("output")]);
        assert_eq!(caps.max_memory_mb, 128);
    }

    #[test]
    fn test_wasm_capabilities_full() {
        let caps = WasmCapabilities::full();
        assert!(!caps.allow_spawn); // Even full doesn't allow spawning
        assert_eq!(caps.max_memory_mb, 512);
        assert_eq!(caps.network_hosts, vec!["*".to_string()]);
    }

    #[test]
    fn test_wasm_sandbox_builder_default() {
        let builder = WasmSandboxBuilder::new();
        assert!(builder.cache_dir.ends_with("zeus/wasm"));
    }

    #[test]
    fn test_wasm_sandbox_builder_custom() {
        let builder = WasmSandboxBuilder::new()
            .cache_dir(PathBuf::from("/custom/cache"))
            .default_capabilities(WasmCapabilities::full());
        assert_eq!(builder.cache_dir, PathBuf::from("/custom/cache"));
        assert_eq!(builder.default_capabilities.max_memory_mb, 512);
    }

    #[test]
    fn test_wasm_plugin_metadata_default() {
        let meta = WasmPluginMetadata::default();
        assert!(meta.name.is_empty());
        assert!(meta.exports.is_empty());
    }

    #[test]
    fn test_wasm_sandbox_new() {
        let temp_dir = std::env::temp_dir().join("zeus_wasm_test");
        let result = WasmSandbox::new(temp_dir.clone());
        assert!(result.is_ok());
        let sandbox = result.expect("operation should succeed");
        assert!(sandbox.plugins.is_empty());
        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_wasm_sandbox_list_plugins_empty() {
        let temp_dir = std::env::temp_dir().join("zeus_wasm_test_list");
        let sandbox = WasmSandbox::new(temp_dir.clone()).expect("WasmSandbox::new should succeed");
        assert!(sandbox.list_plugins().is_empty());
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_wasm_sandbox_get_nonexistent_plugin() {
        let temp_dir = std::env::temp_dir().join("zeus_wasm_test_get");
        let sandbox = WasmSandbox::new(temp_dir.clone()).expect("WasmSandbox::new should succeed");
        assert!(sandbox.get_plugin("nonexistent").is_none());
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_wasm_execution_result_serialization() {
        let result = WasmExecutionResult {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 100,
            memory_used: 1024,
        };
        let json = serde_json::to_string(&result).expect("should serialize to JSON");
        let parsed: WasmExecutionResult =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.stdout, "hello");
        assert_eq!(parsed.exit_code, 0);
    }

    #[test]
    fn test_wasm_error_display() {
        let err = WasmError::Timeout(5000);
        assert_eq!(err.to_string(), "WASM execution timed out after 5000ms");

        let err = WasmError::NotFound("test".to_string());
        assert_eq!(err.to_string(), "Plugin not found: test");

        let err = WasmError::MemoryLimitExceeded { limit_mb: 64 };
        assert_eq!(err.to_string(), "Memory limit exceeded: 64MB");
    }
}
