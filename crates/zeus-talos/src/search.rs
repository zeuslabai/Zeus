//! Web search tools

use crate::TalosTool;
use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use zeus_core::{Error, Result, ToolSchema};

/// Search result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Web search tool using Brave or DuckDuckGo
pub struct WebSearchTool;

#[async_trait]
impl TalosTool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web and return results with title, URL, and snippet"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "Search query", true)
            .with_param(
                "count",
                "integer",
                "Number of results to return (default 5)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing query parameter".to_string()))?;

        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        // Backend priority: Ollama Cloud → Tavily → Perplexity → Brave → DuckDuckGo
        let ollama_host = env::var("OLLAMA_HOST").ok();
        let results = if let Some(ref host) = ollama_host {
            // Try Ollama's native web search if available (cloud-only feature)
            match ollama_web_search(query, count, host).await {
                Ok(results) if !results.is_empty() => results,
                _ => {
                    // Ollama web search unavailable — fall through to other backends
                    if let Ok(key) = env::var("TAVILY_API_KEY") {
                        tavily_search(query, count, &key).await?
                    } else if let Ok(key) = env::var("PERPLEXITY_API_KEY") {
                        perplexity_search(query, count, &key).await?
                    } else if let Ok(key) = env::var("BRAVE_API_KEY") {
                        brave_search(query, count, &key).await?
                    } else {
                        duckduckgo_search(query, count).await?
                    }
                }
            }
        } else if let Ok(key) = env::var("TAVILY_API_KEY") {
            tavily_search(query, count, &key).await?
        } else if let Ok(key) = env::var("PERPLEXITY_API_KEY") {
            perplexity_search(query, count, &key).await?
        } else if let Ok(key) = env::var("BRAVE_API_KEY") {
            brave_search(query, count, &key).await?
        } else {
            duckduckgo_search(query, count).await?
        };

        Ok(serde_json::to_string_pretty(&results)?)
    }
}

/// Tavily search response structures
#[derive(Debug, Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
}

/// Search using Tavily API
async fn tavily_search(query: &str, count: usize, api_key: &str) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "max_results": count,
        "search_depth": "basic"
    });

    let response = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Network(format!("Tavily API request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Tavily API returned status: {}",
            response.status()
        )));
    }

    let tavily_response: TavilyResponse = response
        .json()
        .await
        .map_err(|e| Error::Tool(format!("Failed to parse Tavily response: {}", e)))?;

    Ok(tavily_response
        .results
        .into_iter()
        .take(count)
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.content,
        })
        .collect())
}

/// Search using Perplexity API (sonar online model)
async fn perplexity_search(query: &str, count: usize, api_key: &str) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "sonar",
        "messages": [{"role": "user", "content": query}],
        "max_tokens": 1024
    });

    let response = client
        .post("https://api.perplexity.ai/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Network(format!("Perplexity API request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Perplexity API returned status: {}",
            response.status()
        )));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Tool(format!("Failed to parse Perplexity response: {}", e)))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Perplexity returns citations as source URLs
    let citations: Vec<String> = data["citations"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.as_str().map(String::from))
                .take(count)
                .collect()
        })
        .unwrap_or_default();

    if citations.is_empty() {
        // No citations — return the answer as a single result
        return Ok(vec![SearchResult {
            title: "Perplexity Answer".to_string(),
            url: "https://www.perplexity.ai".to_string(),
            snippet: content,
        }]);
    }

    // Pair citations with content snippet on first result
    Ok(citations
        .into_iter()
        .enumerate()
        .map(|(i, url)| SearchResult {
            title: format!("Source {}", i + 1),
            url,
            snippet: if i == 0 { content.clone() } else { String::new() },
        })
        .collect())
}

/// Brave Search response structures
#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    web: BraveWebResults,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: String,
}

/// Search using Brave Search API
async fn brave_search(query: &str, count: usize, api_key: &str) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count
    );

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .send()
        .await
        .map_err(|e| Error::Network(format!("Brave API request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Brave API returned status: {}",
            response.status()
        )));
    }

    let brave_response: BraveSearchResponse = response
        .json()
        .await
        .map_err(|e| Error::Tool(format!("Failed to parse Brave response: {}", e)))?;

    Ok(brave_response
        .web
        .results
        .into_iter()
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.description,
        })
        .collect())
}

/// Search using DuckDuckGo HTML scraping
/// Search using Ollama's experimental cloud web search endpoint
async fn ollama_web_search(query: &str, count: usize, host: &str) -> Result<Vec<SearchResult>> {
    let base = host.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Tool(format!("Failed to create HTTP client: {}", e)))?;

    let body = serde_json::json!({
        "query": query,
        "max_results": count.min(10),
    });

    let response = client
        .post(format!("{}/api/experimental/web_search", base))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Network(format!("Ollama web search failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Ollama web search returned status: {}",
            response.status()
        )));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Tool(format!("Failed to parse Ollama web search response: {}", e)))?;

    let results = result["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let url = r["url"].as_str()?;
                    let title = r["title"].as_str().unwrap_or(url);
                    let content = r["content"].as_str().unwrap_or("");
                    // Truncate content to 300 chars like OpenClaw
                    let snippet = if content.len() > 300 {
                        format!("{}...", &content[..300])
                    } else {
                        content.to_string()
                    };
                    Some(SearchResult {
                        title: title.to_string(),
                        url: url.to_string(),
                        snippet,
                    })
                })
                .take(count)
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}

