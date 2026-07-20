//! Memory & workspace file API handlers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::SharedState;

use std::path::PathBuf;

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RememberRequest {
    pub fact: String,
    /// Optional scope: "global" (default) | "workspace" | "user" (#433).
    #[serde(default)]
    pub scope: Option<String>,
    /// Scope target for "workspace" scope; ignored otherwise.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Scope target for "user" scope; ignored otherwise.
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NoteRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteMemoryFileRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchMemoryRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// Optional scope filter: "global" | "workspace" | "user" (#433).
    /// Absent = unfiltered (legacy behavior).
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

fn default_search_limit() -> usize {
    10
}

// ============================================================================
// Memory Context Endpoints
// ============================================================================

pub async fn get_memory(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let context = state
        .workspace
        .get_context()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let memory = state.workspace.get_memory().await.unwrap_or_default();
    let daily = state.workspace.get_daily().await.unwrap_or_default();

    Ok(Json(json!({
        "context_length": context.len(),
        "memory": memory,
        "daily": daily
    })))
}

pub async fn remember(
    State(state): State<SharedState>,
    Json(req): Json<RememberRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Resolve the requested scope (#433). Fail-closed: an unknown scope or a
    // non-global scope missing its id is a 400, never a silent global write.
    let scope = parse_memory_scope(
        req.scope.as_deref(),
        req.workspace_id.as_deref(),
        req.user_id.as_deref(),
    )?;

    state
        .workspace
        .remember_scoped(&req.fact, scope.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "scope": scope_label(&scope),
        "message": format!("Remembered: {}", req.fact)
    })))
}

/// Parse the optional scope triple from a remember request into a
/// `zeus_memory::MemoryScope`. Defaults to Global when `scope` is absent.
fn parse_memory_scope(
    scope: Option<&str>,
    workspace_id: Option<&str>,
    user_id: Option<&str>,
) -> Result<zeus_memory::MemoryScope, (StatusCode, String)> {
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, msg.to_string());
    match scope.unwrap_or("global") {
        "global" => Ok(zeus_memory::MemoryScope::Global),
        "workspace" => workspace_id
            .filter(|s| !s.is_empty())
            .map(|s| zeus_memory::MemoryScope::Workspace(s.to_string()))
            .ok_or_else(|| bad("workspace scope requires workspace_id")),
        "user" => user_id
            .filter(|s| !s.is_empty())
            .map(|s| zeus_memory::MemoryScope::User(s.to_string()))
            .ok_or_else(|| bad("user scope requires user_id")),
        other => Err(bad(&format!(
            "unknown scope '{}' (expected global|workspace|user)",
            other
        ))),
    }
}

fn scope_label(scope: &zeus_memory::MemoryScope) -> &'static str {
    match scope {
        zeus_memory::MemoryScope::Global => "global",
        zeus_memory::MemoryScope::Workspace(_) => "workspace",
        zeus_memory::MemoryScope::User(_) => "user",
    }
}

/// Flatten a `MemoryScope` into the (scope, scope_id) pair the Mnemosyne
/// scoped search expects (#433).
fn scope_parts(scope: &zeus_memory::MemoryScope) -> (&'static str, Option<&str>) {
    match scope {
        zeus_memory::MemoryScope::Global => ("global", None),
        zeus_memory::MemoryScope::Workspace(id) => ("workspace", Some(id.as_str())),
        zeus_memory::MemoryScope::User(id) => ("user", Some(id.as_str())),
    }
}

pub async fn add_note(
    State(state): State<SharedState>,
    Json(req): Json<NoteRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    state
        .workspace
        .note(&req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "message": format!("Added note: {}", req.content)
    })))
}

// ============================================================================
// Memory File Endpoints
// ============================================================================

/// List workspace files with metadata
pub async fn list_memory_files(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let root = state.workspace.root().to_path_buf();

    if !root.exists() {
        return Ok(Json(json!({ "files": [] })));
    }

    let mut files = Vec::new();
    collect_files(&root, &root, &mut files)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "files": files })))
}

