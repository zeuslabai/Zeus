//! Plugin discovery and loading from the filesystem.
//!
//! Each plugin lives in its own directory under `~/.zeus/plugins/` and must
//! contain a `plugin.toml` manifest describing the plugin metadata, runtime,
//! tools, and hooks.

use crate::bridge::{NodePlugin, PythonPlugin};
use crate::plugin::{Plugin, PluginRegistry};
use crate::skill_plugin::SkillPlugin;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result};

/// Discovers and loads plugins from a directory.
#[derive(Debug, Clone)]
pub struct PluginLoader {
    plugins_dir: PathBuf,
}

impl PluginLoader {
    /// Create a loader for the given directory.
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    /// Create a loader pointing at the default plugins directory (`~/.zeus/plugins/`).
    pub fn with_default_dir() -> Self {
        let plugins_dir = zeus_core::default_config_dir().join("plugins");
        Self::new(plugins_dir)
    }

    /// Return the plugins directory.
    pub fn plugins_dir(&self) -> &Path {
        &self.plugins_dir
    }

    /// Discover all plugins in the directory.
    ///
    /// Each subdirectory that contains a `plugin.toml` file is considered a
    /// plugin. The manifest is parsed and returned along with the directory path.
    pub async fn discover(&self) -> Result<Vec<PluginManifest>> {
        if !self.plugins_dir.exists() {
            std::fs::create_dir_all(&self.plugins_dir)?;
            return Ok(vec![]);
        }

        let mut manifests = Vec::new();
        let entries = std::fs::read_dir(&self.plugins_dir)
            .map_err(|e| Error::Skill(format!("Failed to read plugins dir: {}", e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("plugin.toml");
                if manifest_path.exists() {
                    match load_manifest(&manifest_path) {
                        Ok(mut manifest) => {
                            manifest.path = Some(path);
                            manifests.push(manifest);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load plugin manifest at {}: {}",
                                manifest_path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        Ok(manifests)
    }

    /// Load a single plugin from its manifest.
    pub async fn load(&self, manifest: &PluginManifest) -> Result<Box<dyn Plugin>> {
        let plugin_dir = manifest
            .path
            .as_deref()
            .ok_or_else(|| Error::Skill("Manifest has no path set".to_string()))?;

        match manifest.runtime {
            PluginRuntime::Skill => {
                // Look for SKILL.md in the plugin directory
                let skill_file = plugin_dir.join("SKILL.md");
                if !skill_file.exists() {
                    return Err(Error::Skill(format!(
                        "SKILL.md not found in {}",
                        plugin_dir.display()
                    )));
                }
                let content = std::fs::read_to_string(&skill_file)?;
                let skill = crate::parse_skill_md(&content, plugin_dir.to_path_buf())?;
                Ok(Box::new(SkillPlugin::from_skill(skill)))
            }
            PluginRuntime::Node => {
                let entry = manifest.entry.as_deref().unwrap_or("index.js");
                let entry_path = plugin_dir.join(entry);
                if !entry_path.exists() {
                    return Err(Error::Skill(format!(
                        "Node entry point not found: {}",
                        entry_path.display()
                    )));
                }
                let plugin = NodePlugin::new(manifest.clone(), plugin_dir).await?;
                Ok(Box::new(plugin))
            }
            PluginRuntime::Python => {
                let entry = manifest.entry.as_deref().unwrap_or("main.py");
                let entry_path = plugin_dir.join(entry);
                if !entry_path.exists() {
                    return Err(Error::Skill(format!(
                        "Python entry point not found: {}",
                        entry_path.display()
                    )));
                }
                let plugin = PythonPlugin::new(manifest.clone(), plugin_dir).await?;
                Ok(Box::new(plugin))
            }
            PluginRuntime::Shell => {
                // Shell plugins are backed by SkillPlugin with shell implementations
                let skill = crate::Skill {
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    version: manifest.version.clone(),
                    author: manifest.author.clone(),
                    system_prompt: String::new(),
                    tools: manifest
                        .tools
                        .iter()
                        .map(|t| crate::SkillTool {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            input_schema: t.parameters.clone(),
                            implementation: crate::ToolImplementation::Shell {
                                command: t.command.clone().unwrap_or_default(),
                            },
                        })
                        .collect(),
                    permissions: vec![],
                    path: plugin_dir.to_path_buf(),
                    raw_content: String::new(),
                    invocation: crate::SkillInvocationPolicy::default(),
                    command_dispatch: None,
                    metadata: None,
                    frontmatter: std::collections::HashMap::new(),
                    read_when: vec![],
                };
                Ok(Box::new(SkillPlugin::from_skill(skill)))
            }
        }
    }

    /// Discover and load all plugins into a new registry.
    pub async fn load_all(&self) -> Result<PluginRegistry> {
        let manifests = self.discover().await?;
        let mut registry = PluginRegistry::new();

        for manifest in &manifests {
            match self.load(manifest).await {
                Ok(plugin) => {
                    tracing::info!("Loaded plugin: {} v{}", manifest.name, manifest.version);
                    registry.register(plugin);
                }
                Err(e) => {
                    tracing::warn!("Failed to load plugin {}: {}", manifest.name, e);
                }
            }
        }

        Ok(registry)
    }
}

/// Parse a `plugin.toml` manifest file.
fn load_manifest(path: &Path) -> Result<PluginManifest> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Error::Skill(format!("Failed to read {}: {}", path.display(), e)))?;
    let manifest: PluginManifest = toml::from_str(&content)
        .map_err(|e| Error::Skill(format!("Failed to parse {}: {}", path.display(), e)))?;
    Ok(manifest)
}

/// Plugin manifest, typically read from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin name (unique identifier).
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Optional author.
    #[serde(default)]
    pub author: Option<String>,
    /// Runtime that executes this plugin.
    pub runtime: PluginRuntime,
    /// Optional entry-point file (e.g. "index.js", "main.py").
    #[serde(default)]
    pub entry: Option<String>,
    /// Tools provided by this plugin.
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    /// Hook events this plugin subscribes to.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Arbitrary extra configuration.
    #[serde(default)]
    pub config: Option<Value>,
    /// Directory path (set at discovery time, not serialized).
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// Definition of a tool in a plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool parameters.
    #[serde(default = "default_parameters")]
    pub parameters: Value,
    /// Optional shell command (for shell-runtime plugins).
    #[serde(default)]
    pub command: Option<String>,
}

fn default_parameters() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "required": []
    })
}

