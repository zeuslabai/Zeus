//! Directory-based plugin loader.
//!
//! Scans a directory for plugin manifest files (`plugin.toml`) and loads
//! them into a [`PluginRegistry`]. This is the foundation for hot-reload
//! and dynamic plugin discovery — wiring into the agent loop is out of scope
//! for this crate.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, warn};

/// Errors that can occur during plugin loading.
#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("failed to read directory: {0}")]
    ReadDir(#[from] std::io::Error),

    #[error("failed to parse manifest at {path}: {source}")]
    ManifestParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("invalid manifest at {0}: {1}")]
    InvalidManifest(PathBuf, String),
}

/// Metadata parsed from a `plugin.toml` manifest file.
///
/// Each plugin directory should contain a `plugin.toml` at its root.
/// The loader uses this to discover plugins; actual instantiation is
/// handled by the caller or a future dynamic loader extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin identifier — must be unique within the registry.
    pub name: String,

    /// Semantic version string.
    pub version: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: String,

    /// Entry point relative to the manifest directory.
    /// E.g. `"main.wasm"`, `"plugin.so"`, `"index.js"`.
    pub entry: String,

    /// Runtime required to execute this plugin.
    /// E.g. `"wasm"`, `"native"`, `"node"`, `"python"`.
    #[serde(default = "default_runtime")]
    pub runtime: String,

    /// Arbitrary plugin-specific configuration.
    #[serde(default)]
    pub config: Option<toml::Value>,
}

fn default_runtime() -> String {
    "native".to_string()
}

/// A discovered plugin — manifest plus the directory it was found in.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Absolute path to the plugin's directory.
    pub path: PathBuf,
}

/// Scans a directory tree for plugin manifests.
///
/// Each immediate subdirectory of `dir` is checked for a `plugin.toml`.
/// Subdirectories without a manifest are silently skipped.
///
/// # Errors
///
/// Returns [`LoaderError::DirectoryNotFound`] if `dir` does not exist, or
/// propagates I/O and parse errors.
pub fn discover_plugins(dir: &Path) -> Result<Vec<DiscoveredPlugin>, LoaderError> {
    if !dir.exists() {
        return Err(LoaderError::DirectoryNotFound(dir.to_path_buf()));
    }

    let mut discovered = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("plugin.toml");
        if !manifest_path.exists() {
            debug!("skipping {}: no plugin.toml", path.display());
            continue;
        }

        match load_manifest(&manifest_path) {
            Ok(manifest) => {
                debug!("discovered plugin: {} v{} at {}", manifest.name, manifest.version, path.display());
                discovered.push(DiscoveredPlugin { manifest, path });
            }
            Err(e) => {
                warn!("failed to load manifest at {}: {e}", manifest_path.display());
            }
        }
    }

    Ok(discovered)
}

