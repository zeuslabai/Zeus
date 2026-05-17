// Tools listing, execution, MCP, skills, extensions, sandbox, web search, image gen

use super::*;

// Skills

pub async fn fetch_skills() -> Result<SkillsResponse, String> {
    fetch_json("/v1/skills").await
}

pub async fn install_skill(req: &InstallSkillReq) -> Result<MsgResponse, String> {
    post_json("/v1/skills", req).await
}

pub async fn toggle_skill(id: &str, enable: bool) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/skills/{}", id), &serde_json::json!({ "enabled": enable })).await
}

pub async fn delete_skill(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/skills/{}", id)).await
}

pub async fn fetch_skill(id: &str) -> Result<Skill, String> {
    fetch_json(&format!("/v1/skills/{}", id)).await
}

pub async fn search_skills(query: Option<&str>, category: Option<&str>) -> Result<SkillsResponse, String> {
    let mut params = Vec::new();
    if let Some(q) = query
        && !q.is_empty()
    {
        params.push(format!("q={}", q));
    }
    if let Some(cat) = category
        && !cat.is_empty()
    {
        params.push(format!("category={}", cat));
    }
    let url = if params.is_empty() {
        "/v1/skills/search".to_string()
    } else {
        format!("/v1/skills/search?{}", params.join("&"))
    };
    fetch_json(&url).await
}

pub async fn fetch_skill_categories() -> Result<SkillCategoriesResponse, String> {
    fetch_json("/v1/skills/categories").await
}

// MCP

pub async fn fetch_mcp_servers() -> Result<McpServersResponse, String> {
    fetch_json("/v1/mcp/servers").await
}

pub async fn connect_mcp(req: &ConnectMcpReq) -> Result<MsgResponse, String> {
    post_json("/v1/mcp/servers", req).await
}

pub async fn disconnect_mcp(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/mcp/servers/{}", id)).await
}

pub async fn fetch_mcp_tools(server_id: &str) -> Result<McpToolsResponse, String> {
    fetch_json(&format!("/v1/mcp/servers/{}/tools", server_id)).await
}

pub async fn test_mcp_tool(tool_name: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/mcp/tools/{}/test", tool_name), &serde_json::json!({})).await
}

// Tools

/// Fetch all available tools from the API and derive categories.
pub async fn get_tools() -> Result<ToolsResponse, String> {
    let mut resp: ToolsResponse = fetch_json("/v1/tools").await?;
    resp.tools = resp.tools.into_iter().map(|t| t.with_derived_category()).collect();
    Ok(resp)
}

/// Get a single tool by name (filters from the full list).
pub async fn get_tool(name: &str) -> Result<Option<ToolDef>, String> {
    let resp = get_tools().await?;
    Ok(resp.tools.into_iter().find(|t| t.name == name))
}

/// Backwards-compatible alias.
pub async fn fetch_tools() -> Result<ToolsResponse, String> {
    get_tools().await
}

pub async fn execute_tool(name: &str, args: &serde_json::Value) -> Result<ToolExecResponse, String> {
    post_json(&format!("/v1/tools/{}", name), &serde_json::json!({ "arguments": args })).await
}

pub async fn fetch_tool_detail(name: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/tools/{}", name)).await
}

// Extensions

pub async fn fetch_extensions() -> Result<ExtensionsResponse, String> {
    fetch_json("/v1/extensions").await
}

pub async fn install_extension(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/extensions", body).await
}

pub async fn toggle_extension(id: &str, enabled: bool) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/extensions/{}", id), &serde_json::json!({"enabled": enabled})).await
}

pub async fn delete_extension(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/extensions/{}", id)).await
}

pub async fn start_extension(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/extensions/{}/start", id), &serde_json::json!({})).await
}

pub async fn stop_extension(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/extensions/{}/stop", id), &serde_json::json!({})).await
}

// Sandbox

pub async fn fetch_sandbox_policies() -> Result<SandboxPoliciesResponse, String> {
    fetch_json("/v1/sandbox/policies").await
}

pub async fn create_sandbox_policy(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/sandbox/policies", body).await
}

pub async fn delete_sandbox_policy(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/sandbox/policies/{}", id)).await
}

pub async fn run_sandbox_command(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/sandbox/execute", body).await
}

