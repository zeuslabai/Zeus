//! Zeus Browser - Chrome DevTools Protocol browser automation
//!
//! Provides a CDP client and tool wrappers for browser automation,
//! allowing the Zeus agent to control Chrome/Chromium programmatically.
//!
//! # Architecture
//!
//! - [`cdp::CdpClient`] - Low-level CDP WebSocket client
//! - [`tools`] - Tool wrappers implementing [`tools::BrowserTool`] for the agent registry
//! - [`BrowserRegistry`] - Collection of all browser tools with a shared client
//! - [`page_cache`] - LRU page content cache with TTL and eviction
//!
//! # Usage
//!
//! ```rust,no_run
//! use zeus_browser::{create_browser_tools, BrowserRegistry};
//!
//! // Create tools with default debug URL
//! let (schemas, browser) = create_browser_tools("http://localhost:9222");
//!
//! // Or use the registry directly
//! let registry = BrowserRegistry::default();
//! let schemas = registry.schemas();
//! ```

pub mod ai_agent;
pub mod cdp;
pub mod meet;
pub mod page_cache;
pub mod stealth;
pub mod tools;

pub use ai_agent::{BrowserAgent, ExtractionResult, PageLink, PageSnapshot};
pub use cdp::{CdpClient, CdpCommand, CdpError, CdpResponse, TabInfo};
pub use page_cache::{CacheEntry, CachePolicy, CacheRule, CacheStats, ContentType, PageCache};
pub use stealth::StealthConfig;
pub use tools::{
    BrowserClickTool, BrowserCloseTabTool, BrowserConnectTool, BrowserEnableStealthTool,
    BrowserExecuteJsTool, BrowserGetTextTool, BrowserListTabsTool, BrowserNavigateTool,
    BrowserNewTabTool, BrowserPageSnapshotTool, BrowserRegistry, BrowserScreenshotTool,
    BrowserTool, BrowserTypeTool, SharedBrowser,
};

use std::sync::Arc;
use tokio::sync::Mutex;
use zeus_core::ToolSchema;

/// Create browser tools with a shared client targeting the given debug URL.
///
/// Returns a tuple of (tool schemas, shared browser handle). The schemas can be
/// registered with the agent's tool registry, and the browser handle can be used
/// to execute tools.
///
/// # Arguments
///
/// * `debug_url` - Chrome's debugging URL, e.g. `"http://localhost:9222"`
pub fn create_browser_tools(debug_url: &str) -> (Vec<ToolSchema>, SharedBrowser) {
    let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::new(debug_url)));
    let registry = BrowserRegistry::with_tools(browser.clone());
    (registry.schemas(), browser)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_browser_tools() {
        let (schemas, _browser) = create_browser_tools("http://localhost:9222");
        assert_eq!(schemas.len(), 13);
    }

    #[test]
    fn test_create_browser_tools_custom_url() {
        let (schemas, _browser) = create_browser_tools("http://127.0.0.1:9333");
        assert_eq!(schemas.len(), 13);
    }

    #[test]
    fn test_reexports() {
        // Verify that key types are accessible through the crate root
        let client = CdpClient::with_default_url();
        assert!(!client.is_connected());

        let _browser: SharedBrowser = Arc::new(Mutex::new(client));
    }
}
