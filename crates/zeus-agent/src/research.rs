//! Deep Research Engine — multi-step web search + synthesis
//!
//! Pipeline:
//! 1. Query decomposition — LLM breaks research query into sub-queries
//! 2. Parallel web search — DuckDuckGo HTML search for each sub-query
//! 3. Source fetching — fetch top results in parallel
//! 4. Synthesis — LLM synthesizes findings into structured report
//!
//! Configurable via env vars:
//! - `ZEUS_RESEARCH_MAX_QUERIES`  — max sub-queries (default: 5)
//! - `ZEUS_RESEARCH_MAX_SOURCES`  — max sources per sub-query (default: 3)
//! - `ZEUS_RESEARCH_TIMEOUT`      — overall timeout in seconds (default: 120)

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{debug, info, warn};
use zeus_core::{Error, Message, Result};
use zeus_llm::LlmClient;

// ============================================================================
// Configuration
// ============================================================================

/// Research engine configuration, read from env vars with defaults.
#[derive(Debug, Clone)]
pub struct ResearchConfig {
    /// Maximum sub-queries to decompose into
    pub max_queries: usize,
    /// Maximum sources to fetch per sub-query
    pub max_sources: usize,
    /// Overall timeout in seconds
    pub timeout_secs: u64,
}

impl ResearchConfig {
    /// Load config from environment variables with defaults.
    pub fn from_env() -> Self {
        Self {
            max_queries: parse_env("ZEUS_RESEARCH_MAX_QUERIES", 5),
            max_sources: parse_env("ZEUS_RESEARCH_MAX_SOURCES", 3),
            timeout_secs: parse_env("ZEUS_RESEARCH_TIMEOUT", 120),
        }
    }
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            max_queries: 5,
            max_sources: 3,
            timeout_secs: 120,
        }
    }
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ============================================================================
// Types
// ============================================================================

/// A source fetched during research.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSource {
    /// Source URL
    pub url: String,
    /// Page title
    pub title: String,
    /// Extracted content (truncated)
    pub content: String,
    /// Which sub-query found this source
    pub query: String,
    /// Fetch timestamp
    pub fetched_at: DateTime<Utc>,
}

/// A sub-query generated during decomposition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuery {
    /// The sub-query text
    pub query: String,
    /// Why this sub-query is relevant
    pub rationale: String,
}

/// Structured research report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchReport {
    /// Original research query
    pub query: String,
    /// Sub-queries used
    pub sub_queries: Vec<SubQuery>,
    /// Sources consulted
    pub sources: Vec<ResearchSource>,
    /// Synthesized findings
    pub synthesis: String,
    /// Key findings (bullet points)
    pub key_findings: Vec<String>,
    /// Number of sources fetched
    pub sources_count: usize,
    /// Total research time in milliseconds
    pub duration_ms: u64,
}

// ============================================================================
// Research Engine
// ============================================================================

/// Deep research engine that decomposes queries, searches in parallel,
/// fetches sources, and synthesizes findings via LLM.
pub struct ResearchEngine {
    config: ResearchConfig,
    client: reqwest::Client,
}

impl ResearchEngine {
    /// Create a new research engine with the given config.
    pub fn new(config: ResearchConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; Zeus/1.0 Deep Research)")
            .build()
            .expect("reqwest client");