/// Verify that `rel` resolves inside `root`, guarding against path traversal.
///
/// Uses `std::fs::canonicalize` + prefix check so that symlinks and encoded
/// `..` sequences cannot escape the allowed directory. Handles paths that do
/// not yet exist (write / create) by canonicalizing the nearest existing
/// ancestor and re-attaching the remaining components.
pub(crate) async fn verify_safe_path(
    root: std::path::PathBuf,
    rel: String,
) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let canonical_root = tokio::fs::canonicalize(&root).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Root directory inaccessible: {e}"),
        )
    })?;

    let full = root.join(&rel);

    let canonical_full = match tokio::fs::canonicalize(&full).await {
        Ok(p) => p,
        Err(_) => {
            // Path does not exist yet (create / write).
            // Walk up to the nearest existing ancestor, canonicalize that,
            // then re-join the non-existing tail components.
            let mut existing = full.clone();
            let mut tail = std::path::PathBuf::new();
            while !existing.exists() {
                if let Some(name) = existing.file_name() {
                    tail = std::path::Path::new(name).join(&tail);
                } else {
                    break;
                }
                match existing.parent() {
                    Some(p) => existing = p.to_path_buf(),
                    None => break,
                }
            }
            match tokio::fs::canonicalize(&existing).await {
                Ok(c) => c.join(&tail),
                Err(_) => full.clone(),
            }
        }
    };

    if !canonical_full.starts_with(&canonical_root) {
        warn!("Path traversal attempt blocked: {}", rel);
        return Err((
            StatusCode::BAD_REQUEST,
            "Path traversal not allowed".to_string(),
        ));
    }

    Ok(canonical_full)
}

/// Recursively collect files from workspace directory
fn collect_files(base: &PathBuf, dir: &PathBuf, files: &mut Vec<Value>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let metadata = std::fs::metadata(&path)?;
            let modified = metadata
                .modified()
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();

            files.push(json!({
                "path": relative,
                "size": metadata.len(),
                "modified": modified,
            }));
        }
    }
    Ok(())
}

/// Read a specific workspace file
pub async fn read_memory_file(
    State(state): State<SharedState>,
    Path(path): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Strip leading slash from catch-all path
    let clean_path = path.trim_start_matches('/');

    let state = state.read().await;

    // Security: canonicalize + prefix check prevents all traversal variants
    verify_safe_path(state.workspace.root().to_path_buf(), clean_path.to_string()).await?;

    let content = state.workspace.read(clean_path).await.map_err(|e| {
        if e.to_string().contains("traversal") {
            (
                StatusCode::BAD_REQUEST,
                "Path traversal not allowed".to_string(),
            )
        } else {
            (
                StatusCode::NOT_FOUND,
                format!("File not found: {}", clean_path),
            )
        }
    })?;

    let full_path = state.workspace.root().join(clean_path);
    let (size, modified) = match tokio::fs::metadata(&full_path).await {
        Ok(meta) => {
            let modified = meta
                .modified()
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();
            (meta.len(), modified)
        }
        Err(_) => (content.len() as u64, String::new()),
    };

    Ok(Json(json!({
        "path": clean_path,
        "content": content,
        "size": size,
        "modified": modified,
    })))
}

