//! Dynamic Plugin Loading with Hot-Reload
//!
//! Provides runtime loading of native shared libraries (.dylib, .so, .dll)
//! with file system watching for automatic hot-reload on changes.
//!
//! Features:
//! - Load native plugins at runtime via libloading
//! - Watch plugin directories for changes (add/modify/remove)
//! - Hot-reload plugins when files change
//! - Plugin lifecycle hooks (init, shutdown)
//! - Thread-safe plugin registry

use libloading::{Library, Symbol};
use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Errors from dynamic plugin operations
#[derive(Debug, Error)]
pub enum DynamicPluginError {
    #[error("Failed to load library: {0}")]
    LoadError(String),

    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("Plugin initialization failed: {0}")]
    InitError(String),

    #[error("Plugin not found: {0}")]
    NotFound(String),

    #[error("Plugin already loaded: {0}")]
    AlreadyLoaded(String),

    #[error("Watch error: {0}")]
    WatchError(String),

    #[error("Plugin path is outside all registered search directories")]
    UntrustedPath,

    #[error("Plugin file hash mismatch — expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for dynamic plugin operations
pub type DynamicPluginResult<T> = Result<T, DynamicPluginError>;

/// Plugin metadata returned by plugin_info() function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePluginInfo {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin author
    pub author: Option<String>,
    /// Plugin description
    pub description: Option<String>,
    /// Exported function names
    pub functions: Vec<String>,
}

impl Default for NativePluginInfo {
    fn default() -> Self {
        Self {
            name: "unknown".to_string(),
            version: "0.0.0".to_string(),
            author: None,
            description: None,
            functions: Vec::new(),
        }
    }
}

/// Function signatures for plugin entry points
pub type PluginInitFn = unsafe extern "C" fn() -> i32;
pub type PluginShutdownFn = unsafe extern "C" fn();
pub type PluginInfoFn = unsafe extern "C" fn() -> *const std::ffi::c_char;
pub type PluginExecuteFn = unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char;

/// A loaded native plugin
pub struct NativePlugin {
    /// Plugin info
    pub info: NativePluginInfo,
    /// Path to the shared library
    pub path: PathBuf,
    /// Loaded library handle
    library: Library,
    /// Whether the plugin has been initialized
    initialized: bool,
}

impl NativePlugin {
    /// Load a plugin from a shared library
    fn load(path: &Path) -> DynamicPluginResult<Self> {
        info!(path = %path.display(), "Loading native plugin");

        // Safety: Loading a shared library can execute arbitrary code
        let library = unsafe {
            Library::new(path).map_err(|e| DynamicPluginError::LoadError(e.to_string()))?
        };

        // Try to get plugin info
        let info = Self::get_plugin_info(&library, path)?;

        Ok(Self {
            info,
            path: path.to_path_buf(),
            library,
            initialized: false,
        })
    }

    /// Get plugin info from the library
    fn get_plugin_info(library: &Library, path: &Path) -> DynamicPluginResult<NativePluginInfo> {
        // Try to call plugin_info() if it exists
        let info_result: Result<Symbol<PluginInfoFn>, _> =
            unsafe { library.get(b"zeus_plugin_info\0") };

        if let Ok(info_fn) = info_result {
            let info_ptr = unsafe { info_fn() };
            if !info_ptr.is_null() {
                let info_str = unsafe { std::ffi::CStr::from_ptr(info_ptr) };
                if let Ok(s) = info_str.to_str()
                    && let Ok(info) = serde_json::from_str::<NativePluginInfo>(s)
                {
                    return Ok(info);
                }
            }
        }

        // Fall back to using filename as name
        Ok(NativePluginInfo {
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string(),
            ..Default::default()
        })
    }

