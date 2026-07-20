//! Browser automation tools for the Zeus agent tool registry.
//!
//! Each tool wraps a high-level action on the shared [`CdpClient`] and implements
//! the `BrowserTool` trait so it can be registered in the `BrowserRegistry`.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use zeus_core::{Error, Result, ToolSchema};

use crate::cdp::CdpClient;
use crate::stealth::StealthConfig;

/// Shared browser client handle.
pub type SharedBrowser = Arc<Mutex<CdpClient>>;

// ============================================================================
// BrowserTool trait (mirrors TalosTool)
// ============================================================================

/// Trait for browser automation tools.
#[async_trait]
pub trait BrowserTool: Send + Sync {
    /// Tool name as registered in the tool registry.
    fn name(&self) -> &'static str;

    /// Human-readable description.
    fn description(&self) -> &'static str;

    /// JSON schema describing the tool's parameters.
    fn schema(&self) -> ToolSchema;

    /// Execute the tool with the given JSON arguments.
    async fn execute(&self, args: Value) -> Result<String>;
}

// ============================================================================
// Individual tools
// ============================================================================

/// Connect to a Chrome browser (or specific tab) for CDP automation.
pub struct BrowserConnectTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserConnectTool {
    fn name(&self) -> &'static str {
        "browser_connect"
    }

    fn description(&self) -> &'static str {
        "Connect to a Chrome browser for automation. Chrome must be running with --remote-debugging-port."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "tab_id",
                "string",
                "Specific tab ID to connect to (optional, connects to first page tab if omitted)",
                false,
            )
            .with_param(
                "url",
                "string",
                "Chrome debug URL (default: http://localhost:9222)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let tab_id = args.get("tab_id").and_then(|v| v.as_str());

        // If a custom debug URL is provided, create a new client
        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            let mut browser = self.browser.lock().await;
            *browser = CdpClient::new(url);
        }

        let mut browser = self.browser.lock().await;
        browser.connect(tab_id).await?;

        let tab = browser.current_tab_id().unwrap_or("unknown").to_string();
        Ok(format!("Connected to tab: {}", tab))
    }
}

/// Navigate the browser to a URL.
pub struct BrowserNavigateTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserNavigateTool {
    fn name(&self) -> &'static str {
        "browser_navigate"
    }

    fn description(&self) -> &'static str {
        "Navigate the browser to a URL"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "url",
            "string",
            "URL to navigate to",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: url".to_string()))?;

        let browser = self.browser.lock().await;
        let result = browser.navigate(url).await?;
        Ok(format!("Navigated to {}. Response: {}", url, result))
    }
}

/// Click an element by CSS selector.
pub struct BrowserClickTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserClickTool {
    fn name(&self) -> &'static str {
        "browser_click"
    }

    fn description(&self) -> &'static str {
        "Click an element on the page by CSS selector"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "selector",
            "string",
            "CSS selector of the element to click",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let selector = args
            .get("selector")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: selector".to_string()))?;

        let browser = self.browser.lock().await;
        browser.click(selector).await?;
        Ok(format!("Clicked element: {}", selector))
    }
}

/// Type text into the focused element.
pub struct BrowserTypeTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserTypeTool {
    fn name(&self) -> &'static str {
        "browser_type"
    }

    fn description(&self) -> &'static str {
        "Type text into the currently focused element or focus an element first by selector"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Text to type", true)
            .with_param(
                "selector",
                "string",
                "CSS selector to focus before typing (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: text".to_string()))?;

        let browser = self.browser.lock().await;

        // If a selector is given, focus that element first
        if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
            let focus_script = format!(
                "document.querySelector('{}')?.focus()",
                selector.replace('\'', "\\'").replace('\\', "\\\\"),
            );
            browser.evaluate(&focus_script).await?;
        }

        browser.type_text(text).await?;
        Ok(format!("Typed {} characters", text.len()))
    }
}