/// Write/update a workspace file
pub async fn write_memory_file(
    State(state): State<SharedState>,
    Path(path): Path<String>,
    Json(req): Json<WriteMemoryFileRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let clean_path = path.trim_start_matches('/');

    let state = state.read().await;

    // Security: canonicalize + prefix check prevents all traversal variants
    verify_safe_path(state.workspace.root().to_path_buf(), clean_path.to_string()).await?;

    state
        .workspace
        .write(clean_path, &req.content)
        .await
        .map_err(|e| {
            if e.to_string().contains("traversal") {
                (
                    StatusCode::BAD_REQUEST,
                    "Path traversal not allowed".to_string(),
                )
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;

    Ok(Json(json!({
        "success": true,
        "path": clean_path,
        "size": req.content.len()
    })))
}

/// POST /v1/memory/files — Create a new memory file
pub async fn create_memory_file(
    State(state): State<SharedState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = req.get("path").and_then(|p| p.as_str()).unwrap_or("");
    let content = req.get("content").and_then(|c| c.as_str()).unwrap_or("");

    if path.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing 'path' field".to_string()));
    }

    let clean_path = path.trim_start_matches('/');

    let state = state.read().await;

    // Security: canonicalize + prefix check prevents all traversal variants
    verify_safe_path(state.workspace.root().to_path_buf(), clean_path.to_string()).await?;

    let full_path = state.workspace.root().join(clean_path);

    if full_path.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("File already exists: {}", clean_path),
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    tokio::fs::write(&full_path, content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Created memory file: {}", clean_path);

    Ok(Json(json!({
        "success": true,
        "path": clean_path,
        "size": content.len()
    })))
}

/// DELETE /v1/memory/files/*path — Delete a memory file
pub async fn delete_memory_file(
    State(state): State<SharedState>,
    Path(path): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let clean_path = path.trim_start_matches('/');

    let state = state.read().await;

    // Security: canonicalize + prefix check prevents all traversal variants
    verify_safe_path(state.workspace.root().to_path_buf(), clean_path.to_string()).await?;

    let full_path = state.workspace.root().join(clean_path);

    if !full_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("File not found: {}", clean_path),
        ));
    }

    tokio::fs::remove_file(&full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Deleted memory file: {}", clean_path);

    Ok(Json(json!({
        "success": true,
        "path": clean_path,
        "message": format!("File '{}' deleted", clean_path)
    })))
}

/// Search across workspace memory files
pub async fn search_memory(
    State(state): State<SharedState>,
    Json(req): Json<SearchMemoryRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let root = state.workspace.root().to_path_buf();
    let limit = req.limit.min(zeus_core::MAX_PAGE_LIMIT_SMALL);

    // Resolve the optional scope filter (#433). Absent scope = unfiltered
    // legacy behavior; present-but-invalid scope = 400, never silent.
    let scope = parse_memory_scope(
        req.scope.as_deref(),
        req.workspace_id.as_deref(),
        req.user_id.as_deref(),
    )?;
    let scoped = req.scope.is_some();

    // Use Mnemosyne hybrid search when available
    if let Some(ref mnemosyne) = state.mnemosyne {
        let mn = mnemosyne.clone();
        drop(state);
        let result = if scoped {
            let (s, id) = scope_parts(&scope);
            mn.semantic_search_scoped(&req.query, limit, Some((s, id))).await
        } else {
            mn.semantic_search(&req.query, limit).await
        };
        match result {
            Ok(hits) => {
                let results: Vec<Value> = hits
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "session_id": r.session_id,
                            "content": r.content,
                            "score": r.score,
                            "memory_type": format!("{:?}", r.memory_type),
                            "importance": r.importance,
                        })
                    })
                    .collect();
                return Ok(Json(json!({
                    "results": results,
                    "search_method": "hybrid",
                })));
            }
            Err(_) => {
                // Fall through to file-based search
            }
        }
    } else {
        drop(state);
    }

    if !root.exists() {
        return Ok(Json(json!({ "results": [], "search_method": "file" })));
    }

    let query_lower = req.query.to_lowercase();
    let mut results = Vec::new();

    search_files_recursive(&root, &root, &query_lower, limit, &mut results);

    // Sort by score descending
    results.sort_by(|a, b| {
        let sa = a["score"].as_f64().unwrap_or(0.0);
        let sb = b["score"].as_f64().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    results.truncate(limit);

    Ok(Json(json!({ "results": results, "search_method": "file" })))
}

/// Recursively search files for query
fn search_files_recursive(
    base: &PathBuf,
    dir: &PathBuf,
    query: &str,
    limit: usize,
    results: &mut Vec<Value>,
) {
    if results.len() >= limit || !dir.exists() {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if results.len() >= limit {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            search_files_recursive(base, &path, query, limit, results);
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            let content_lower = content.to_lowercase();
            if content_lower.contains(query) {
                let relative = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                // Find snippet around match
                let snippet = extract_snippet(&content, query);

                // Simple scoring: count occurrences
                let count = content_lower.matches(query).count();
                let score = (count as f64 / content.len().max(1) as f64 * 100.0).min(1.0);

                results.push(json!({
                    "path": relative,
                    "snippet": snippet,
                    "score": (score * 100.0).round() / 100.0,
                }));
            }
        }
    }
}

/// Extract a snippet around the first match occurrence
fn extract_snippet(content: &str, query: &str) -> String {
    let lower = content.to_lowercase();
    if let Some(pos) = lower.find(query) {
        let start = pos.saturating_sub(50);
        let end = (pos + query.len() + 50).min(content.len());
        let mut snippet = content[start..end].to_string();
        if start > 0 {
            snippet = format!("...{}", snippet);
        }
        if end < content.len() {
            snippet = format!("{}...", snippet);
        }
        snippet
    } else {
        content.chars().take(100).collect()
    }
}

/// GET /v1/memory/timeline — Recent memory file changes
pub async fn memory_timeline(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let root = state.workspace.root().to_path_buf();
    let mut entries = Vec::new();

    if root.exists() {
        collect_timeline_entries(&root, &root, &mut entries);
    }

    // Sort by modification time descending (most recent first)
    entries.sort_by(|a, b| {
        let ta = a["timestamp"].as_str().unwrap_or("");
        let tb = b["timestamp"].as_str().unwrap_or("");
        tb.cmp(ta)
    });

    entries.truncate(50);

    Json(json!({ "entries": entries }))
}

fn collect_timeline_entries(base: &PathBuf, dir: &PathBuf, entries: &mut Vec<Value>) {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_timeline_entries(base, &path, entries);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let relative = path
                .strip_prefix(base)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();
            let action = if relative.starts_with("daily/") {
                "note"
            } else if relative == "MEMORY.md" {
                "remember"
            } else {
                "write"
            };
            entries.push(json!({
                "timestamp": modified,
                "path": relative,
                "action": action,
                "source": "workspace"
            }));
        }
    }
}