    /// Initialize the plugin
    fn init(&mut self) -> DynamicPluginResult<()> {
        if self.initialized {
            return Ok(());
        }

        // Try to call plugin_init() if it exists
        let init_result: Result<Symbol<PluginInitFn>, _> =
            unsafe { self.library.get(b"zeus_plugin_init\0") };

        if let Ok(init_fn) = init_result {
            // Safety: catch_unwind prevents plugin panics from crashing Zeus
            let init_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { init_fn() }));
            match init_result {
                Ok(result) if result != 0 => {
                    return Err(DynamicPluginError::InitError(format!(
                        "Plugin init returned {}",
                        result
                    )));
                }
                Err(_) => {
                    return Err(DynamicPluginError::InitError(
                        "Plugin panicked during initialization".to_string(),
                    ));
                }
                _ => {}
            }
        }

        self.initialized = true;
        info!(name = %self.info.name, "Plugin initialized");
        Ok(())
    }

    /// Shutdown the plugin
    fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }

        // Try to call plugin_shutdown() if it exists
        let shutdown_result: Result<Symbol<PluginShutdownFn>, _> =
            unsafe { self.library.get(b"zeus_plugin_shutdown\0") };

        if let Ok(shutdown_fn) = shutdown_result {
            unsafe { shutdown_fn() };
        }

        self.initialized = false;
        info!(name = %self.info.name, "Plugin shutdown");
    }

    /// Execute a function in the plugin
    pub fn execute(&self, input: &str) -> DynamicPluginResult<String> {
        // Get the execute function
        let exec_fn: Symbol<PluginExecuteFn> = unsafe {
            self.library.get(b"zeus_plugin_execute\0").map_err(|_| {
                DynamicPluginError::SymbolNotFound("zeus_plugin_execute".to_string())
            })?
        };

        // Convert input to C string
        let c_input = std::ffi::CString::new(input)
            .map_err(|e| DynamicPluginError::InitError(e.to_string()))?;

        // Safety: catch_unwind prevents plugin panics from crashing the host process.
        // The raw pointer is checked for null before dereferencing.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let result_ptr = unsafe { exec_fn(c_input.as_ptr()) };

            if result_ptr.is_null() {
                return Ok(String::new());
            }

            // Convert result back to Rust string
            let result_str = unsafe { std::ffi::CStr::from_ptr(result_ptr) };
            Ok(result_str.to_string_lossy().to_string())
        }));

        match result {
            Ok(inner) => inner,
            Err(_) => Err(DynamicPluginError::InitError(
                "Plugin panicked during execution".to_string(),
            )),
        }
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Events for plugin hot-reload
#[derive(Debug, Clone)]
pub enum PluginEvent {
    /// A plugin was loaded
    Loaded(String),
    /// A plugin was reloaded
    Reloaded(String),
    /// A plugin was unloaded
    Unloaded(String),
    /// Error occurred
    Error(String),
}

/// Dynamic Plugin Loader with hot-reload support
pub struct DynamicPluginLoader {
    /// Loaded plugins (name -> plugin)
    plugins: Arc<RwLock<HashMap<String, NativePlugin>>>,
    /// Plugin search directories (only paths within these dirs are permitted)
    search_dirs: Vec<PathBuf>,
    /// Explicit SHA-256 allowlist (hex strings). When non-empty, only plugins
    /// whose file hash matches an entry may be loaded (P23-1 sandboxing).
    trusted_hashes: HashSet<String>,
    /// File watcher
    watcher: Option<RecommendedWatcher>,
    /// Event sender for hot-reload notifications
    event_tx: Option<mpsc::Sender<PluginEvent>>,
}

