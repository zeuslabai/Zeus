//! Runtime plugin registry — register, look up, and dispatch to plugins.

use crate::plugin::Plugin;
use serde_json::Value;
use tracing::{debug, warn};
use zeus_core::{Error, Result, ToolSchema};

/// Central registry for runtime plugin management.
///
/// Plugins are keyed by [`Plugin::name`]. Registering a plugin with a
/// duplicate name replaces the previous entry after calling `shutdown` on it.
///
/// # Example
///
/// ```rust,ignore
/// let mut registry = PluginRegistry::new();
/// registry.register(Box::new(MyPlugin)).await?;
///
/// // Dispatch a tool call
/// let result = registry.execute_tool("my_plugin", "do_thing", args).await?;
/// ```
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
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin, calling `init()` on it first.
    ///
    /// If a plugin with the same name is already registered, `shutdown()` is
    /// called on the old one before it is replaced.
    pub async fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<()> {
        let name = plugin.name().to_string();

        // Shutdown & evict any existing plugin with this name
        if let Some(pos) = self.plugins.iter().position(|p| p.name() == name) {
            debug!("replacing existing plugin: {name}");
            let old = self.plugins.remove(pos);
            if let Err(e) = old.shutdown().await {
                warn!("error shutting down replaced plugin {name}: {e}");
            }
        }

        plugin.init().await?;
        debug!("registered plugin: {name} v{}", plugin.version());
        self.plugins.push(plugin);
        Ok(())
    }

    /// Unregister a plugin by name, calling `shutdown()` on it.
    ///
    /// Returns `true` if a plugin was found and removed.
    pub async fn unregister(&mut self, name: &str) -> bool {
        if let Some(pos) = self.plugins.iter().position(|p| p.name() == name) {
            let plugin = self.plugins.remove(pos);
            if let Err(e) = plugin.shutdown().await {
                warn!("error shutting down plugin {name}: {e}");
            }
            debug!("unregistered plugin: {name}");
            return true;
        }
        false
    }

    /// Get a reference to a registered plugin by name.
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

    /// Collect all tool schemas from every plugin.
    ///
    /// Tool names are namespaced as `<plugin_name>/<tool_name>` to avoid
    /// collisions across plugins.
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

    /// Execute a tool on a named plugin.
    pub async fn execute_tool(
        &self,
        plugin_name: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<String> {
        let plugin = self
            .get(plugin_name)
            .ok_or_else(|| Error::Skill(format!("Plugin not found: {plugin_name}")))?;
        plugin.execute_tool(tool_name, args).await
    }

    /// Fire a lifecycle hook on all subscribed plugins.
    pub async fn fire_hook(&self, event: &str, context: &Value) -> Result<()> {
        for plugin in &self.plugins {
            if plugin.hook_events().iter().any(|e| e == event) {
                plugin.on_hook_event(event, context).await?;
            }
        }
        Ok(())
    }

    /// Shutdown all plugins and clear the registry.
    pub async fn shutdown_all(&mut self) -> Result<()> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.shutdown().await {
                warn!("error shutting down plugin {}: {e}", plugin.name());
            }
        }
        self.plugins.clear();
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
    use crate::plugin::Plugin;
    use async_trait::async_trait;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct MockPlugin {
        name: String,
        init_count: Arc<AtomicUsize>,
        shutdown_count: Arc<AtomicUsize>,
    }

    impl MockPlugin {
        fn new(name: &str) -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let init = Arc::new(AtomicUsize::new(0));
            let shutdown = Arc::new(AtomicUsize::new(0));
            (
                Self { name: name.to_string(), init_count: init.clone(), shutdown_count: shutdown.clone() },
                init,
                shutdown,
            )
        }
    }

    #[async_trait]
    impl Plugin for MockPlugin {
        fn name(&self) -> &str { &self.name }
        fn version(&self) -> &str { "0.1.0" }
        fn tools(&self) -> Vec<ToolSchema> {
            vec![ToolSchema::new("ping", "Ping tool")]
        }
        async fn execute_tool(&self, name: &str, _args: Value) -> Result<String> {
            match name {
                "ping" => Ok("pong".to_string()),
                _ => Err(Error::Skill(format!("Unknown tool: {name}"))),
            }
        }
        async fn init(&self) -> Result<()> {
            self.init_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&self) -> Result<()> {
            self.shutdown_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_register_and_get() {
        let mut reg = PluginRegistry::new();
        let (plugin, init, _) = MockPlugin::new("alpha");
        reg.register(Box::new(plugin)).await.unwrap();
        assert_eq!(init.load(Ordering::SeqCst), 1);
        assert_eq!(reg.len(), 1);
        assert!(reg.get("alpha").is_some());
    }

    #[tokio::test]
    async fn test_replace_calls_shutdown_on_old() {
        let mut reg = PluginRegistry::new();
        let (p1, _, shutdown1) = MockPlugin::new("alpha");
        let (p2, _, _) = MockPlugin::new("alpha");
        reg.register(Box::new(p1)).await.unwrap();
        reg.register(Box::new(p2)).await.unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(shutdown1.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_unregister() {
        let mut reg = PluginRegistry::new();
        let (p, _, shutdown) = MockPlugin::new("alpha");
        reg.register(Box::new(p)).await.unwrap();
        assert!(reg.unregister("alpha").await);
        assert_eq!(shutdown.load(Ordering::SeqCst), 1);
        assert!(reg.is_empty());
        assert!(!reg.unregister("alpha").await);
    }

    #[tokio::test]
    async fn test_all_schemas_namespaced() {
        let mut reg = PluginRegistry::new();
        let (p1, _, _) = MockPlugin::new("alpha");
        let (p2, _, _) = MockPlugin::new("beta");
        reg.register(Box::new(p1)).await.unwrap();
        reg.register(Box::new(p2)).await.unwrap();
        let schemas = reg.all_schemas();
        assert_eq!(schemas.len(), 2);
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha/ping"));
        assert!(names.contains(&"beta/ping"));
    }

    #[tokio::test]
    async fn test_execute_tool() {
        let mut reg = PluginRegistry::new();
        let (p, _, _) = MockPlugin::new("alpha");
        reg.register(Box::new(p)).await.unwrap();
        let res = reg.execute_tool("alpha", "ping", Value::Null).await.unwrap();
        assert_eq!(res, "pong");
    }

    #[tokio::test]
    async fn test_execute_tool_not_found() {
        let reg = PluginRegistry::new();
        assert!(reg.execute_tool("ghost", "ping", Value::Null).await.is_err());
    }

    #[tokio::test]
    async fn test_shutdown_all() {
        let mut reg = PluginRegistry::new();
        let (p1, _, s1) = MockPlugin::new("alpha");
        let (p2, _, s2) = MockPlugin::new("beta");
        reg.register(Box::new(p1)).await.unwrap();
        reg.register(Box::new(p2)).await.unwrap();
        reg.shutdown_all().await.unwrap();
        assert!(reg.is_empty());
        assert_eq!(s1.load(Ordering::SeqCst), 1);
        assert_eq!(s2.load(Ordering::SeqCst), 1);
    }
}