// ============================================================================
// Graph Memory Endpoints (Sprint 9)
// ============================================================================

/// GET /v1/memory/graph/:entity_id — Get entity graph neighborhood.
pub async fn get_entity_graph(
    State(state): State<SharedState>,
    axum::extract::Path(entity_id): axum::extract::Path<i64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;

    let entity = store
        .get_entity_by_id(entity_id)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Entity not found: {}", e)))?;

    let rels = store
        .get_relationships(entity_id, zeus_mnemosyne::Direction::Both)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let relationships: Vec<Value> = rels
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "source_entity_id": r.source_entity_id,
                "target_entity_id": r.target_entity_id,
                "relationship_type": r.relationship_type.as_label(),
                "weight": r.weight,
                "mention_count": r.mention_count,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "entity": {
            "id": entity.id,
            "name": entity.canonical_name,
            "type": entity.entity_type,
            "aliases": entity.aliases,
            "mention_count": entity.mention_count,
        },
        "relationships": relationships,
        "relationship_count": rels.len(),
    })))
}

/// GET /v1/memory/communities — List all detected communities.
pub async fn list_communities(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;

    let communities = store
        .get_communities()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let results: Vec<Value> = communities
        .iter()
        .map(|c| {
            let summary = zeus_mnemosyne::community::community_summary(&store, c.id);
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "description": c.description,
                "entity_count": c.entity_count,
                "summary": summary,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "communities": results,
        "count": communities.len(),
    })))
}

/// POST /v1/memory/graph/search — Graph-augmented search.
pub async fn graph_search(
    State(state): State<SharedState>,
    Json(req): Json<SearchMemoryRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;
    let limit = req.limit.min(50);

    let results = zeus_mnemosyne::graph_augmented_search(&store, &req.query, limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<Value> = results
        .iter()
        .map(|gr| {
            serde_json::json!({
                "id": gr.result.id,
                "content": gr.result.content,
                "score": gr.result.score,
                "memory_type": format!("{:?}", gr.result.memory_type),
                "graph_context": gr.context_text,
                "entities": gr.graph_context.entities.iter().map(|e| {
                    serde_json::json!({"id": e.id, "name": e.canonical_name, "type": e.entity_type})
                }).collect::<Vec<Value>>(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "results": hits,
        "count": results.len(),
        "search_method": "graph_augmented",
    })))
}

// ============================================================================
// Memory Sync Endpoint
// ============================================================================

/// POST /v1/memory/sync — Sync workspace files into Mnemosyne with embedding cache.
pub async fn sync_memory(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let mnemosyne = match &state.mnemosyne {
        Some(m) => m.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Mnemosyne is not configured. Enable [mnemosyne] in config.toml.".to_string(),
            ));
        }
    };

    let root = state.workspace.root().to_path_buf();
    let sessions_dir = state.config.sessions.clone();
    drop(state); // Release read lock during sync

    match mnemosyne.sync_workspace_with_cache_stats(&root).await {
        Ok(mut stats) => {
            // Also sync session transcripts
            match mnemosyne.sync_sessions(&sessions_dir).await {
                Ok(count) => stats.sessions_indexed = count,
                Err(e) => stats.errors.push(format!("Session sync: {}", e)),
            }
            Ok(Json(json!({
                "status": "ok",
                "files_scanned": stats.files_scanned,
                "files_changed": stats.files_changed,
                "files_unchanged": stats.files_unchanged,
                "chunks_embedded": stats.chunks_embedded,
                "cache_hits": stats.cache_hits,
                "cache_misses": stats.cache_misses,
                "sessions_indexed": stats.sessions_indexed,
                "errors": stats.errors,
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Sync failed: {}", e),
        )),
    }
}

// ============================================================================
// Context Journal Endpoints
// ============================================================================

/// List all context journal files
pub async fn list_context_journals(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    let journal_dir = state
        .config
        .prometheus
        .as_ref()
        .and_then(|p| p.context_journal.as_ref())
        .map(|cj| zeus_core::default_config_dir().join(&cj.path))
        .unwrap_or_else(|| zeus_core::default_config_dir().join("context-journals"));

    let threshold_pct = state
        .config
        .prometheus
        .as_ref()
        .and_then(|p| p.context_journal.as_ref())
        .map(|cj| cj.threshold_pct)
        .unwrap_or(10);

    let journal = zeus_session::ContextJournal::new(journal_dir, threshold_pct);
    match journal.list_journals() {
        Ok(summaries) => {
            let count = summaries.len();
            Json(json!({
                "journals": summaries,
                "count": count,
            }))
        }
        Err(e) => Json(json!({
            "journals": [],
            "count": 0,
            "error": e.to_string(),
        })),
    }
}