        Self { config, client }
    }

    /// Create with config loaded from environment.
    pub fn from_env() -> Self {
        Self::new(ResearchConfig::from_env())
    }

    /// Execute a deep research query.
    ///
    /// Pipeline: decompose → search → fetch → synthesize
    pub async fn research(&self, query: &str, llm: &LlmClient) -> Result<ResearchReport> {
        let start = std::time::Instant::now();
        info!("deep_research: starting for query: {}", query);

        // Wrap entire pipeline in a timeout
        let timeout = Duration::from_secs(self.config.timeout_secs);
        match tokio::time::timeout(timeout, self.research_inner(query, llm)).await {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                match result {
                    Ok(mut report) => {
                        report.duration_ms = duration_ms;
                        info!(
                            "deep_research: completed in {}ms, {} sources",
                            duration_ms, report.sources_count
                        );
                        Ok(report)
                    }
                    Err(e) => Err(e),
                }
            }
            Err(_) => Err(Error::Timeout(format!(
                "Deep research timed out after {}s",
                self.config.timeout_secs
            ))),
        }
    }

    async fn research_inner(&self, query: &str, llm: &LlmClient) -> Result<ResearchReport> {
        // Step 1: Decompose query into sub-queries
        let sub_queries = self.decompose_query(query, llm).await?;
        debug!(
            "deep_research: decomposed into {} sub-queries",
            sub_queries.len()
        );

        // Step 2: Parallel web search for each sub-query
        let search_results = self.parallel_search(&sub_queries).await;
        debug!(
            "deep_research: found {} total search results",
            search_results.len()
        );

        // Step 3: Fetch top sources in parallel
        let sources = self.parallel_fetch(&search_results).await;
        debug!("deep_research: fetched {} sources", sources.len());

        // Step 4: Synthesize findings
        let (synthesis, key_findings) = self.synthesize(query, &sources, llm).await?;

        let sources_count = sources.len();
        Ok(ResearchReport {
            query: query.to_string(),
            sub_queries,
            sources,
            synthesis,
            key_findings,
            sources_count,
            duration_ms: 0, // Set by caller
        })
    }

    // ── Step 1: Query Decomposition ─────────────────────────────────────

    async fn decompose_query(&self, query: &str, llm: &LlmClient) -> Result<Vec<SubQuery>> {
        let system = format!(
            "You are a research query decomposer. Given a research question, break it into \
             {} or fewer specific sub-queries that together will provide comprehensive coverage.\n\n\
             Respond with ONLY valid JSON:\n\
             {{\"sub_queries\": [\n  {{\"query\": \"specific search query\", \"rationale\": \"why this matters\"}}\n]}}\n\n\
             Rules:\n\
             - Each sub-query should be a specific, searchable phrase\n\
             - Cover different aspects of the topic\n\
             - If the query is already specific enough, return just 1-2 sub-queries\n\
             - Focus on factual, verifiable information",
            self.config.max_queries
        );

        let messages = vec![Message::user(format!(
            "Decompose this research question: {}",
            query
        ))];

        let response = llm.complete(&messages, &[], Some(&system)).await?;
        self.parse_sub_queries(&response.content, query)
    }

    fn parse_sub_queries(&self, response: &str, original_query: &str) -> Result<Vec<SubQuery>> {
        let json_str = extract_json(response);

        match serde_json::from_str::<SubQueryResponse>(&json_str) {
            Ok(parsed) => {
                let mut queries: Vec<SubQuery> = parsed
                    .sub_queries
                    .into_iter()
                    .take(self.config.max_queries)
                    .collect();

                // Ensure at least one sub-query
                if queries.is_empty() {
                    queries.push(SubQuery {
                        query: original_query.to_string(),
                        rationale: "Original query used directly".to_string(),
                    });
                }

                Ok(queries)
            }
            Err(e) => {
                warn!("Failed to parse sub-queries ({}), using original query", e);
                Ok(vec![SubQuery {
                    query: original_query.to_string(),
                    rationale: "Fallback: original query used directly".to_string(),
                }])
            }
        }
    }

    // ── Step 2: Parallel Web Search ─────────────────────────────────────

    async fn parallel_search(&self, sub_queries: &[SubQuery]) -> Vec<SearchResult> {
        let mut handles = Vec::new();

        for sq in sub_queries {
            let client = self.client.clone();
            let query = sq.query.clone();
            let max = self.config.max_sources;

            handles.push(tokio::spawn(async move {
                match search_ddg(&client, &query, max).await {
                    Ok(results) => results,
                    Err(e) => {
                        warn!("Search failed for '{}': {}", query, e);
                        Vec::new()
                    }
                }
            }));
        }

        let mut all_results = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();

        for handle in handles {
            if let Ok(results) = handle.await {
                for result in results {
                    // Deduplicate by URL
                    if seen_urls.insert(result.url.clone()) {
                        all_results.push(result);
                    }
                }
            }
        }

        all_results
    }

    // ── Step 3: Parallel Source Fetching ─────────────────────────────────

    async fn parallel_fetch(&self, search_results: &[SearchResult]) -> Vec<ResearchSource> {
        let mut handles = Vec::new();

        for sr in search_results {
            let client = self.client.clone();
            let url = sr.url.clone();
            let title = sr.title.clone();
            let query = sr.query.clone();

            handles.push(tokio::spawn(async move {
                match fetch_source(&client, &url).await {
                    Ok(content) => Some(ResearchSource {
                        url,
                        title,
                        content,
                        query,
                        fetched_at: Utc::now(),
                    }),
                    Err(e) => {
                        warn!("Fetch failed for '{}': {}", url, e);
                        None
                    }
                }
            }));
        }

        let mut sources = Vec::new();
        for handle in handles {
            if let Ok(Some(source)) = handle.await {
                sources.push(source);
            }
        }

        sources
    }

    // ── Step 4: Synthesis ───────────────────────────────────────────────

    async fn synthesize(
        &self,
        query: &str,
        sources: &[ResearchSource],
        llm: &LlmClient,
    ) -> Result<(String, Vec<String>)> {
        if sources.is_empty() {
            return Ok((
                "No sources could be fetched for this research query.".to_string(),
                vec!["No results found".to_string()],
            ));
        }

        // Build source context (truncate each source to keep within limits)
        let max_per_source = 2000;
        let source_text: String = sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let truncated: String = s.content.chars().take(max_per_source).collect();
                format!(
                    "### Source {} — {} ({})\n{}\n",
                    i + 1,
                    s.title,
                    s.url,
                    truncated
                )
            })
            .collect();

        let system = "You are a research synthesizer. Given multiple web sources about a topic, \
             produce a comprehensive, well-organized synthesis.\n\n\
             Respond with ONLY valid JSON:\n\
             {\n  \"synthesis\": \"A comprehensive multi-paragraph synthesis of findings...\",\n  \
             \"key_findings\": [\"Finding 1\", \"Finding 2\", ...]}\n\n\
             Rules:\n\
             - Cite sources by number [1], [2], etc.\n\
             - Be factual and objective\n\
             - Highlight agreements and contradictions between sources\n\
             - Include 3-7 key findings as bullet points\n\
             - If sources are insufficient, say so honestly"
            .to_string();

        let messages = vec![Message::user(format!(
            "Research question: {}\n\nSources:\n\n{}",
            query, source_text
        ))];

        let response = llm.complete(&messages, &[], Some(&system)).await?;
        self.parse_synthesis(&response.content)
    }

    fn parse_synthesis(&self, response: &str) -> Result<(String, Vec<String>)> {
        let json_str = extract_json(response);

        match serde_json::from_str::<SynthesisResponse>(&json_str) {
            Ok(parsed) => Ok((parsed.synthesis, parsed.key_findings)),
            Err(e) => {
                warn!("Failed to parse synthesis JSON ({}), using raw response", e);
                Ok((response.to_string(), vec![]))
            }
        }
    }
}