/// Take a screenshot of the current page.
pub struct BrowserScreenshotTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserScreenshotTool {
    fn name(&self) -> &'static str {
        "browser_screenshot"
    }

    fn description(&self) -> &'static str {
        "Take a screenshot of the current page (returns base64 PNG or saves to file)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "output",
            "string",
            "File path to save the screenshot to (optional, returns base64 if omitted)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let browser = self.browser.lock().await;
        let b64 = browser.screenshot().await?;

        if let Some(output_path) = args.get("output").and_then(|v| v.as_str()) {
            // Decode and save to file
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .map_err(|e| Error::Tool(format!("Failed to decode screenshot: {}", e)))?;
            tokio::fs::write(output_path, &bytes).await.map_err(|e| {
                Error::Tool(format!(
                    "Failed to write screenshot to {}: {}",
                    output_path, e
                ))
            })?;
            Ok(format!(
                "Screenshot saved to {} ({} bytes)",
                output_path,
                bytes.len()
            ))
        } else {
            Ok(format!("data:image/png;base64,{}", b64))
        }
    }
}

/// Get the text content of the page or a specific element.
pub struct BrowserGetTextTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserGetTextTool {
    fn name(&self) -> &'static str {
        "browser_get_text"
    }

    fn description(&self) -> &'static str {
        "Get the text content of the page body or a specific element"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "selector",
            "string",
            "CSS selector to get text from (optional, defaults to body)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let browser = self.browser.lock().await;

        if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
            browser.get_element_text(selector).await
        } else {
            browser.get_text().await
        }
    }
}

/// Execute JavaScript in the browser.
pub struct BrowserExecuteJsTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserExecuteJsTool {
    fn name(&self) -> &'static str {
        "browser_execute_js"
    }

    fn description(&self) -> &'static str {
        "Execute JavaScript in the browser page and return the result"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "expression",
            "string",
            "JavaScript expression to evaluate",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let expression = args
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: expression".to_string()))?;

        let browser = self.browser.lock().await;
        let result = browser.evaluate(expression).await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
    }
}

/// Get a snapshot of the page DOM.
pub struct BrowserPageSnapshotTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserPageSnapshotTool {
    fn name(&self) -> &'static str {
        "browser_page_snapshot"
    }

    fn description(&self) -> &'static str {
        "Get a snapshot of the page HTML (truncated to ~50KB for context)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let browser = self.browser.lock().await;
        browser.page_snapshot().await
    }
}

/// List all open browser tabs.
pub struct BrowserListTabsTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserListTabsTool {
    fn name(&self) -> &'static str {
        "browser_list_tabs"
    }

    fn description(&self) -> &'static str {
        "List all open browser tabs with their titles and URLs"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let browser = self.browser.lock().await;
        let tabs = browser.list_tabs().await?;

        let mut output = String::new();
        for tab in &tabs {
            output.push_str(&format!(
                "[{}] {} - {} ({})\n",
                tab.target_type, tab.id, tab.title, tab.url
            ));
        }

        if output.is_empty() {
            Ok("No tabs found".to_string())
        } else {
            Ok(output.trim_end().to_string())
        }
    }
}

/// Open a new browser tab.
pub struct BrowserNewTabTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserNewTabTool {
    fn name(&self) -> &'static str {
        "browser_new_tab"
    }

    fn description(&self) -> &'static str {
        "Open a new browser tab, optionally navigating to a URL"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "url",
            "string",
            "URL to open in the new tab (optional)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let url = args.get("url").and_then(|v| v.as_str());
        let browser = self.browser.lock().await;
        let tab = browser.new_tab(url).await?;
        Ok(format!(
            "New tab created: {} ({}) - {}",
            tab.id, tab.title, tab.url
        ))
    }
}