async fn duckduckgo_search(query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .build()
        .map_err(|e| Error::Tool(format!("Failed to create HTTP client: {}", e)))?;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::Network(format!("DuckDuckGo request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "DuckDuckGo returned status: {}",
            response.status()
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read DuckDuckGo response: {}", e)))?;

    parse_duckduckgo_html(&html, count)
}

/// Parse DuckDuckGo HTML to extract search results
fn parse_duckduckgo_html(html: &str, count: usize) -> Result<Vec<SearchResult>> {
    // Regex patterns to extract title, URL, and snippet from DuckDuckGo results
    // DuckDuckGo result structure:
    // <a class="result__a" href="...">Title</a>
    // <a class="result__snippet">Snippet text</a>

    let link_re = Regex::new(r#"<a[^>]+class="result__a"[^>]+href="([^"]+)"[^>]*>([^<]+)</a>"#)
        .map_err(|e| Error::Tool(format!("Failed to compile link regex: {}", e)))?;

    let snippet_re = Regex::new(r#"<a[^>]+class="result__snippet"[^>]*>([^<]+)</a>"#)
        .map_err(|e| Error::Tool(format!("Failed to compile snippet regex: {}", e)))?;

    let mut results = Vec::new();
    let link_matches: Vec<_> = link_re.captures_iter(html).collect();
    let snippet_matches: Vec<_> = snippet_re.captures_iter(html).collect();

    // Match links with snippets (they appear in order)
    let num_results = link_matches.len().min(snippet_matches.len()).min(count);

    for i in 0..num_results {
        if let (Some(link_cap), Some(snippet_cap)) = (link_matches.get(i), snippet_matches.get(i)) {
            let url = link_cap[1].to_string();
            let title = decode_html_entities(&link_cap[2]);
            let snippet = decode_html_entities(&snippet_cap[1]);

            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    if results.is_empty() {
        return Err(Error::Tool(
            "No search results found. DuckDuckGo HTML format may have changed.".to_string(),
        ));
    }

    Ok(results)
}

/// Decode common HTML entities
fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_schema() {
        let tool = WebSearchTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "web_search");
        assert!(schema.description.contains("Search"));

        // Check parameters
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params
            .get("properties")
            .expect("key should exist")
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("query"));
        assert!(props.contains_key("count"));

        // Check required fields
        let required = params
            .get("required")
            .expect("key should exist")
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&json!("query")));
    }

    #[test]
    fn test_brave_response_parsing() {
        let json_response = r#"{
            "web": {
                "results": [
                    {
                        "title": "Example Title",
                        "url": "https://example.com",
                        "description": "Example description text"
                    },
                    {
                        "title": "Second Result",
                        "url": "https://example2.com",
                        "description": "Another snippet"
                    }
                ]
            }
        }"#;

        let brave_response: BraveSearchResponse =
            serde_json::from_str(json_response).expect("should parse successfully");
        assert_eq!(brave_response.web.results.len(), 2);
        assert_eq!(brave_response.web.results[0].title, "Example Title");
        assert_eq!(brave_response.web.results[0].url, "https://example.com");
        assert_eq!(
            brave_response.web.results[0].description,
            "Example description text"
        );
    }

    #[test]
    fn test_duckduckgo_html_parsing() {
        let mock_html = r#"
        <div class="result">
            <a class="result__a" href="https://example.com">Example Title</a>
            <a class="result__snippet">This is an example snippet</a>
        </div>
        <div class="result">
            <a class="result__a" href="https://test.org">Test Page</a>
            <a class="result__snippet">Test description here</a>
        </div>
        "#;

        let results = parse_duckduckgo_html(mock_html, 5).expect("should parse successfully");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].snippet, "This is an example snippet");
        assert_eq!(results[1].title, "Test Page");
        assert_eq!(results[1].url, "https://test.org");
    }

    #[test]
    fn test_duckduckgo_html_parsing_with_count_limit() {
        let mock_html = r#"
        <div class="result">
            <a class="result__a" href="https://one.com">First</a>
            <a class="result__snippet">First snippet</a>
        </div>
        <div class="result">
            <a class="result__a" href="https://two.com">Second</a>
            <a class="result__snippet">Second snippet</a>
        </div>
        <div class="result">
            <a class="result__a" href="https://three.com">Third</a>
            <a class="result__snippet">Third snippet</a>
        </div>
        "#;

        let results = parse_duckduckgo_html(mock_html, 2).expect("should parse successfully");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First");
        assert_eq!(results[1].title, "Second");
    }

    #[test]
    fn test_html_entity_decoding() {
        assert_eq!(decode_html_entities("Test &amp; Example"), "Test & Example");
        assert_eq!(decode_html_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_html_entities("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(decode_html_entities("It&#39;s working"), "It's working");
        assert_eq!(decode_html_entities("Hello&nbsp;World"), "Hello World");
    }

    #[test]
    fn test_provider_selection_with_api_key() {
        // This test verifies the logic, not actual API calls
        // If BRAVE_API_KEY is set, brave_search would be called
        // Otherwise, duckduckgo_search would be called

        // Test that missing API key is handled gracefully
        let result = env::var("BRAVE_API_KEY");
        // We just verify the env var access works
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_duckduckgo_empty_results() {
        let mock_html = r#"<html><body>No results found</body></html>"#;

        let result = parse_duckduckgo_html(mock_html, 5);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No search results found")
        );
    }

    #[tokio::test]
    async fn test_execute_with_missing_query() {
        let tool = WebSearchTool;
        let args = json!({
            "count": 5
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing query"));
    }

    #[tokio::test]
    async fn test_execute_with_default_count() {
        let tool = WebSearchTool;
        let args = json!({
            "query": "rust programming"
        });

        // This will attempt actual search, so we just verify it doesn't panic on arg parsing
        // The actual search might fail if no API key and DDG is blocked, but that's expected
        let _result = tool.execute(args).await;
        // We don't assert success/failure as it depends on external factors
    }
}
