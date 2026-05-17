//! Browser AI Agent — high-level extraction and page snapshot capabilities.
//!
//! Wraps the CDP client with AI-ready helpers:
//! - `navigate_and_extract(url, prompt)`: navigate to a URL, capture text, return structured data
//! - `snapshot()`: capture a full PageSnapshot with URL, title, text, links, timestamp
//!
//! These are designed for agent tool use — the agent can ask "go to this URL and
//! find the pricing information" and get back structured results.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};
use zeus_core::{Error, Result};

use crate::cdp::CdpClient;

/// Shared browser client handle (same as tools.rs)
type SharedBrowser = Arc<Mutex<CdpClient>>;

// ============================================================================
// PageSnapshot
// ============================================================================

/// A complete snapshot of a browser page's state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    /// Current page URL
    pub url: String,
    /// Page title
    pub title: String,
    /// Visible text content (innerText of body)
    pub text: String,
    /// Links found on the page (href, text pairs)
    pub links: Vec<PageLink>,
    /// Timestamp when snapshot was taken
    pub timestamp: DateTime<Utc>,
    /// Truncated HTML structure (for context, max 10KB)
    pub html_preview: String,
}

/// A link extracted from a page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    /// Link URL (href attribute)
    pub href: String,
    /// Link display text
    pub text: String,
}

// ============================================================================
// ExtractionResult
// ============================================================================

/// Result of a navigate-and-extract operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// The URL that was navigated to
    pub url: String,
    /// Page title
    pub title: String,
    /// Extracted text content matching the prompt intent
    pub content: String,
    /// Full page snapshot for additional context
    pub snapshot: PageSnapshot,
}

// ============================================================================
// BrowserAgent
// ============================================================================

/// AI-oriented browser agent that wraps CDP for high-level page interaction.
///
/// Provides navigate-and-extract patterns useful for agent tool calls.
pub struct BrowserAgent {
    browser: SharedBrowser,
    /// Maximum text length to capture (default 50KB)
    max_text_len: usize,
    /// Maximum HTML preview length (default 10KB)
    max_html_len: usize,
}

impl BrowserAgent {
    /// Create a new BrowserAgent wrapping an existing shared browser handle.
    pub fn new(browser: SharedBrowser) -> Self {
        Self {
            browser,
            max_text_len: 50_000,
            max_html_len: 10_000,
        }
    }

    /// Create with custom text limits.
    pub fn with_limits(browser: SharedBrowser, max_text_len: usize, max_html_len: usize) -> Self {
        Self {
            browser,
            max_text_len,
            max_html_len,
        }
    }

    /// Navigate to a URL, wait for load, and extract page content.
    ///
    /// The `prompt` parameter describes what information to extract.
    /// Currently returns the full page text — a future version will use
    /// LLM-based extraction guided by the prompt.
    ///
    /// Returns an `ExtractionResult` with the page snapshot and content.
    pub async fn navigate_and_extract(&self, url: &str, prompt: &str) -> Result<ExtractionResult> {
        info!(url, prompt, "BrowserAgent: navigate and extract");

        let mut browser = self.browser.lock().await;

        // Ensure connected
        if !browser.is_connected() {
            browser.connect(None).await?;
        }

        // Navigate
        browser.navigate(url).await?;

        // Brief wait for page to settle (dynamic content)
        drop(browser);
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Take snapshot
        let snapshot = self.snapshot_inner().await?;

        // Extract content based on prompt
        // For now, return the full text — keyword filtering as a basic heuristic
        let content = self.extract_relevant(&snapshot.text, prompt);

        debug!(
            url,
            title = %snapshot.title,
            text_len = content.len(),
            links = snapshot.links.len(),
            "BrowserAgent: extraction complete"
        );

        Ok(ExtractionResult {
            url: snapshot.url.clone(),
            title: snapshot.title.clone(),
            content,
            snapshot,
        })
    }

    /// Capture a snapshot of the current page without navigating.
    pub async fn snapshot(&self) -> Result<PageSnapshot> {
        self.snapshot_inner().await
    }

    // -- internals --

    async fn snapshot_inner(&self) -> Result<PageSnapshot> {
        let browser = self.browser.lock().await;

        if !browser.is_connected() {
            return Err(Error::Tool(
                "Browser not connected. Call navigate_and_extract() first or connect manually."
                    .into(),
            ));
        }

        // Get URL
        let url_val = browser
            .evaluate("window.location.href")
            .await
            .unwrap_or(serde_json::Value::String("unknown".into()));
        let url = url_val.as_str().unwrap_or("unknown").to_string();

        // Get title
        let title_val = browser
            .evaluate("document.title || ''")
            .await
            .unwrap_or(serde_json::Value::String(String::new()));
        let title = title_val.as_str().unwrap_or("").to_string();

        // Get visible text
        let text_val = browser
            .evaluate("document.body?.innerText || ''")
            .await
            .unwrap_or(serde_json::Value::String(String::new()));
        let mut text = text_val.as_str().unwrap_or("").to_string();
        if text.len() > self.max_text_len {
            text.truncate(self.max_text_len);
            text.push_str("\n... [truncated]");
        }

        // Get links
        let links_js = r#"
            JSON.stringify(
                Array.from(document.querySelectorAll('a[href]'))
                    .slice(0, 100)
                    .map(a => ({
                        href: a.href,
                        text: (a.innerText || a.title || '').trim().substring(0, 200)
                    }))
                    .filter(l => l.href && !l.href.startsWith('javascript:'))
            )
        "#;
        let links_val = browser
            .evaluate(links_js)
            .await
            .unwrap_or(serde_json::Value::String("[]".into()));
        let links_str = links_val.as_str().unwrap_or("[]");
        let links: Vec<PageLink> = serde_json::from_str(links_str).unwrap_or_default();

        // Get HTML preview
        let html_expr = format!(
            "document.documentElement?.outerHTML?.substring(0, {}) || ''",
            self.max_html_len
        );
        let html_val = browser
            .evaluate(&html_expr)
            .await
            .unwrap_or(serde_json::Value::String(String::new()));
        let html_preview = html_val.as_str().unwrap_or("").to_string();

        Ok(PageSnapshot {
            url,
            title,
            text,
            links,
            timestamp: Utc::now(),
            html_preview,
        })
    }