/// Close a browser tab by ID.
pub struct BrowserCloseTabTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserCloseTabTool {
    fn name(&self) -> &'static str {
        "browser_close_tab"
    }

    fn description(&self) -> &'static str {
        "Close a browser tab by its ID"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "tab_id",
            "string",
            "ID of the tab to close",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let tab_id = args
            .get("tab_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: tab_id".to_string()))?;

        let browser = self.browser.lock().await;
        browser.close_tab(tab_id).await?;
        Ok(format!("Tab {} closed", tab_id))
    }
}

/// Enable stealth mode to evade bot detection (reCAPTCHA v3, Cloudflare, etc.).
pub struct BrowserEnableStealthTool {
    pub browser: SharedBrowser,
}

#[async_trait]
impl BrowserTool for BrowserEnableStealthTool {
    fn name(&self) -> &'static str {
        "browser_enable_stealth"
    }

    fn description(&self) -> &'static str {
        "Enable stealth mode to evade bot detection (reCAPTCHA v3, Cloudflare, etc.). Must be called before navigating to a page."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "user_agent",
                "string",
                "Custom User-Agent string (optional, uses realistic default if omitted)",
                false,
            )
            .with_param(
                "platform",
                "string",
                "Platform string like 'MacIntel' or 'Win32' (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mut config = StealthConfig::default();

        if let Some(ua) = args.get("user_agent").and_then(|v| v.as_str()) {
            config.user_agent = Some(ua.to_string());
        }

        if let Some(platform) = args.get("platform").and_then(|v| v.as_str()) {
            config.platform = Some(platform.to_string());
        }

        let browser = self.browser.lock().await;
        browser.enable_stealth(&config).await?;

        Ok("Stealth mode enabled: navigator.webdriver overridden, chrome runtime mocked, plugins/mimeTypes injected, WebGL spoofed".to_string())
    }
}

// ============================================================================
// Registry
// ============================================================================

/// Registry of all browser automation tools.
pub struct BrowserRegistry {
    tools: std::collections::HashMap<String, Box<dyn BrowserTool>>,
}