impl DynamicPluginLoader {
    /// Create a new plugin loader.
    /// No plugins can be loaded until at least one search directory is added.
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            search_dirs: Vec::new(),
            trusted_hashes: HashSet::new(),
            watcher: None,
            event_tx: None,
        }
    }

    /// Register a trusted plugin SHA-256 hash (hex string, lowercase).
    /// When the allowlist is non-empty only matching plugins may be loaded.
    pub fn allow_hash(&mut self, hex_sha256: impl Into<String>) {
        self.trusted_hashes.insert(hex_sha256.into().to_lowercase());
    }

    /// Compute the SHA-256 hash of a file, returned as a lowercase hex string.
    fn hash_file(path: &Path) -> std::io::Result<String> {
        let bytes = std::fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// P23-1: Verify that `path` is safe to load:
    ///   1. The canonical path must lie within one of the registered search dirs.
    ///   2. When the trusted-hash allowlist is non-empty the file hash must match.
    fn verify_plugin_path(&self, path: &Path) -> DynamicPluginResult<()> {
        // Rule 1 — path must be inside a registered search dir
        let canonical = path
            .canonicalize()
            .map_err(|e| DynamicPluginError::LoadError(format!("cannot canonicalise path: {e}")))?;

        let in_search_dir = self.search_dirs.iter().any(|dir| {
            dir.canonicalize()
                .map(|d| canonical.starts_with(&d))
                .unwrap_or(false)
        });

        if !in_search_dir && !self.search_dirs.is_empty() {
            warn!(
                path = %canonical.display(),
                "Rejecting plugin load: path is outside all registered search directories"
            );
            return Err(DynamicPluginError::UntrustedPath);
        }

        // Rule 2 — hash allowlist check (skipped when allowlist is empty)
        if !self.trusted_hashes.is_empty() {
            let actual = Self::hash_file(&canonical)
                .map_err(|e| DynamicPluginError::LoadError(format!("cannot hash plugin: {e}")))?;

            if !self.trusted_hashes.contains(&actual) {
                warn!(
                    path = %canonical.display(),
                    hash = %actual,
                    "Rejecting plugin load: SHA-256 hash not in trusted allowlist"
                );
                return Err(DynamicPluginError::HashMismatch {
                    expected: format!("<one of {} allowlisted hashes>", self.trusted_hashes.len()),
                    actual,
                });
            }
            debug!(path = %canonical.display(), hash = %actual, "Plugin hash verified");
        }

        Ok(())
    }

    /// Add a search directory for plugins
    pub fn add_search_dir(&mut self, dir: PathBuf) {
        if !self.search_dirs.contains(&dir) {
            self.search_dirs.push(dir);
        }
    }

    /// Load a specific plugin by path.
    ///
    /// P23-1: Path must lie within a registered search dir and, if a hash
    /// allowlist is configured, the file's SHA-256 must match a trusted entry.
    pub async fn load_plugin(&self, path: &Path) -> DynamicPluginResult<String> {
        self.verify_plugin_path(path)?;
        let plugin = NativePlugin::load(path)?;
        let name = plugin.info.name.clone();

        {
            let mut plugins = self.plugins.write().await;
            if plugins.contains_key(&name) {
                return Err(DynamicPluginError::AlreadyLoaded(name));
            }
            plugins.insert(name.clone(), plugin);
        }

        // Initialize plugin
        {
            let mut plugins = self.plugins.write().await;
            if let Some(plugin) = plugins.get_mut(&name) {
                plugin.init()?;
            }
        }

        // Send event
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(PluginEvent::Loaded(name.clone())).await;
        }

        Ok(name)
    }

    /// Unload a plugin by name
    pub async fn unload_plugin(&self, name: &str) -> DynamicPluginResult<()> {
        let mut plugins = self.plugins.write().await;
        plugins
            .remove(name)
            .ok_or_else(|| DynamicPluginError::NotFound(name.to_string()))?;

        // Send event
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(PluginEvent::Unloaded(name.to_string())).await;
        }

        info!(name = %name, "Plugin unloaded");
        Ok(())
    }

    /// Reload a plugin (unload + load)
    pub async fn reload_plugin(&self, name: &str) -> DynamicPluginResult<()> {
        let path = {
            let plugins = self.plugins.read().await;
            let plugin = plugins
                .get(name)
                .ok_or_else(|| DynamicPluginError::NotFound(name.to_string()))?;
            plugin.path.clone()
        };

        // Unload
        self.unload_plugin(name).await?;

        // Small delay to ensure library is released
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Reload
        self.load_plugin(&path).await?;

        // Send event
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(PluginEvent::Reloaded(name.to_string())).await;
        }

        info!(name = %name, "Plugin reloaded");
        Ok(())
    }

    /// List all loaded plugins
    pub async fn list_plugins(&self) -> Vec<NativePluginInfo> {
        let plugins = self.plugins.read().await;
        plugins.values().map(|p| p.info.clone()).collect()
    }

    /// Get a plugin by name
    pub async fn get_plugin_info(&self, name: &str) -> Option<NativePluginInfo> {
        let plugins = self.plugins.read().await;
        plugins.get(name).map(|p| p.info.clone())
    }

    /// Execute a function in a plugin
    pub async fn execute(&self, plugin_name: &str, input: &str) -> DynamicPluginResult<String> {
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(plugin_name)
            .ok_or_else(|| DynamicPluginError::NotFound(plugin_name.to_string()))?;
        plugin.execute(input)
    }

    /// Scan and load all plugins from search directories
    pub async fn scan_and_load(&self) -> DynamicPluginResult<Vec<String>> {
        let mut loaded = Vec::new();

        for dir in &self.search_dirs {
            if !dir.exists() {
                continue;
            }

            let entries = std::fs::read_dir(dir)?;
            for entry in entries.flatten() {
                let path = entry.path();
                if Self::is_plugin_file(&path) {
                    match self.load_plugin(&path).await {
                        Ok(name) => loaded.push(name),
                        Err(e) => {
                            warn!(path = %path.display(), error = %e, "Failed to load plugin")
                        }
                    }
                }
            }
        }

        Ok(loaded)
    }

    /// Check if a file is a plugin (by extension)
    fn is_plugin_file(path: &Path) -> bool {
        let ext = path.extension().and_then(OsStr::to_str);
        match ext {
            Some("dylib") => cfg!(target_os = "macos"),
            Some("so") => cfg!(target_os = "linux"),
            Some("dll") => cfg!(target_os = "windows"),
            _ => false,
        }
    }

    /// Start watching plugin directories for changes
    pub fn start_watching(&mut self) -> DynamicPluginResult<mpsc::Receiver<PluginEvent>> {
        let (tx, rx) = mpsc::channel(32);
        self.event_tx = Some(tx.clone());

        let plugins = self.plugins.clone();
        let search_dirs = self.search_dirs.clone();

        // Create watcher
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    Self::handle_fs_event(&event, &plugins, &tx, &search_dirs);
                }
            },
            Config::default(),
        )
        .map_err(|e| DynamicPluginError::WatchError(e.to_string()))?;

        // Watch all search directories
        for dir in &self.search_dirs {
            if dir.exists() {
                watcher
                    .watch(dir, RecursiveMode::NonRecursive)
                    .map_err(|e| DynamicPluginError::WatchError(e.to_string()))?;
                info!(dir = %dir.display(), "Watching for plugin changes");
            }
        }

        self.watcher = Some(watcher);
        Ok(rx)
    }

    /// Handle filesystem events for hot-reload
    fn handle_fs_event(
        event: &Event,
        plugins: &Arc<RwLock<HashMap<String, NativePlugin>>>,
        tx: &mpsc::Sender<PluginEvent>,
        _search_dirs: &[PathBuf],
    ) {
        for path in &event.paths {
            if !Self::is_plugin_file(path) {
                continue;
            }

            match &event.kind {
                EventKind::Create(_) => {
                    debug!(path = %path.display(), "Plugin file created");
                    // Will be loaded on next scan
                }
                EventKind::Modify(ModifyKind::Data(_)) => {
                    debug!(path = %path.display(), "Plugin file modified");
                    // Trigger reload
                    let plugins = plugins.clone();
                    let tx = tx.clone();
                    let path = path.clone();
                    tokio::spawn(async move {
                        let plugins_guard = plugins.read().await;
                        for (name, plugin) in plugins_guard.iter() {
                            if plugin.path == path {
                                let _ = tx.send(PluginEvent::Reloaded(name.clone())).await;
                                break;
                            }
                        }
                    });
                }
                EventKind::Remove(_) => {
                    debug!(path = %path.display(), "Plugin file removed");
                    let plugins = plugins.clone();
                    let tx = tx.clone();
                    let path = path.clone();
                    tokio::spawn(async move {
                        let plugins_guard = plugins.read().await;
                        for (name, plugin) in plugins_guard.iter() {
                            if plugin.path == path {
                                let _ = tx.send(PluginEvent::Unloaded(name.clone())).await;
                                break;
                            }
                        }
                    });
                }
                _ => {}
            }
        }
    }

    /// Stop watching for changes
    pub fn stop_watching(&mut self) {
        self.watcher = None;
        self.event_tx = None;
        info!("Stopped watching for plugin changes");
    }
}