// ============================================================================
// Web Search (DuckDuckGo HTML)
// ============================================================================

#[derive(Debug, Clone)]
struct SearchResult {
    url: String,
    title: String,
    #[allow(dead_code)]
    snippet: String,
    query: String,
}

async fn search_ddg(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!("deep_research: searching DDG for '{}'", query);

    let response = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| Error::Network(format!("DDG search failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "DDG returned HTTP {}",
            response.status()
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|e| Error::Network(format!("Failed to read DDG response: {}", e)))?;

    // Parse results
    static RESULT_RE: OnceLock<Regex> = OnceLock::new();
    static SNIPPET_RE: OnceLock<Regex> = OnceLock::new();

    let result_re = RESULT_RE.get_or_init(|| {
        Regex::new(
            r#"<a rel="nofollow" class="result__a" href="([^"]*)"[^>]*>([^<]*(?:<b>[^<]*</b>[^<]*)*)</a>"#
        ).expect("valid regex")
    });
    let snippet_re = SNIPPET_RE.get_or_init(|| {
        Regex::new(r#"<a class="result__snippet"[^>]*>([^<]*(?:<b>[^<]*</b>[^<]*)*)</a>"#)
            .expect("valid regex")
    });

    let urls: Vec<(String, String)> = result_re
        .captures_iter(&html)
        .map(|cap| {
            let url = cap[1].to_string();
            let title = cap[2].replace("<b>", "").replace("</b>", "");
            (url, title)
        })
        .collect();

    let snippets: Vec<String> = snippet_re
        .captures_iter(&html)
        .map(|cap| {
            cap[1]
                .replace("<b>", "")
                .replace("</b>", "")
                .trim()
                .to_string()
        })
        .collect();

    let mut results = Vec::new();
    for (i, (url, title)) in urls.into_iter().enumerate().take(max_results) {
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        results.push(SearchResult {
            url,
            title,
            snippet,
            query: query.to_string(),
        });
    }

    Ok(results)
}

// ============================================================================
// Source Fetching
// ============================================================================

/// Fetch a URL and extract readable text content.
async fn fetch_source(client: &reqwest::Client, url: &str) -> Result<String> {
    debug!("deep_research: fetching {}", url);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Network(format!("Fetch failed for {}: {}", url, e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "HTTP {} for {}",
            response.status(),
            url
        )));
    }

    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_lowercase().contains("text/html"))
        .unwrap_or(false);

    let text = response
        .text()
        .await
        .map_err(|e| Error::Network(format!("Failed to read response from {}: {}", url, e)))?;

    if is_html {
        Ok(strip_html_to_text(&text))
    } else {
        // Plain text or other — truncate
        let max = 8000;
        if text.len() > max {
            Ok(text.chars().take(max).collect())
        } else {
            Ok(text)
        }
    }
}