    /// Basic keyword-based extraction from page text.
    ///
    /// Finds paragraphs containing words from the prompt.
    /// Returns the full text if no keywords match (better to over-include).
    fn extract_relevant(&self, text: &str, prompt: &str) -> String {
        let keywords: Vec<&str> = prompt
            .split_whitespace()
            .filter(|w| w.len() > 3) // skip short words
            .collect();

        if keywords.is_empty() {
            return self.truncate_text(text);
        }

        let paragraphs: Vec<&str> = text.split('\n').collect();
        let mut relevant: Vec<&str> = Vec::new();

        for para in &paragraphs {
            let lower = para.to_lowercase();
            if keywords.iter().any(|kw| lower.contains(&kw.to_lowercase())) {
                relevant.push(para);
            }
        }

        if relevant.is_empty() {
            // No keyword matches — return full text
            self.truncate_text(text)
        } else {
            self.truncate_text(&relevant.join("\n"))
        }
    }

    fn truncate_text(&self, text: &str) -> String {
        if text.len() <= self.max_text_len {
            text.to_string()
        } else {
            let mut s = text[..zeus_core::floor_char_boundary(text, self.max_text_len)].to_string();
            s.push_str("\n... [truncated]");
            s
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_snapshot_serialization() {
        let snapshot = PageSnapshot {
            url: "https://example.com".into(),
            title: "Example".into(),
            text: "Hello world".into(),
            links: vec![PageLink {
                href: "https://example.com/about".into(),
                text: "About".into(),
            }],
            timestamp: Utc::now(),
            html_preview: "<html>...</html>".into(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: PageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.url, "https://example.com");
        assert_eq!(parsed.title, "Example");
        assert_eq!(parsed.text, "Hello world");
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].href, "https://example.com/about");
    }

    #[test]
    fn test_page_link_serialization() {
        let link = PageLink {
            href: "https://test.com".into(),
            text: "Test Link".into(),
        };
        let json = serde_json::to_string(&link).unwrap();
        let parsed: PageLink = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.href, "https://test.com");
        assert_eq!(parsed.text, "Test Link");
    }

    #[test]
    fn test_extraction_result_serialization() {
        let result = ExtractionResult {
            url: "https://example.com".into(),
            title: "Example".into(),
            content: "Extracted text".into(),
            snapshot: PageSnapshot {
                url: "https://example.com".into(),
                title: "Example".into(),
                text: "Full page text".into(),
                links: vec![],
                timestamp: Utc::now(),
                html_preview: String::new(),
            },
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ExtractionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.content, "Extracted text");
        assert_eq!(parsed.snapshot.text, "Full page text");
    }

    #[test]
    fn test_browser_agent_creation() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::new(browser);
        assert_eq!(agent.max_text_len, 50_000);
        assert_eq!(agent.max_html_len, 10_000);
    }

    #[test]
    fn test_browser_agent_custom_limits() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::with_limits(browser, 1000, 500);
        assert_eq!(agent.max_text_len, 1000);
        assert_eq!(agent.max_html_len, 500);
    }

    #[test]
    fn test_extract_relevant_with_keywords() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::new(browser);

        let text = "Welcome to our site\nOur pricing starts at $10/month\nContact us for more info\nPricing details below";
        let result = agent.extract_relevant(text, "What is the pricing?");
        assert!(result.contains("pricing"));
        assert!(result.contains("$10/month"));
    }

    #[test]
    fn test_extract_relevant_no_match_returns_full() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::new(browser);

        let text = "Hello world\nThis is a test page";
        let result = agent.extract_relevant(text, "find the xyzzy");
        // "xyzzy" won't match, so full text returned
        assert!(result.contains("Hello world"));
        assert!(result.contains("test page"));
    }

    #[test]
    fn test_truncate_text() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::with_limits(browser, 20, 10);

        let short = "short text";
        assert_eq!(agent.truncate_text(short), "short text");

        let long = "this is a very long text that exceeds the limit";
        let truncated = agent.truncate_text(long);
        assert!(truncated.len() <= 40); // 20 chars + "... [truncated]"
        assert!(truncated.ends_with("[truncated]"));
    }

    #[tokio::test]
    async fn test_snapshot_not_connected() {
        let browser: SharedBrowser = Arc::new(Mutex::new(CdpClient::with_default_url()));
        let agent = BrowserAgent::new(browser);
        let result = agent.snapshot().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }
}