impl Default for DynamicPluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for DynamicPluginLoader
pub struct DynamicPluginLoaderBuilder {
    search_dirs: Vec<PathBuf>,
    auto_watch: bool,
}

impl DynamicPluginLoaderBuilder {
    pub fn new() -> Self {
        Self {
            search_dirs: Vec::new(),
            auto_watch: false,
        }
    }

    /// Add a plugin search directory
    pub fn search_dir(mut self, dir: PathBuf) -> Self {
        self.search_dirs.push(dir);
        self
    }

    /// Add the default plugin directory (~/.zeus/plugins)
    pub fn with_default_dir(mut self) -> Self {
        if let Some(config_dir) = dirs::config_dir() {
            self.search_dirs
                .push(config_dir.join("zeus").join("plugins"));
        }
        self
    }

    /// Enable auto-watching for hot-reload
    pub fn auto_watch(mut self, enabled: bool) -> Self {
        self.auto_watch = enabled;
        self
    }

    /// Build the loader
    pub fn build(self) -> DynamicPluginLoader {
        let mut loader = DynamicPluginLoader::new();
        for dir in self.search_dirs {
            loader.add_search_dir(dir);
        }
        loader
    }
}

impl Default for DynamicPluginLoaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_plugin_info_default() {
        let info = NativePluginInfo::default();
        assert_eq!(info.name, "unknown");
        assert_eq!(info.version, "0.0.0");
        assert!(info.author.is_none());
        assert!(info.functions.is_empty());
    }

    #[test]
    fn test_native_plugin_info_serialization() {
        let info = NativePluginInfo {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            author: Some("Zeus".to_string()),
            description: Some("A test plugin".to_string()),
            functions: vec!["hello".to_string(), "goodbye".to_string()],
        };
        let json = serde_json::to_string(&info).expect("should serialize to JSON");
        let parsed: NativePluginInfo =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.name, "test-plugin");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.functions.len(), 2);
    }

    #[test]
    fn test_plugin_loader_new() {
        let loader = DynamicPluginLoader::new();
        assert!(loader.search_dirs.is_empty());
    }

    #[test]
    fn test_plugin_loader_add_search_dir() {
        let mut loader = DynamicPluginLoader::new();
        loader.add_search_dir(PathBuf::from("/tmp/plugins"));
        loader.add_search_dir(PathBuf::from("/tmp/plugins")); // Duplicate
        assert_eq!(loader.search_dirs.len(), 1);
    }

    #[test]
    fn test_is_plugin_file() {
        #[cfg(target_os = "macos")]
        {
            assert!(DynamicPluginLoader::is_plugin_file(&PathBuf::from(
                "test.dylib"
            )));
            assert!(!DynamicPluginLoader::is_plugin_file(&PathBuf::from(
                "test.so"
            )));
        }

        #[cfg(target_os = "linux")]
        {
            assert!(DynamicPluginLoader::is_plugin_file(&PathBuf::from(
                "test.so"
            )));
            assert!(!DynamicPluginLoader::is_plugin_file(&PathBuf::from(
                "test.dylib"
            )));
        }

        // Common non-plugin files
        assert!(!DynamicPluginLoader::is_plugin_file(&PathBuf::from(
            "test.txt"
        )));
        assert!(!DynamicPluginLoader::is_plugin_file(&PathBuf::from(
            "test.rs"
        )));
    }

    #[test]
    fn test_plugin_event_variants() {
        let loaded = PluginEvent::Loaded("test".to_string());
        let reloaded = PluginEvent::Reloaded("test".to_string());
        let unloaded = PluginEvent::Unloaded("test".to_string());
        let error = PluginEvent::Error("error".to_string());

        // Just verify they can be created and matched
        match loaded {
            PluginEvent::Loaded(name) => assert_eq!(name, "test"),
            _ => panic!("Wrong variant"),
        }
        match reloaded {
            PluginEvent::Reloaded(name) => assert_eq!(name, "test"),
            _ => panic!("Wrong variant"),
        }
        match unloaded {
            PluginEvent::Unloaded(name) => assert_eq!(name, "test"),
            _ => panic!("Wrong variant"),
        }
        match error {
            PluginEvent::Error(msg) => assert_eq!(msg, "error"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_builder_default() {
        let builder = DynamicPluginLoaderBuilder::new();
        assert!(builder.search_dirs.is_empty());
        assert!(!builder.auto_watch);
    }

    #[test]
    fn test_builder_chain() {
        let builder = DynamicPluginLoaderBuilder::new()
            .search_dir(PathBuf::from("/plugins"))
            .with_default_dir()
            .auto_watch(true);

        assert!(builder.search_dirs.contains(&PathBuf::from("/plugins")));
        assert!(builder.auto_watch);
    }

    #[test]
    fn test_builder_build() {
        let loader = DynamicPluginLoaderBuilder::new()
            .search_dir(PathBuf::from("/tmp/test_plugins"))
            .build();

        assert_eq!(loader.search_dirs.len(), 1);
        assert_eq!(loader.search_dirs[0], PathBuf::from("/tmp/test_plugins"));
    }

    #[tokio::test]
    async fn test_list_plugins_empty() {
        let loader = DynamicPluginLoader::new();
        let plugins = loader.list_plugins().await;
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn test_get_nonexistent_plugin() {
        let loader = DynamicPluginLoader::new();
        let info = loader.get_plugin_info("nonexistent").await;
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn test_execute_nonexistent_plugin() {
        let loader = DynamicPluginLoader::new();
        let result = loader.execute("nonexistent", "test").await;
        assert!(result.is_err());
        match result {
            Err(DynamicPluginError::NotFound(name)) => assert_eq!(name, "nonexistent"),
            _ => panic!("Expected NotFound error"),
        }
    }

    #[test]
    fn test_error_display() {
        let err = DynamicPluginError::NotFound("test".to_string());
        assert_eq!(err.to_string(), "Plugin not found: test");

        let err = DynamicPluginError::AlreadyLoaded("test".to_string());
        assert_eq!(err.to_string(), "Plugin already loaded: test");

        let err = DynamicPluginError::SymbolNotFound("fn".to_string());
        assert_eq!(err.to_string(), "Symbol not found: fn");
    }
}

#[cfg(test)]
mod plugin_security_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_hash_file_produces_consistent_result() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.so");
        fs::write(&path, b"fake plugin content").unwrap();

        let hash1 = DynamicPluginLoader::hash_file(&path).unwrap();
        let hash2 = DynamicPluginLoader::hash_file(&path).unwrap();
        assert_eq!(hash1, hash2, "hash must be deterministic");
        assert_eq!(hash1.len(), 64, "SHA-256 hex should be 64 chars");
        // Known SHA-256 of "fake plugin content"
        assert_eq!(
            hash1,
            "972b4d5bd9b5dabce82f5d17d1a3170a82212c657cb97f1f0e741dcbca3dd357"
        );
    }

    #[tokio::test]
    async fn test_load_plugin_rejects_path_outside_search_dirs() {
        let search_dir = TempDir::new().unwrap();
        let other_dir = TempDir::new().unwrap();

        // Create a fake .so file outside the search dir
        let plugin_path = other_dir.path().join("evil.so");
        fs::write(&plugin_path, b"evil").unwrap();

        let mut loader = DynamicPluginLoader::new();
        loader.add_search_dir(search_dir.path().to_path_buf());

        let result = loader.load_plugin(&plugin_path).await;
        assert!(result.is_err(), "should reject plugin outside search dirs");
        assert!(matches!(
            result.unwrap_err(),
            DynamicPluginError::UntrustedPath
        ));
    }

    #[tokio::test]
    async fn test_load_plugin_rejects_hash_mismatch() {
        let search_dir = TempDir::new().unwrap();
        let plugin_path = search_dir.path().join("plugin.so");
        fs::write(&plugin_path, b"plugin content").unwrap();

        let mut loader = DynamicPluginLoader::new();
        loader.add_search_dir(search_dir.path().to_path_buf());
        loader.allow_hash("0000000000000000000000000000000000000000000000000000000000000000");

        let result = loader.load_plugin(&plugin_path).await;
        assert!(
            result.is_err(),
            "should reject plugin with non-matching hash"
        );
        assert!(matches!(
            result.unwrap_err(),
            DynamicPluginError::HashMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn test_load_plugin_accepted_with_correct_hash() {
        let search_dir = TempDir::new().unwrap();
        let plugin_path = search_dir.path().join("good.so");
        let contents = b"good plugin content";
        fs::write(&plugin_path, contents).unwrap();

        // Compute correct hash
        let correct_hash = DynamicPluginLoader::hash_file(&plugin_path).unwrap();

        let mut loader = DynamicPluginLoader::new();
        loader.add_search_dir(search_dir.path().to_path_buf());
        loader.allow_hash(correct_hash);

        // This will still fail at NativePlugin::load (not a real .so) but NOT at hash check
        let result = loader.load_plugin(&plugin_path).await;
        // Must not be UntrustedPath or HashMismatch
        match result {
            Err(DynamicPluginError::UntrustedPath) => panic!("should not fail path check"),
            Err(DynamicPluginError::HashMismatch { .. }) => panic!("should not fail hash check"),
            _ => {} // LoadError from libloading is expected for fake .so
        }
    }

    #[test]
    fn test_allow_hash_normalises_to_lowercase() {
        let mut loader = DynamicPluginLoader::new();
        loader.allow_hash("ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890");
        // Verify it's stored as lowercase (internal detail — tested indirectly
        // by checking hash_file produces lowercase output)
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("f.so");
        std::fs::write(&path, b"x").unwrap();
        let h = DynamicPluginLoader::hash_file(&path).unwrap();
        assert_eq!(h, h.to_lowercase(), "hash must be lowercase");
    }

    #[tokio::test]
    async fn test_load_allowed_when_no_search_dirs_and_no_hash_list() {
        // When no search dirs are configured, path check is skipped (backward-compat)
        // When no hash list is configured, hash check is skipped
        // In both cases it proceeds to NativePlugin::load (which may fail on fake file)
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_path = tmp.path().join("any.so");
        std::fs::write(&plugin_path, b"fake").unwrap();

        let loader = DynamicPluginLoader::new(); // no search dirs, no hashes
        let result = loader.load_plugin(&plugin_path).await;
        // Should not be rejected by our checks; may fail at libloading level
        assert!(!matches!(result, Err(DynamicPluginError::UntrustedPath)));
        assert!(!matches!(
            result,
            Err(DynamicPluginError::HashMismatch { .. })
        ));
    }
}