/// Runtime that executes a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntime {
    /// SKILL.md-based plugin.
    Skill,
    /// Node.js process.
    Node,
    /// Python process.
    Python,
    /// Shell commands.
    Shell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest_from_toml() {
        let toml_str = r#"
name = "example"
version = "1.0.0"
description = "An example plugin"
runtime = "node"
entry = "index.js"
hooks = ["on_tool_executed"]

[[tools]]
name = "hello"
description = "Say hello"

[tools.parameters]
type = "object"

[[tools]]
name = "goodbye"
description = "Say goodbye"

[tools.parameters]
type = "object"
"#;

        let manifest: PluginManifest = toml::from_str(toml_str).expect("should parse successfully");
        assert_eq!(manifest.name, "example");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description, "An example plugin");
        assert_eq!(manifest.runtime, PluginRuntime::Node);
        assert_eq!(manifest.entry, Some("index.js".to_string()));
        assert_eq!(manifest.tools.len(), 2);
        assert_eq!(manifest.tools[0].name, "hello");
        assert_eq!(manifest.tools[1].name, "goodbye");
        assert_eq!(manifest.hooks, vec!["on_tool_executed".to_string()]);
    }

    #[test]
    fn test_parse_manifest_minimal() {
        let toml_str = r#"
name = "minimal"
version = "0.1.0"
runtime = "shell"
"#;

        let manifest: PluginManifest = toml::from_str(toml_str).expect("should parse successfully");
        assert_eq!(manifest.name, "minimal");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.description, "");
        assert_eq!(manifest.runtime, PluginRuntime::Shell);
        assert!(manifest.tools.is_empty());
        assert!(manifest.hooks.is_empty());
        assert!(manifest.entry.is_none());
        assert!(manifest.config.is_none());
    }

    #[test]
    fn test_parse_manifest_all_runtimes() {
        for (runtime_str, expected) in [
            ("skill", PluginRuntime::Skill),
            ("node", PluginRuntime::Node),
            ("python", PluginRuntime::Python),
            ("shell", PluginRuntime::Shell),
        ] {
            let toml_str = format!(
                r#"
name = "test"
version = "1.0.0"
runtime = "{}"
"#,
                runtime_str
            );
            let manifest: PluginManifest =
                toml::from_str(&toml_str).expect("should parse successfully");
            assert_eq!(manifest.runtime, expected);
        }
    }

    #[tokio::test]
    async fn test_loader_discover_empty_dir() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let loader = PluginLoader::new(tmp.path().to_path_buf());
        let manifests = loader
            .discover()
            .await
            .expect("async operation should succeed");
        assert!(manifests.is_empty());
    }

    #[tokio::test]
    async fn test_loader_discover_with_plugins() {
        let tmp = tempfile::tempdir().expect("should create temp dir");

        // Create a plugin directory with a manifest
        let plugin_dir = tmp.path().join("my-plugin");
        std::fs::create_dir_all(&plugin_dir).expect("should create directory");
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
name = "my-plugin"
version = "0.1.0"
description = "Test plugin"
runtime = "shell"

[[tools]]
name = "echo"
description = "Echo a message"
command = "echo hello"
"#,
        )
        .expect("operation should succeed");

        let loader = PluginLoader::new(tmp.path().to_path_buf());
        let manifests = loader
            .discover()
            .await
            .expect("async operation should succeed");

        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "my-plugin");
        assert_eq!(manifests[0].version, "0.1.0");
        assert!(manifests[0].path.is_some());
    }

    #[tokio::test]
    async fn test_loader_discover_nonexistent_dir() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let loader = PluginLoader::new(tmp.path().join("nonexistent"));
        let manifests = loader
            .discover()
            .await
            .expect("async operation should succeed");
        assert!(manifests.is_empty());
    }
}