// Web search

pub async fn web_search(query: &str) -> Result<Vec<SearchResult>, String> {
    let encoded = js_sys::encode_uri_component(query);
    let mut results = Vec::new();

    // 1) Wikipedia search
    let wiki_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=5",
        encoded
    );
    let wiki_tool_body = serde_json::json!({
        "arguments": { "url": wiki_url }
    });
    match post_json::<serde_json::Value, ToolExecResp>("/v1/tools/web_fetch", &wiki_tool_body).await {
        Ok(resp) if resp.success => {
            if let Ok(wiki) = serde_json::from_str::<WikiSearchResp>(&resp.output) {
                for r in wiki.query.search {
                    let snippet = r.snippet
                        .replace("<span class=\"searchmatch\">", "")
                        .replace("</span>", "")
                        .replace("&quot;", "\"")
                        .replace("&amp;", "&");
                    let url = format!(
                        "https://en.wikipedia.org/wiki/{}",
                        r.title.replace(' ', "_")
                    );
                    results.push(SearchResult {
                        title: r.title,
                        snippet,
                        url,
                    });
                }
            }
        }
        _ => {}
    }

    // 2) DuckDuckGo instant answer API
    let ddg_url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_redirect=1&no_html=1",
        encoded
    );
    let ddg_tool_body = serde_json::json!({
        "arguments": { "url": ddg_url }
    });
    match post_json::<serde_json::Value, ToolExecResp>("/v1/tools/web_fetch", &ddg_tool_body).await {
        Ok(resp) if resp.success => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&resp.output) {
                if let Some(abs) = val.get("AbstractText").and_then(|v| v.as_str())
                    && !abs.is_empty() {
                        let url = val.get("AbstractURL").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let src = val.get("AbstractSource").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        results.insert(0, SearchResult {
                            title: format!("{} ({})", val.get("Heading").and_then(|v| v.as_str()).unwrap_or(""), src),
                            snippet: abs.to_string(),
                            url,
                        });
                    }
                if let Some(topics) = val.get("RelatedTopics").and_then(|v| v.as_array()) {
                    for topic in topics.iter().take(3) {
                        if let (Some(text), Some(url)) = (
                            topic.get("Text").and_then(|v| v.as_str()),
                            topic.get("FirstURL").and_then(|v| v.as_str()),
                        ) {
                            results.push(SearchResult {
                                title: text.split(" - ").next().unwrap_or(text).to_string(),
                                snippet: text.to_string(),
                                url: url.to_string(),
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(results)
}

// Image generation

/// Generate an image using Fooocus API (proxied through nginx at /imggen/).
pub async fn generate_image(prompt: &str, style: Option<&str>, size: Option<&str>) -> Result<String, String> {
    let style_selection = style.unwrap_or("Fooocus V2").to_string();
    let aspect = size.unwrap_or("1024\u{00d7}1024").to_string();

    let body = FooocusRequest {
        prompt: prompt.to_string(),
        negative_prompt: String::new(),
        style_selections: vec![style_selection],
        performance_selection: "Speed".to_string(),
        aspect_ratios_selection: aspect,
        image_number: 1,
        output_format: "png".to_string(),
        image_seed: -1,
    };

    let resp = gloo_net::http::Request::post("/imggen/v1/generation/text-to-image-with-ip")
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).map_err(|e| format!("serialize: {}", e))?)
        .map_err(|e| format!("request: {}", e))?
        .send()
        .await
        .map_err(|e| {
            format!("Image generation server (Fooocus) is not currently running or unreachable: {}", e)
        })?;

    if !resp.ok() {
        let status = resp.status();
        if status == 502 || status == 503 {
            return Err("Image generation server (Fooocus) is not currently running. Start it and try again.".to_string());
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Image generation failed (HTTP {}): {}", status, body));
    }

    let text = resp.text().await.map_err(|e| format!("read response: {}", e))?;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| format!("parse response: {}", e))?;

    if let Some(first) = arr.first() {
        if let Some(b64) = first.get("base64").and_then(|v| v.as_str()) {
            return Ok(b64.to_string());
        }
        if let Some(url) = first.get("url").and_then(|v| v.as_str()) {
            return Ok(url.to_string());
        }
    }

    Err("No image data in response".to_string())
}