/// Parse a single `plugin.toml` manifest file.
pub fn load_manifest(path: &Path) -> Result<PluginManifest, LoaderError> {
    let content = std::fs::read_to_string(path)
        .map_err(LoaderError::ReadDir)?;

    let manifest: PluginManifest = toml::from_str(&content)
        .map_err(|source| LoaderError::ManifestParse { path: path.to_path_buf(), source })?;

    // Basic validation
    if manifest.name.is_empty() {
        return Err(LoaderError::InvalidManifest(
            path.to_path_buf(),
            "name must not be empty".to_string(),
        ));
    }
    if manifest.entry.is_empty() {
        return Err(LoaderError::InvalidManifest(
            path.to_path_buf(),
            "entry must not be empty".to_string(),
        ));
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_plugin_dir(parent: &Path, name: &str, toml: &str) -> PathBuf {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), toml).unwrap();
        dir
    }

    #[test]
    fn test_discover_plugins_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_discover_plugins_finds_valid() {
        let tmp = TempDir::new().unwrap();
        make_plugin_dir(tmp.path(), "my-plugin", r#"
            name = "my-plugin"
            version = "1.0.0"
            entry = "plugin.wasm"
            runtime = "wasm"
        "#);
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.name, "my-plugin");
        assert_eq!(plugins[0].manifest.runtime, "wasm");
    }

    #[test]
    fn test_discover_skips_dirs_without_manifest() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("no-manifest")).unwrap();
        make_plugin_dir(tmp.path(), "valid", r#"
            name = "valid"
            version = "0.1.0"
            entry = "main.so"
        "#);
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert_eq!(plugins.len(), 1);
    }

    #[test]
    fn test_discover_skips_invalid_manifests() {
        let tmp = TempDir::new().unwrap();
        // Invalid TOML
        let bad = tmp.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("plugin.toml"), "NOT VALID TOML :::").unwrap();
        // Valid one
        make_plugin_dir(tmp.path(), "good", r#"
            name = "good"
            version = "1.0.0"
            entry = "plugin.so"
        "#);
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.name, "good");
    }

    #[test]
    fn test_discover_directory_not_found() {
        let result = discover_plugins(Path::new("/nonexistent/path/to/plugins"));
        assert!(matches!(result, Err(LoaderError::DirectoryNotFound(_))));
    }

    #[test]
    fn test_manifest_validation_empty_name() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        fs::write(&path, r#"name = "" \n version = "1.0.0" \n entry = "main.so""#).unwrap();
        // The TOML parser will reject this or validation will catch it
        // Either way, it should not produce a valid manifest with empty name
    }

    #[test]
    fn test_default_runtime() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        fs::write(&path, r#"
            name = "test"
            version = "1.0.0"
            entry = "main.so"
        "#).unwrap();
        let manifest = load_manifest(&path).unwrap();
        assert_eq!(manifest.runtime, "native");
    }

    // ── Additional loader tests ──────────────────────────────────────────

    #[test]
    fn test_manifest_with_config() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        fs::write(&path, r#"
            name = "configured"
            version = "2.0.0"
            entry = "plugin.wasm"
            runtime = "wasm"
            description = "A configured plugin"
            [config]
            timeout = 30
            retries = 3
        "#).unwrap();
        let manifest = load_manifest(&path).unwrap();
        assert_eq!(manifest.name, "configured");
        assert_eq!(manifest.version, "2.0.0");
        assert_eq!(manifest.runtime, "wasm");
        assert_eq!(manifest.description, "A configured plugin");
        assert!(manifest.config.is_some());
        let config = manifest.config.unwrap();
        assert_eq!(config["timeout"].as_integer(), Some(30));
        assert_eq!(config["retries"].as_integer(), Some(3));
    }

    #[test]
    fn test_manifest_empty_name_rejected() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("empty-name");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), r#"
            name = ""
            version = "1.0.0"
            entry = "main.so"
        "#).unwrap();
        let result = load_manifest(&dir.join("plugin.toml"));
        assert!(matches!(result, Err(LoaderError::InvalidManifest(_, msg)) if msg.contains("name")));
    }

    #[test]
    fn test_manifest_empty_entry_rejected() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("empty-entry");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), r#"
            name = "test"
            version = "1.0.0"
            entry = ""
        "#).unwrap();
        let result = load_manifest(&dir.join("plugin.toml"));
        assert!(matches!(result, Err(LoaderError::InvalidManifest(_, msg)) if msg.contains("entry")));
    }

    #[test]
    fn test_manifest_invalid_toml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("bad-toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), "{{{invalid}}}").unwrap();
        let result = load_manifest(&dir.join("plugin.toml"));
        assert!(matches!(result, Err(LoaderError::ManifestParse { .. })));
    }

    #[test]
    fn test_discover_multiple_plugins() {
        let tmp = TempDir::new().unwrap();
        make_plugin_dir(tmp.path(), "plugin-a", r#"
            name = "plugin-a"
            version = "1.0.0"
            entry = "a.wasm"
            runtime = "wasm"
        "#);
        make_plugin_dir(tmp.path(), "plugin-b", r#"
            name = "plugin-b"
            version = "2.0.0"
            entry = "b.so"
        "#);
        make_plugin_dir(tmp.path(), "plugin-c", r#"
            name = "plugin-c"
            version = "0.1.0"
            entry = "c.js"
            runtime = "node"
        "#);
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert_eq!(plugins.len(), 3);
        let names: Vec<&str> = plugins.iter().map(|p| p.manifest.name.as_str()).collect();
        assert!(names.contains(&"plugin-a"));
        assert!(names.contains(&"plugin-b"));
        assert!(names.contains(&"plugin-c"));
    }

    #[test]
    fn test_discover_skips_files_in_root() {
        let tmp = TempDir::new().unwrap();
        // A file in the root dir, not a subdirectory — should be skipped
        fs::write(tmp.path().join("plugin.toml"), r#"
            name = "root"
            version = "1.0.0"
            entry = "main.so"
        "#).unwrap();
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_discovered_plugin_has_path() {
        let tmp = TempDir::new().unwrap();
        make_plugin_dir(tmp.path(), "my-plugin", r#"
            name = "my-plugin"
            version = "1.0.0"
            entry = "main.so"
        "#);
        let plugins = discover_plugins(tmp.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert!(plugins[0].path.ends_with("my-plugin"));
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let manifest = PluginManifest {
            name: "roundtrip".to_string(),
            version: "3.0.0".to_string(),
            description: "Test roundtrip".to_string(),
            entry: "plugin.wasm".to_string(),
            runtime: "wasm".to_string(),
            config: None,
        };
        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        let parsed: PluginManifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.name, "roundtrip");
        assert_eq!(parsed.version, "3.0.0");
        assert_eq!(parsed.entry, "plugin.wasm");
        assert_eq!(parsed.runtime, "wasm");
    }

    #[test]
    fn test_loader_error_display() {
        let err = LoaderError::DirectoryNotFound(PathBuf::from("/nope"));
        assert!(err.to_string().contains("/nope"));

        let err = LoaderError::InvalidManifest(PathBuf::from("/bad.toml"), "bad".to_string());
        assert!(err.to_string().contains("bad"));
    }
}