impl BrowserRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    /// Create a registry with all browser tools using the given shared browser client.
    pub fn with_tools(browser: SharedBrowser) -> Self {
        let mut registry = Self::new();

        registry.register(Box::new(BrowserConnectTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserNavigateTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserClickTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserTypeTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserScreenshotTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserGetTextTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserExecuteJsTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserPageSnapshotTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserListTabsTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserNewTabTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserCloseTabTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(BrowserEnableStealthTool {
            browser: browser.clone(),
        }));
        registry.register(Box::new(crate::meet::GoogleMeetTool::new(browser.clone())));

        registry
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn BrowserTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn BrowserTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Execute a tool by name.
    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| Error::Tool(format!("Browser tool not found: {}", name)))?;
        tool.execute(args).await
    }

    /// Get all tool schemas.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// List all tool names.
    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool count.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for BrowserRegistry {
    fn default() -> Self {
        let browser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        Self::with_tools(browser)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_browser() -> SharedBrowser {
        Arc::new(Mutex::new(CdpClient::new("http://localhost:9222")))
    }

    #[test]
    fn test_browser_connect_schema() {
        let tool = BrowserConnectTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_connect");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("tab_id"));
        assert!(props.contains_key("url"));
        // No required params
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty());
    }

    #[test]
    fn test_browser_navigate_schema() {
        let tool = BrowserNavigateTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_navigate");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("url"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&Value::String("url".to_string())));
    }

    #[test]
    fn test_browser_click_schema() {
        let tool = BrowserClickTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_click");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("selector"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&Value::String("selector".to_string())));
    }

    #[test]
    fn test_browser_type_schema() {
        let tool = BrowserTypeTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_type");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("text"));
        assert!(props.contains_key("selector"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&Value::String("text".to_string())));
        assert!(!required.contains(&Value::String("selector".to_string())));
    }

    #[test]
    fn test_browser_screenshot_schema() {
        let tool = BrowserScreenshotTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_screenshot");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("output"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty());
    }

    #[test]
    fn test_browser_get_text_schema() {
        let tool = BrowserGetTextTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_get_text");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("selector"));
    }

    #[test]
    fn test_browser_execute_js_schema() {
        let tool = BrowserExecuteJsTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_execute_js");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("expression"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&Value::String("expression".to_string())));
    }

    #[test]
    fn test_browser_page_snapshot_schema() {
        let tool = BrowserPageSnapshotTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_page_snapshot");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.is_empty());
    }

    #[test]
    fn test_browser_list_tabs_schema() {
        let tool = BrowserListTabsTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_list_tabs");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.is_empty());
    }

    #[test]
    fn test_browser_new_tab_schema() {
        let tool = BrowserNewTabTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_new_tab");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("url"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty());
    }

    #[test]
    fn test_browser_close_tab_schema() {
        let tool = BrowserCloseTabTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_close_tab");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("tab_id"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&Value::String("tab_id".to_string())));
    }

    #[test]
    fn test_registry_creation() {
        let registry = BrowserRegistry::with_tools(make_browser());
        assert_eq!(registry.len(), 13);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_registry_default() {
        let registry = BrowserRegistry::default();
        assert_eq!(registry.len(), 13);
    }

    #[test]
    fn test_registry_schemas() {
        let registry = BrowserRegistry::with_tools(make_browser());
        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 13);

        // Verify all expected tool names are present
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"browser_connect"));
        assert!(names.contains(&"browser_navigate"));
        assert!(names.contains(&"browser_click"));
        assert!(names.contains(&"browser_type"));
        assert!(names.contains(&"browser_screenshot"));
        assert!(names.contains(&"browser_get_text"));
        assert!(names.contains(&"browser_execute_js"));
        assert!(names.contains(&"browser_page_snapshot"));
        assert!(names.contains(&"browser_list_tabs"));
        assert!(names.contains(&"browser_new_tab"));
        assert!(names.contains(&"browser_close_tab"));
        assert!(names.contains(&"browser_enable_stealth"));
    }

    #[test]
    fn test_registry_list() {
        let registry = BrowserRegistry::with_tools(make_browser());
        let names = registry.list();
        assert_eq!(names.len(), 13);
    }

    #[test]
    fn test_browser_enable_stealth_schema() {
        let tool = BrowserEnableStealthTool {
            browser: make_browser(),
        };
        let schema = tool.schema();
        assert_eq!(schema.name, "browser_enable_stealth");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("user_agent"));
        assert!(props.contains_key("platform"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty()); // Both params are optional
    }

    #[test]
    fn test_registry_get() {
        let registry = BrowserRegistry::with_tools(make_browser());
        assert!(registry.get("browser_navigate").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_empty_registry() {
        let registry = BrowserRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_all_tools_have_unique_names() {
        let registry = BrowserRegistry::with_tools(make_browser());
        let names = registry.list();
        let mut unique: Vec<&str> = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "Tool names must be unique");
    }

    #[test]
    fn test_all_schemas_have_valid_parameters() {
        let registry = BrowserRegistry::with_tools(make_browser());
        for schema in registry.schemas() {
            // Every schema must have a valid parameters object
            assert!(
                schema.parameters.is_object(),
                "Schema for {} has non-object parameters",
                schema.name
            );
            let obj = schema.parameters.as_object().expect("should be an object");
            assert!(
                obj.contains_key("type"),
                "Schema for {} missing 'type'",
                schema.name
            );
            assert!(
                obj.contains_key("properties"),
                "Schema for {} missing 'properties'",
                schema.name
            );
            assert!(
                obj.contains_key("required"),
                "Schema for {} missing 'required'",
                schema.name
            );
            assert_eq!(
                obj["type"], "object",
                "Schema for {} type should be 'object'",
                schema.name
            );
        }
    }
}
