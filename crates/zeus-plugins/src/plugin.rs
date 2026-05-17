//! Core Plugin trait for extending Zeus with custom tools and behavior.
//!
//! Implement [`Plugin`] to expose named tools to the agent at runtime.
//! Plugins can also subscribe to lifecycle hook events fired by the system.

use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Result, ToolSchema};

/// A Zeus plugin — a named, versioned unit that provides tools and/or reacts
/// to lifecycle hooks.
///
/// # Implementing a Plugin
///
/// ```rust,ignore
/// use async_trait::async_trait;
/// use zeus_plugins::Plugin;
/// use zeus_core::{Result, ToolSchema};
/// use serde_json::Value;
///
/// struct GreeterPlugin;
///
/// #[async_trait]
/// impl Plugin for GreeterPlugin {
///     fn name(&self) -> &str { "greeter" }
///     fn version(&self) -> &str { "0.1.0" }
///
///     async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
///         match name {
///             "greet" => Ok(format!("Hello, {}!", args["name"].as_str().unwrap_or("world"))),
///             _ => Err(zeus_core::Error::Skill(format!("Unknown tool: {name}"))),
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique identifier for this plugin. Must be stable across restarts.
    fn name(&self) -> &str;

    /// Semantic version string (e.g. `"1.0.0"`).
    fn version(&self) -> &str;

    /// Human-readable description shown in listings.
    fn description(&self) -> &str {
        ""
    }

    /// Tool schemas exposed by this plugin for agent discovery.
    fn tools(&self) -> Vec<ToolSchema> {
        vec![]
    }

    /// Execute a named tool with JSON arguments. Called by the registry
    /// dispatcher.
    async fn execute_tool(&self, name: &str, args: Value) -> Result<String>;

    /// Hook event names this plugin subscribes to (e.g. `"on_tool_executed"`).
    fn hook_events(&self) -> Vec<String> {
        vec![]
    }

    /// Called when a subscribed hook event fires.
    async fn on_hook_event(&self, _event: &str, _context: &Value) -> Result<()> {
        Ok(())
    }

    /// Called once after the plugin is registered. Use for resource setup.
    async fn init(&self) -> Result<()> {
        Ok(())
    }

    /// Called once before the plugin is removed. Use for clean shutdown.
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Minimal plugin implementation for testing the trait contract.
    struct TestPlugin {
        name: String,
        version: String,
        description: String,
        tools: Vec<ToolSchema>,
        hook_events: Vec<String>,
        init_called: Arc<AtomicUsize>,
        shutdown_called: Arc<AtomicUsize>,
    }

    impl TestPlugin {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: "Test plugin".to_string(),
                tools: vec![ToolSchema::new("greet", "Say hello")],
                hook_events: vec!["on_message".to_string()],
                init_called: Arc::new(AtomicUsize::new(0)),
                shutdown_called: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        fn name(&self) -> &str { &self.name }
        fn version(&self) -> &str { &self.version }
        fn description(&self) -> &str { &self.description }
        fn tools(&self) -> Vec<ToolSchema> { self.tools.clone() }
        fn hook_events(&self) -> Vec<String> { self.hook_events.clone() }

        async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
            match name {
                "greet" => {
                    let who = args.get("name").and_then(|v| v.as_str()).unwrap_or("world");
                    Ok(format!("Hello, {}!", who))
                }
                _ => Err(zeus_core::Error::Skill(format!("Unknown tool: {name}"))),
            }
        }

        async fn init(&self) -> Result<()> {
            self.init_called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn shutdown(&self) -> Result<()> {
            self.shutdown_called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_plugin_name_and_version() {
        let p = TestPlugin::new("test-plugin");
        assert_eq!(p.name(), "test-plugin");
        assert_eq!(p.version(), "1.0.0");
    }

    #[tokio::test]
    async fn test_plugin_description() {
        let p = TestPlugin::new("test-plugin");
        assert_eq!(p.description(), "Test plugin");
    }

    #[tokio::test]
    async fn test_plugin_tools() {
        let p = TestPlugin::new("test-plugin");
        let tools = p.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "greet");
    }

    #[tokio::test]
    async fn test_plugin_execute_tool_success() {
        let p = TestPlugin::new("test-plugin");
        let result = p.execute_tool("greet", serde_json::json!({"name": "Zeus"})).await;
        assert_eq!(result.unwrap(), "Hello, Zeus!");
    }

    #[tokio::test]
    async fn test_plugin_execute_tool_default_name() {
        let p = TestPlugin::new("test-plugin");
        let result = p.execute_tool("greet", serde_json::json!({})).await;
        assert_eq!(result.unwrap(), "Hello, world!");
    }

    #[tokio::test]
    async fn test_plugin_execute_tool_unknown() {
        let p = TestPlugin::new("test-plugin");
        let result = p.execute_tool("nonexistent", Value::Null).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_hook_events() {
        let p = TestPlugin::new("test-plugin");
        let events = p.hook_events();
        assert_eq!(events, vec!["on_message"]);
    }

    #[tokio::test]
    async fn test_plugin_on_hook_event() {
        let p = TestPlugin::new("test-plugin");
        // Default implementation should return Ok
        let result = p.on_hook_event("on_message", &Value::Null).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_init_called() {
        let p = TestPlugin::new("test-plugin");
        assert_eq!(p.init_called.load(Ordering::SeqCst), 0);
        p.init().await.unwrap();
        assert_eq!(p.init_called.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_plugin_shutdown_called() {
        let p = TestPlugin::new("test-plugin");
        assert_eq!(p.shutdown_called.load(Ordering::SeqCst), 0);
        p.shutdown().await.unwrap();
        assert_eq!(p.shutdown_called.load(Ordering::SeqCst), 1);
    }

    /// Plugin with no tools and no hooks — tests default implementations.
    struct MinimalPlugin;

    #[async_trait]
    impl Plugin for MinimalPlugin {
        fn name(&self) -> &str { "minimal" }
        fn version(&self) -> &str { "0.1.0" }
        async fn execute_tool(&self, name: &str, _args: Value) -> Result<String> {
            Err(zeus_core::Error::Skill(format!("No tools: {name}")))
        }
    }

    #[tokio::test]
    async fn test_minimal_plugin_defaults() {
        let p = MinimalPlugin;
        assert_eq!(p.name(), "minimal");
        assert_eq!(p.version(), "0.1.0");
        assert_eq!(p.description(), "");
        assert!(p.tools().is_empty());
        assert!(p.hook_events().is_empty());
        assert!(p.init().await.is_ok());
        assert!(p.shutdown().await.is_ok());
        assert!(p.on_hook_event("anything", &Value::Null).await.is_ok());
    }

    #[tokio::test]
    async fn test_minimal_plugin_execute_errors() {
        let p = MinimalPlugin;
        let result = p.execute_tool("anything", Value::Null).await;
        assert!(result.is_err());
    }
}