/// Minimal HTML-to-text extraction: remove tags, decode entities, collapse whitespace.
fn strip_html_to_text(html: &str) -> String {
    static SCRIPT_RE: OnceLock<Regex> = OnceLock::new();
    static STYLE_RE: OnceLock<Regex> = OnceLock::new();
    static TAG_RE: OnceLock<Regex> = OnceLock::new();
    static WS_RE: OnceLock<Regex> = OnceLock::new();

    let script_re = SCRIPT_RE
        .get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script\s*>").expect("valid regex"));
    let style_re = STYLE_RE
        .get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style\s*>").expect("valid regex"));
    let tag_re = TAG_RE.get_or_init(|| Regex::new(r"<[^>]+>").expect("valid regex"));
    let ws_re = WS_RE.get_or_init(|| Regex::new(r"\s{3,}").expect("valid regex"));

    let text = script_re.replace_all(html, " ");
    let text = style_re.replace_all(&text, " ");
    let text = tag_re.replace_all(&text, " ");
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let text = ws_re.replace_all(&text, "\n");

    // Truncate to reasonable size
    let max = 8000;
    if text.len() > max {
        text.chars().take(max).collect()
    } else {
        text.to_string()
    }
}

// ============================================================================
// JSON Helpers
// ============================================================================

#[derive(Deserialize)]
struct SubQueryResponse {
    sub_queries: Vec<SubQuery>,
}

#[derive(Deserialize)]
struct SynthesisResponse {
    synthesis: String,
    #[serde(default)]
    key_findings: Vec<String>,
}

/// Extract JSON from a response that may contain markdown code blocks.
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
    {
        return trimmed[start..=end].to_string();
    }
    trimmed.to_string()
}

