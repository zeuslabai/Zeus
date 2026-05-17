//! Chrome DevTools Protocol client
//!
//! Connects to a Chrome/Chromium instance running with `--remote-debugging-port`
//! and communicates over the CDP WebSocket protocol.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, warn};
use zeus_core::{Error, Result};

/// Default timeout for CDP commands (30 seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// CDP message sent to Chrome.
#[derive(Debug, Clone, Serialize)]
pub struct CdpCommand {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// CDP response from Chrome.
#[derive(Debug, Clone, Deserialize)]
pub struct CdpResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<CdpError>,
    /// Event method name (for events, not command responses).
    pub method: Option<String>,
    /// Event params (for events, not command responses).
    pub params: Option<Value>,
}

/// CDP error payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
}

/// Tab/target info returned by Chrome's HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

/// Chrome DevTools Protocol client.
///
/// Connects to Chrome via its HTTP debugging API and WebSocket CDP endpoint.
/// Supports both low-level CDP commands and high-level convenience methods.
pub struct CdpClient {
    /// Base URL for Chrome's HTTP debugging API (e.g. http://localhost:9222).
    debug_url: String,
    /// WebSocket sender for the active tab.
    ws_tx: Option<mpsc::Sender<CdpCommand>>,
    /// Pending response channels keyed by request ID.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<CdpResponse>>>>,
    /// Next request ID counter.
    next_id: Arc<Mutex<u64>>,
    /// Currently connected tab ID.
    current_tab: Option<String>,
    /// Handle to the reader task so we can abort on disconnect.
    reader_handle: Option<tokio::task::JoinHandle<()>>,
}

