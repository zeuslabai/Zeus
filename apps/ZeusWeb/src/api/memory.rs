// Memory search, sync, tracked files, graph, patterns

use super::*;

pub async fn fetch_memory() -> Result<MemoryResponse, String> {
    fetch_json("/v1/memory").await
}

pub async fn fetch_memory_files() -> Result<MemoryFilesResponse, String> {
    fetch_json("/v1/memory/files").await
}

pub async fn fetch_memory_file(path: &str) -> Result<MemoryFileContent, String> {
    fetch_json(&format!("/v1/memory/files/{}", path)).await
}

pub async fn search_memory(query: &str) -> Result<MemorySearchResponse, String> {
    post_json("/v1/memory/search", &serde_json::json!({ "query": query })).await
}

pub async fn remember(fact: &str) -> Result<MsgResponse, String> {
    post_json("/v1/memory/remember", &serde_json::json!({ "fact": fact })).await
}

pub async fn add_note(content: &str) -> Result<MsgResponse, String> {
    post_json("/v1/memory/note", &serde_json::json!({ "content": content })).await
}

pub async fn fetch_reindex() -> Result<MemorySyncResponse, String> {
    post_json("/v1/memory/sync", &serde_json::json!({})).await
}

pub async fn fetch_tracked_files() -> Result<TrackedFilesResponse, String> {
    fetch_json("/v1/memory/files").await
}

pub async fn fetch_memory_timeline() -> Result<MemoryTimelineResponse, String> {
    fetch_json("/v1/memory/timeline").await
}

pub async fn fetch_memory_communities() -> Result<MemoryCommunitiesResponse, String> {
    fetch_json("/v1/memory/communities").await
}

pub async fn fetch_memory_graph(entity_id: &str) -> Result<MemoryGraphResponse, String> {
    fetch_json(&format!("/v1/memory/graph/{}", entity_id)).await
}

pub async fn search_memory_graph(query: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/memory/graph/search", &serde_json::json!({ "query": query })).await
}

pub async fn fetch_graph_nodes(limit: Option<usize>) -> Result<GraphNodesResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/memory/graph/nodes{}", q)).await
}

pub async fn fetch_graph_edges() -> Result<GraphEdgesResponse, String> {
    fetch_json("/v1/memory/graph/edges").await
}

pub async fn fetch_graph_stats() -> Result<MemoryGraphStats, String> {
    fetch_json("/v1/memory/graph/stats").await
}

pub async fn fetch_memory_patterns(limit: Option<usize>) -> Result<MemoryPatternsResponse, String> {
    let q = limit.map(|l| format!("?limit={}", l)).unwrap_or_default();
    fetch_json(&format!("/v1/memory/patterns{}", q)).await
}