/// Format a research report as readable text for the LLM/user.
pub fn format_report(report: &ResearchReport) -> String {
    let mut out = String::new();

    out.push_str(&format!("# Deep Research: {}\n\n", report.query));
    out.push_str(&format!(
        "_Researched {} sources in {:.1}s_\n\n",
        report.sources_count,
        report.duration_ms as f64 / 1000.0
    ));

    // Key findings
    if !report.key_findings.is_empty() {
        out.push_str("## Key Findings\n\n");
        for finding in &report.key_findings {
            out.push_str(&format!("- {}\n", finding));
        }
        out.push('\n');
    }

    // Synthesis
    out.push_str("## Synthesis\n\n");
    out.push_str(&report.synthesis);
    out.push_str("\n\n");

    // Sources
    out.push_str("## Sources\n\n");
    for (i, source) in report.sources.iter().enumerate() {
        out.push_str(&format!("[{}] {} — {}\n", i + 1, source.title, source.url));
    }

    // Sub-queries used
    out.push_str("\n## Sub-queries\n\n");
    for sq in &report.sub_queries {
        out.push_str(&format!("- **{}** — {}\n", sq.query, sq.rationale));
    }

    out
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = ResearchConfig::default();
        assert_eq!(config.max_queries, 5);
        assert_eq!(config.max_sources, 3);
        assert_eq!(config.timeout_secs, 120);
    }

    #[test]
    fn test_config_from_env() {
        // With no env vars set, should use defaults
        let config = ResearchConfig::from_env();
        // Can't assert exact values since env may have them set,
        // but should not panic
        assert!(config.max_queries > 0);
        assert!(config.max_sources > 0);
        assert!(config.timeout_secs > 0);
    }

    #[test]
    fn test_extract_json_from_code_block() {
        let input =
            "Here:\n```json\n{\"sub_queries\": [{\"query\": \"test\", \"rationale\": \"r\"}]}\n```";
        let json = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["sub_queries"].is_array());
    }

    #[test]
    fn test_extract_json_raw() {
        let input = "{\"synthesis\": \"hello\", \"key_findings\": [\"a\", \"b\"]}";
        let json = extract_json(input);
        let parsed: SynthesisResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.synthesis, "hello");
        assert_eq!(parsed.key_findings.len(), 2);
    }

    #[test]
    fn test_extract_json_no_json() {
        let json = extract_json("no json here");
        assert_eq!(json, "no json here");
    }

    #[test]
    fn test_parse_sub_queries_valid() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        let response = r#"{"sub_queries": [{"query": "rust async", "rationale": "core concept"}]}"#;
        let result = engine.parse_sub_queries(response, "rust async programming");
        assert!(result.is_ok());
        let queries = result.unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query, "rust async");
    }

    #[test]
    fn test_parse_sub_queries_invalid_json_fallback() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        let result = engine.parse_sub_queries("not json", "original query");
        assert!(result.is_ok());
        let queries = result.unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query, "original query");
    }

    #[test]
    fn test_parse_sub_queries_empty_array_fallback() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        let result = engine.parse_sub_queries(r#"{"sub_queries": []}"#, "fallback query");
        assert!(result.is_ok());
        let queries = result.unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query, "fallback query");
    }

    #[test]
    fn test_parse_sub_queries_respects_max() {
        let config = ResearchConfig {
            max_queries: 2,
            ..Default::default()
        };
        let engine = ResearchEngine::new(config);
        let response = r#"{"sub_queries": [
            {"query": "q1", "rationale": "r1"},
            {"query": "q2", "rationale": "r2"},
            {"query": "q3", "rationale": "r3"}
        ]}"#;
        let queries = engine.parse_sub_queries(response, "test").unwrap();
        assert_eq!(queries.len(), 2);
    }

    #[test]
    fn test_parse_synthesis_valid() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        let response = r#"{"synthesis": "The findings show...", "key_findings": ["A", "B", "C"]}"#;
        let (synthesis, findings) = engine.parse_synthesis(response).unwrap();
        assert_eq!(synthesis, "The findings show...");
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn test_parse_synthesis_invalid_json_fallback() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        let (synthesis, findings) = engine.parse_synthesis("Raw text response").unwrap();
        assert_eq!(synthesis, "Raw text response");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_strip_html_to_text() {
        let html =
            "<html><head><title>Test</title></head><body><p>Hello <b>world</b></p></body></html>";
        let text = strip_html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<p>"));
        assert!(!text.contains("<b>"));
    }

    #[test]
    fn test_strip_html_removes_scripts() {
        let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
        let text = strip_html_to_text(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_strip_html_removes_styles() {
        let html = "<p>Content</p><style>body { color: red; }</style>";
        let text = strip_html_to_text(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn test_strip_html_decodes_entities() {
        let html = "<p>&amp; &lt; &gt; &quot; &#39;</p>";
        let text = strip_html_to_text(html);
        assert!(text.contains("&"));
        assert!(text.contains("<"));
        assert!(text.contains(">"));
    }

    #[test]
    fn test_strip_html_truncation() {
        let html = format!("<p>{}</p>", "x".repeat(20000));
        let text = strip_html_to_text(&html);
        assert!(text.len() <= 8001); // 8000 chars + possible partial
    }

    #[test]
    fn test_format_report() {
        let report = ResearchReport {
            query: "Test query".to_string(),
            sub_queries: vec![SubQuery {
                query: "sub q".to_string(),
                rationale: "reason".to_string(),
            }],
            sources: vec![ResearchSource {
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                content: "Content here".to_string(),
                query: "sub q".to_string(),
                fetched_at: Utc::now(),
            }],
            synthesis: "Synthesis text".to_string(),
            key_findings: vec!["Finding 1".to_string(), "Finding 2".to_string()],
            sources_count: 1,
            duration_ms: 1500,
        };

        let formatted = format_report(&report);
        assert!(formatted.contains("# Deep Research: Test query"));
        assert!(formatted.contains("Finding 1"));
        assert!(formatted.contains("Finding 2"));
        assert!(formatted.contains("Synthesis text"));
        assert!(formatted.contains("https://example.com"));
        assert!(formatted.contains("1.5s"));
    }

    #[test]
    fn test_format_report_empty_sources() {
        let report = ResearchReport {
            query: "Empty".to_string(),
            sub_queries: vec![],
            sources: vec![],
            synthesis: "Nothing found".to_string(),
            key_findings: vec![],
            sources_count: 0,
            duration_ms: 500,
        };

        let formatted = format_report(&report);
        assert!(formatted.contains("0 sources"));
        assert!(formatted.contains("Nothing found"));
    }

    #[test]
    fn test_research_source_serialization() {
        let source = ResearchSource {
            url: "https://example.com".to_string(),
            title: "Test".to_string(),
            content: "Content".to_string(),
            query: "query".to_string(),
            fetched_at: Utc::now(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let deser: ResearchSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.url, "https://example.com");
        assert_eq!(deser.title, "Test");
    }

    #[test]
    fn test_research_report_serialization() {
        let report = ResearchReport {
            query: "test".to_string(),
            sub_queries: vec![],
            sources: vec![],
            synthesis: "result".to_string(),
            key_findings: vec!["f1".to_string()],
            sources_count: 0,
            duration_ms: 100,
        };
        let json = serde_json::to_string(&report).unwrap();
        let deser: ResearchReport = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.query, "test");
        assert_eq!(deser.synthesis, "result");
        assert_eq!(deser.key_findings, vec!["f1"]);
    }

    #[test]
    fn test_sub_query_serialization() {
        let sq = SubQuery {
            query: "search term".to_string(),
            rationale: "because".to_string(),
        };
        let json = serde_json::to_string(&sq).unwrap();
        let deser: SubQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.query, "search term");
        assert_eq!(deser.rationale, "because");
    }

    #[test]
    fn test_engine_creation() {
        let engine = ResearchEngine::new(ResearchConfig::default());
        assert_eq!(engine.config.max_queries, 5);
        assert_eq!(engine.config.max_sources, 3);
    }

    #[test]
    fn test_engine_from_env() {
        let engine = ResearchEngine::from_env();
        assert!(engine.config.max_queries > 0);
    }
}