impl CdpClient {
    /// Create a new CDP client targeting the given debug URL.
    ///
    /// The debug URL should point to Chrome's HTTP debugging endpoint,
    /// typically `http://localhost:9222`.
    pub fn new(debug_url: &str) -> Self {
        Self {
            debug_url: debug_url.trim_end_matches('/').to_string(),
            ws_tx: None,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(1)),
            current_tab: None,
            reader_handle: None,
        }
    }

    /// Create a CDP client with the default debug URL.
    ///
    /// Reads `ZEUS_CDP_URL` env var; falls back to `http://localhost:9222`.
    pub fn with_default_url() -> Self {
        let url = std::env::var("ZEUS_CDP_URL")
            .unwrap_or_else(|_| "http://localhost:9222".to_string());
        Self::new(&url)
    }

    /// Get the configured debug URL.
    pub fn debug_url(&self) -> &str {
        &self.debug_url
    }

    /// Check if connected to a tab's WebSocket.
    pub fn is_connected(&self) -> bool {
        self.ws_tx.is_some()
    }

    /// Get the currently connected tab ID, if any.
    pub fn current_tab_id(&self) -> Option<&str> {
        self.current_tab.as_deref()
    }

    // ========================================================================
    // HTTP API methods (no WebSocket connection required)
    // ========================================================================

    /// List all open tabs via Chrome's HTTP API.
    pub async fn list_tabs(&self) -> Result<Vec<TabInfo>> {
        let url = format!("{}/json/list", self.debug_url);
        let resp = reqwest::get(&url).await.map_err(|e| {
            Error::Tool(format!(
                "Failed to list tabs (is Chrome running with --remote-debugging-port?): {}",
                e
            ))
        })?;

        let tabs: Vec<TabInfo> = resp
            .json()
            .await
            .map_err(|e| Error::Tool(format!("Failed to parse tab list: {}", e)))?;
        Ok(tabs)
    }

    /// Open a new tab, optionally navigating to a URL.
    pub async fn new_tab(&self, url: Option<&str>) -> Result<TabInfo> {
        let endpoint = if let Some(url) = url {
            format!("{}/json/new?{}", self.debug_url, urlencoding::encode(url))
        } else {
            format!("{}/json/new", self.debug_url)
        };

        let resp = reqwest::get(&endpoint)
            .await
            .map_err(|e| Error::Tool(format!("Failed to create new tab: {}", e)))?;

        let tab: TabInfo = resp
            .json()
            .await
            .map_err(|e| Error::Tool(format!("Failed to parse new tab response: {}", e)))?;
        Ok(tab)
    }

    /// Close a tab by ID.
    pub async fn close_tab(&self, tab_id: &str) -> Result<()> {
        let url = format!("{}/json/close/{}", self.debug_url, tab_id);
        reqwest::get(&url)
            .await
            .map_err(|e| Error::Tool(format!("Failed to close tab {}: {}", tab_id, e)))?;
        Ok(())
    }

    // ========================================================================
    // WebSocket connection management
    // ========================================================================

    /// Connect to a specific tab's WebSocket for CDP commands.
    ///
    /// If `tab_id` is provided, connects to that tab. Otherwise connects to the
    /// first available page tab.
    pub async fn connect(&mut self, tab_id: Option<&str>) -> Result<()> {
        // Disconnect if already connected
        self.disconnect().await;

        // Find the target tab
        let tabs = self.list_tabs().await?;
        let tab = if let Some(id) = tab_id {
            tabs.iter()
                .find(|t| t.id == id)
                .ok_or_else(|| Error::Tool(format!("Tab not found: {}", id)))?
                .clone()
        } else {
            tabs.into_iter()
                .find(|t| t.target_type == "page")
                .ok_or_else(|| Error::Tool("No page tabs available".to_string()))?
        };

        let ws_url = tab
            .ws_url
            .as_ref()
            .ok_or_else(|| Error::Tool(format!("Tab {} has no WebSocket URL", tab.id)))?;

        debug!("Connecting to CDP WebSocket: {}", ws_url);

        // Connect via WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| Error::Tool(format!("WebSocket connection failed: {}", e)))?;

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Create channel for sending commands
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<CdpCommand>(64);

        let pending = self.pending.clone();

        // Spawn writer task: forwards CdpCommands to the WebSocket
        let writer_handle = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let json = match serde_json::to_string(&cmd) {
                    Ok(j) => j,
                    Err(e) => {
                        error!("Failed to serialize CDP command: {}", e);
                        continue;
                    }
                };
                debug!("CDP send: {}", json);
                if let Err(e) = ws_write.send(WsMessage::Text(json)).await {
                    error!("WebSocket send error: {}", e);
                    break;
                }
            }
        });

        // Spawn reader task: routes responses to pending oneshot channels
        let reader_pending = pending.clone();
        let reader_handle = tokio::spawn(async move {
            while let Some(msg) = ws_read.next().await {
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        let text_str: &str = &text;
                        debug!("CDP recv: {}", text_str);
                        match serde_json::from_str::<CdpResponse>(text_str) {
                            Ok(resp) => {
                                if let Some(id) = resp.id {
                                    // This is a command response
                                    let mut pending = reader_pending.lock().await;
                                    if let Some(tx) = pending.remove(&id) {
                                        let _ = tx.send(resp);
                                    }
                                } else if let Some(ref method) = resp.method {
                                    // This is an event -- log but don't route for now
                                    debug!("CDP event: {}", method);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse CDP response: {}", e);
                            }
                        }
                    }
                    Ok(WsMessage::Close(_)) => {
                        debug!("CDP WebSocket closed");
                        break;
                    }
                    Err(e) => {
                        error!("CDP WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            // Drain any remaining pending requests so callers don't hang forever
            let mut pending = reader_pending.lock().await;
            pending.clear();
            // Also ensure the writer task terminates
            drop(writer_handle);
        });

        self.ws_tx = Some(cmd_tx);
        self.current_tab = Some(tab.id.clone());
        self.reader_handle = Some(reader_handle);

        debug!("Connected to tab: {} ({})", tab.title, tab.id);
        Ok(())
    }

    /// Disconnect from the current tab.
    pub async fn disconnect(&mut self) {
        self.ws_tx = None;
        self.current_tab = None;
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        // Clear any pending requests
        let mut pending = self.pending.lock().await;
        pending.clear();
        // Reset ID counter
        let mut id = self.next_id.lock().await;
        *id = 1;
    }

    // ========================================================================
    // Low-level CDP command
    // ========================================================================

    /// Send a CDP command and wait for the response with a timeout.
    ///
    /// Returns the `result` field from the response, or an error if the command
    /// failed or timed out.
    pub async fn send(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let ws_tx = self.ws_tx.as_ref().ok_or_else(|| {
            Error::Tool("Not connected to a tab. Call connect() first.".to_string())
        })?;

        // Allocate request ID
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let cmd = CdpCommand {
            id,
            method: method.to_string(),
            params,
        };

        // Register pending response channel
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // Send the command
        ws_tx
            .send(cmd)
            .await
            .map_err(|e| Error::Tool(format!("Failed to send CDP command: {}", e)))?;

        // Wait for response with timeout
        let resp = timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS), rx)
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "CDP command '{}' timed out after {}s",
                    method, DEFAULT_TIMEOUT_SECS
                ))
            })?
            .map_err(|_| {
                Error::Tool(format!(
                    "CDP response channel closed for '{}' (connection lost?)",
                    method
                ))
            })?;

        // Check for error
        if let Some(err) = resp.error {
            return Err(Error::Tool(format!(
                "CDP error ({}): {}",
                err.code, err.message
            )));
        }

        Ok(resp.result.unwrap_or(Value::Null))
    }

    // ========================================================================
    // High-level convenience methods
    // ========================================================================

    /// Navigate to a URL and wait for load.
    pub async fn navigate(&self, url: &str) -> Result<Value> {
        self.send("Page.navigate", Some(serde_json::json!({ "url": url })))
            .await
    }

    /// Click an element by CSS selector.
    ///
    /// Uses `Runtime.evaluate` to find the element and trigger a click.
    pub async fn click(&self, selector: &str) -> Result<Value> {
        let script = format!(
            r#"(() => {{ const el = document.querySelector('{}'); if (!el) throw new Error('Element not found: {}'); el.click(); return 'clicked'; }})()"#,
            selector.replace('\'', "\\'").replace('\\', "\\\\"),
            selector.replace('\'', "\\'").replace('\\', "\\\\"),
        );
        self.evaluate(&script).await
    }

    /// Type text into the focused element using CDP Input events.
    ///
    /// Each character is dispatched as a keyDown/keyUp pair.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            let ch_str = ch.to_string();
            self.send(
                "Input.dispatchKeyEvent",
                Some(serde_json::json!({
                    "type": "keyDown",
                    "text": ch_str,
                })),
            )
            .await?;
            self.send(
                "Input.dispatchKeyEvent",
                Some(serde_json::json!({
                    "type": "keyUp",
                    "text": ch_str,
                })),
            )
            .await?;
        }
        Ok(())
    }

    /// Take a screenshot of the current page.
    ///
    /// Returns the screenshot as a base64-encoded PNG string.
    pub async fn screenshot(&self) -> Result<String> {
        let result = self
            .send(
                "Page.captureScreenshot",
                Some(serde_json::json!({ "format": "png" })),
            )
            .await?;
        result
            .get("data")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Tool("No screenshot data in response".to_string()))
    }

    /// Get the text content of the page body.
    pub async fn get_text(&self) -> Result<String> {
        let result = self.evaluate("document.body?.innerText || ''").await?;
        Ok(result.as_str().unwrap_or("").to_string())
    }

    /// Get the text content of a specific element by CSS selector.
    pub async fn get_element_text(&self, selector: &str) -> Result<String> {
        let script = format!(
            "document.querySelector('{}')?.innerText || ''",
            selector.replace('\'', "\\'").replace('\\', "\\\\"),
        );
        let result = self.evaluate(&script).await?;
        Ok(result.as_str().unwrap_or("").to_string())
    }

    /// Execute JavaScript in the page and return the result.
    ///
    /// The expression is evaluated with `returnByValue: true` so the result
    /// is serialized back as a JSON value.
    pub async fn evaluate(&self, expression: &str) -> Result<Value> {
        let result = self
            .send(
                "Runtime.evaluate",
                Some(serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                })),
            )
            .await?;

        // Check for exceptions
        if let Some(exception) = result.get("exceptionDetails") {
            let msg = exception
                .get("text")
                .and_then(|t| t.as_str())
                .or_else(|| {
                    exception
                        .get("exception")
                        .and_then(|e| e.get("description"))
                        .and_then(|d| d.as_str())
                })
                .unwrap_or("Unknown JavaScript error");
            return Err(Error::Tool(format!("JavaScript error: {}", msg)));
        }

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Get a simplified snapshot of the page DOM (truncated outer HTML).
    ///
    /// Useful for giving an LLM context about the page structure without
    /// sending the full DOM.
    pub async fn page_snapshot(&self) -> Result<String> {
        let result = self
            .evaluate("document.documentElement?.outerHTML?.substring(0, 50000) || ''")
            .await?;
        Ok(result.as_str().unwrap_or("").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cdp_client_new() {
        let client = CdpClient::new("http://localhost:9222");
        assert_eq!(client.debug_url(), "http://localhost:9222");
        assert!(!client.is_connected());
        assert!(client.current_tab_id().is_none());
    }

    #[test]
    fn test_cdp_client_new_trailing_slash() {
        let client = CdpClient::new("http://localhost:9222/");
        assert_eq!(client.debug_url(), "http://localhost:9222");
    }

    #[test]
    fn test_cdp_client_default_url() {
        let client = CdpClient::with_default_url();
        assert_eq!(client.debug_url(), "http://localhost:9222");
    }

    #[test]
    fn test_cdp_command_serialization() {
        let cmd = CdpCommand {
            id: 1,
            method: "Page.navigate".to_string(),
            params: Some(serde_json::json!({"url": "https://example.com"})),
        };
        let json = serde_json::to_string(&cmd).expect("should serialize to JSON");
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"Page.navigate\""));
        assert!(json.contains("\"url\":\"https://example.com\""));
    }

    #[test]
    fn test_cdp_command_serialization_no_params() {
        let cmd = CdpCommand {
            id: 42,
            method: "Page.captureScreenshot".to_string(),
            params: None,
        };
        let json = serde_json::to_string(&cmd).expect("should serialize to JSON");
        assert!(json.contains("\"id\":42"));
        assert!(json.contains("\"method\":\"Page.captureScreenshot\""));
        assert!(!json.contains("\"params\""));
    }

    #[test]
    fn test_cdp_response_deserialization_success() {
        let json = r#"{"id":1,"result":{"frameId":"ABC123","loaderId":"DEF456"}}"#;
        let resp: CdpResponse = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert!(resp.method.is_none());
    }

    #[test]
    fn test_cdp_response_deserialization_error() {
        let json = r#"{"id":2,"error":{"code":-32000,"message":"Page not found"}}"#;
        let resp: CdpResponse = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(resp.id, Some(2));
        assert!(resp.result.is_none());
        let err = resp.error.expect("operation should succeed");
        assert_eq!(err.code, -32000);
        assert_eq!(err.message, "Page not found");
    }

    #[test]
    fn test_cdp_response_deserialization_event() {
        let json = r#"{"method":"Page.loadEventFired","params":{"timestamp":12345.6}}"#;
        let resp: CdpResponse = serde_json::from_str(json).expect("should parse successfully");
        assert!(resp.id.is_none());
        assert_eq!(resp.method, Some("Page.loadEventFired".to_string()));
        assert!(resp.params.is_some());
    }

    #[test]
    fn test_tab_info_deserialization() {
        let json = r#"{
            "description": "",
            "devtoolsFrontendUrl": "/devtools/inspector.html",
            "id": "ABC123",
            "title": "Example",
            "type": "page",
            "url": "https://example.com",
            "webSocketDebuggerUrl": "ws://localhost:9222/devtools/page/ABC123"
        }"#;
        let tab: TabInfo = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(tab.id, "ABC123");
        assert_eq!(tab.target_type, "page");
        assert_eq!(tab.title, "Example");
        assert_eq!(tab.url, "https://example.com");
        assert_eq!(
            tab.ws_url.as_deref(),
            Some("ws://localhost:9222/devtools/page/ABC123")
        );
    }

    #[test]
    fn test_tab_info_deserialization_no_ws_url() {
        let json = r#"{
            "id": "XYZ",
            "title": "Service Worker",
            "type": "service_worker",
            "url": "chrome-extension://abc/sw.js"
        }"#;
        let tab: TabInfo = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(tab.id, "XYZ");
        assert_eq!(tab.target_type, "service_worker");
        assert!(tab.ws_url.is_none());
    }

    #[test]
    fn test_cdp_error_serialization() {
        let err = CdpError {
            code: -32601,
            message: "Method not found".to_string(),
        };
        let json = serde_json::to_string(&err).expect("should serialize to JSON");
        assert!(json.contains("-32601"));
        assert!(json.contains("Method not found"));
    }

    #[test]
    fn test_cdp_command_with_complex_params() {
        let cmd = CdpCommand {
            id: 5,
            method: "Runtime.evaluate".to_string(),
            params: Some(serde_json::json!({
                "expression": "1 + 1",
                "returnByValue": true,
                "awaitPromise": false,
            })),
        };
        let json = serde_json::to_string(&cmd).expect("should serialize to JSON");
        let parsed: Value = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed["params"]["expression"], "1 + 1");
        assert_eq!(parsed["params"]["returnByValue"], true);
    }

    #[test]
    fn test_multiple_tab_info_deserialization() {
        let json = r#"[
            {"id":"tab1","title":"Tab 1","type":"page","url":"https://a.com","webSocketDebuggerUrl":"ws://localhost:9222/devtools/page/tab1"},
            {"id":"tab2","title":"Tab 2","type":"page","url":"https://b.com","webSocketDebuggerUrl":"ws://localhost:9222/devtools/page/tab2"}
        ]"#;
        let tabs: Vec<TabInfo> = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].id, "tab1");
        assert_eq!(tabs[1].id, "tab2");
    }
}
