//! Plugin trait and registry for extending Zeus with custom tools and behavior.

use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// Plugin trait for extending Zeus with custom tools and behavior.
///
/// Plugins provide named tools that can be discovered and executed at runtime.
/// They can also subscribe to hook events to react to system-wide actions.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique name of this plugin.
    fn name(&self) -> &str;

    /// Semantic version string.
    fn version(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str {
        ""
    }

    /// Tool schemas provided by this plugin.
    fn tools(&self) -> Vec<ToolSchema> {
        vec![]
    }

    /// Execute a tool by name with the given JSON arguments.
    async fn execute_tool(&self, name: &str, args: Value) -> Result<String>;

    /// Hook event types this plugin subscribes to (e.g. "on_tool_executed").
    fn hook_events(&self) -> Vec<String> {
        vec![]
    }

    /// Called when a subscribed hook event fires.
    async fn on_hook_event(&self, _event: &str, _context: &Value) -> Result<()> {
        Ok(())
    }

    /// Initialize the plugin. Called once after registration.
    async fn init(&self) -> Result<()> {
        Ok(())
    }

    /// Shutdown the plugin. Called once before removal.
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// Registry for runtime plugin management.
///
/// Holds a collection of plugins that can be looked up by name, and provides
/// aggregate operations like collecting all tool schemas or dispatching tool
/// execution to the correct plugin.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.plugins.iter().map(|p| p.name()).collect();
        f.debug_struct("PluginRegistry")
            .field("plugins", &names)
            .finish()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    /// Create an empty plugin registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin. If a plugin with the same name already exists it is
    /// replaced silently.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        // Remove existing plugin with the same name, if any
        let name = plugin.name().to_string();
        self.plugins.retain(|p| p.name() != name);
        self.plugins.push(plugin);
    }

    /// Unregister a plugin by name. Returns `true` if a plugin was removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|p| p.name() != name);
        self.plugins.len() < before
    }

    /// Get a reference to a plugin by name.
    pub fn get(&self, name: &str) -> Option<&dyn Plugin> {
        self.plugins
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
    }

    /// List all registered plugins.
    pub fn list(&self) -> Vec<&dyn Plugin> {
        self.plugins.iter().map(|p| p.as_ref()).collect()
    }

    /// Collect tool schemas from every registered plugin.
    ///
    /// Tool names are prefixed with `<plugin_name>/` to avoid collisions.
    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        let mut schemas = Vec::new();
        for plugin in &self.plugins {
            for mut schema in plugin.tools() {
                schema.name = format!("{}/{}", plugin.name(), schema.name);
                schemas.push(schema);
            }
        }
        schemas
    }

    /// Execute a tool on a specific plugin.
    pub async fn execute_tool(
        &self,
        plugin_name: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<String> {
        let plugin = self
            .get(plugin_name)
            .ok_or_else(|| Error::Skill(format!("Plugin not found: {}", plugin_name)))?;
        plugin.execute_tool(tool_name, args).await
    }

    /// Initialize all registered plugins.
    pub async fn init_all(&self) -> Result<()> {
        for plugin in &self.plugins {
            plugin.init().await?;
        }
        Ok(())
    }

    /// Shutdown all registered plugins.
    pub async fn shutdown_all(&self) -> Result<()> {
        for plugin in &self.plugins {
            plugin.shutdown().await?;
        }
        Ok(())
    }

    /// Fire a hook event on all plugins that subscribe to it.
    pub async fn fire_hook(&self, event: &str, context: &Value) -> Result<()> {
        for plugin in &self.plugins {
            if plugin.hook_events().iter().any(|e| e == event) {
                plugin.on_hook_event(event, context).await?;
            }
        }
        Ok(())
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A simple test plugin for use in unit tests.
    struct TestPlugin {
        name: String,
        version: String,
        initialized: Arc<AtomicBool>,
        shutdown_called: Arc<AtomicBool>,
    }

    impl TestPlugin {
        fn new(name: &str, version: &str) -> Self {
            Self {
                name: name.to_string(),
                version: version.to_string(),
                initialized: Arc::new(AtomicBool::new(false)),
                shutdown_called: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn version(&self) -> &str {
            &self.version
        }

        fn description(&self) -> &str {
            "A test plugin"
        }

        fn tools(&self) -> Vec<ToolSchema> {
            vec![ToolSchema::new("greet", "Say hello")]
        }

        async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
            match name {
                "greet" => {
                    let who = args.get("name").and_then(|v| v.as_str()).unwrap_or("world");
                    Ok(format!("Hello, {}!", who))
                }
                _ => Err(Error::Skill(format!("Unknown tool: {}", name))),
            }
        }

        fn hook_events(&self) -> Vec<String> {
            vec!["on_tool_executed".to_string()]
        }

        async fn on_hook_event(&self, _event: &str, _context: &Value) -> Result<()> {
            Ok(())
        }

        async fn init(&self) -> Result<()> {
            self.initialized.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn shutdown(&self) -> Result<()> {
            self.shutdown_called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_registry_new_is_empty() {
        let registry = PluginRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        let plugin = registry.get("alpha").expect("key should exist");
        assert_eq!(plugin.name(), "alpha");
        assert_eq!(plugin.version(), "1.0.0");
        assert_eq!(plugin.description(), "A test plugin");
    }

    #[test]
    fn test_registry_register_replaces_duplicate() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));
        registry.register(Box::new(TestPlugin::new("alpha", "2.0.0")));

        assert_eq!(registry.len(), 1);
        let plugin = registry.get("alpha").expect("key should exist");
        assert_eq!(plugin.version(), "2.0.0");
    }

    #[test]
    fn test_registry_unregister() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));
        registry.register(Box::new(TestPlugin::new("beta", "1.0.0")));
        assert_eq!(registry.len(), 2);

        assert!(registry.unregister("alpha"));
        assert_eq!(registry.len(), 1);
        assert!(registry.get("alpha").is_none());
        assert!(registry.get("beta").is_some());

        // Unregistering non-existent returns false
        assert!(!registry.unregister("gamma"));
    }

    #[test]
    fn test_registry_list() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));
        registry.register(Box::new(TestPlugin::new("beta", "2.0.0")));

        let list = registry.list();
        assert_eq!(list.len(), 2);
        let names: Vec<&str> = list.iter().map(|p| p.name()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn test_registry_all_schemas() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));
        registry.register(Box::new(TestPlugin::new("beta", "1.0.0")));

        let schemas = registry.all_schemas();
        assert_eq!(schemas.len(), 2);
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha/greet"));
        assert!(names.contains(&"beta/greet"));
    }

    #[tokio::test]
    async fn test_registry_execute_tool() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));

        let result = registry
            .execute_tool("alpha", "greet", serde_json::json!({"name": "Zeus"}))
            .await
            .expect("async operation should succeed");
        assert_eq!(result, "Hello, Zeus!");
    }

    #[tokio::test]
    async fn test_registry_execute_tool_not_found() {
        let registry = PluginRegistry::new();
        let result = registry
            .execute_tool("nonexistent", "greet", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_registry_init_all() {
        let init_flag = Arc::new(AtomicBool::new(false));
        let plugin = TestPlugin {
            name: "alpha".to_string(),
            version: "1.0.0".to_string(),
            initialized: init_flag.clone(),
            shutdown_called: Arc::new(AtomicBool::new(false)),
        };

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(plugin));
        registry
            .init_all()
            .await
            .expect("async operation should succeed");

        assert!(init_flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_registry_shutdown_all() {
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let plugin = TestPlugin {
            name: "alpha".to_string(),
            version: "1.0.0".to_string(),
            initialized: Arc::new(AtomicBool::new(false)),
            shutdown_called: shutdown_flag.clone(),
        };

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(plugin));
        registry
            .shutdown_all()
            .await
            .expect("async operation should succeed");

        assert!(shutdown_flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_registry_fire_hook() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("alpha", "1.0.0")));

        // Should not error - the TestPlugin subscribes to "on_tool_executed"
        registry
            .fire_hook("on_tool_executed", &serde_json::json!({"tool": "greet"}))
            .await
            .expect("async operation should succeed");
    }

    #[test]
    fn test_registry_default() {
        let registry = PluginRegistry::default();
        assert!(registry.is_empty());
    }
}
