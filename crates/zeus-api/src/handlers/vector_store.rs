//! Vector Store & File Search API handlers
//!
//! OpenAI-compatible vector store management endpoints:
//! - POST   /v1/vector_stores           — Create a vector store
//! - GET    /v1/vector_stores           — List vector stores
//! - GET    /v1/vector_stores/:id       — Get vector store details
//! - DELETE /v1/vector_stores/:id       — Delete a vector store
//! - POST   /v1/vector_stores/:id/search — Search within a vector store
//! - POST   /v1/vector_stores/:id/files  — Add file to vector store
//! - GET    /v1/vector_stores/:id/files  — List files in vector store

use anyhow::{Context as _, Result};
use axum::{
    Json,
    extract::{Path, State},
};
use dashmap::DashMap;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

use crate::SharedState;

// ============================================================================
// Types
// ============================================================================

/// A named collection of documents with vector embeddings for semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStore {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub file_count: usize,
    pub status: VectorStoreStatus,
    pub created_at: String,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreStatus {
    Active,
    Expired,
    Indexing,
}

#[derive(Debug, Deserialize)]
pub struct CreateVectorStoreRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Search mode: "hybrid" (default), "fts" (text only), "vector" (semantic only)
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_limit() -> usize {
    10
}
fn default_mode() -> String {
    "hybrid".to_string()
}

#[derive(Debug, Deserialize)]
pub struct AddFileRequest {
    /// File path or content to index
    pub content: String,
    /// Source identifier (filename, URL, etc.)
    pub source: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub metadata: Option<Value>,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /v1/vector_stores — Create a new vector store
pub async fn create_vector_store(
    State(state): State<SharedState>,
    Json(req): Json<CreateVectorStoreRequest>,
) -> Json<Value> {
    let state = state.read().await;

    // Check Mnemosyne availability
    if state.mnemosyne.is_none() {
        return Json(json!({
            "error": "Mnemosyne not configured. Enable [mnemosyne] in config.toml.",
        }));
    }

    let store = VectorStore {
        id: format!("vs_{}", &Uuid::new_v4().to_string().replace('-', "")[..24]),
        name: req.name,
        description: req.description,
        file_count: 0,
        status: VectorStoreStatus::Active,
        created_at: chrono::Utc::now().to_rfc3339(),
        metadata: req.metadata,
    };

    let db = state.vector_store_db.clone();
    let registry = get_registry(&db);
    let response = json!({
        "id": store.id,
        "object": "vector_store",
        "name": store.name,
        "description": store.description,
        "file_counts": { "total": 0, "completed": 0, "in_progress": 0, "failed": 0 },
        "status": store.status,
        "created_at": store.created_at,
        "metadata": store.metadata,
    });

    registry.insert(store.id.clone(), store.clone());
    if let Err(e) = db.save(&store) {
        tracing::warn!("Failed to persist vector store {}: {e}", store.id);
    }

    Json(response)
}

/// GET /v1/vector_stores — List all vector stores
pub async fn list_vector_stores(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let registry = get_registry(&state.vector_store_db.clone());

    let stores: Vec<Value> = registry
        .iter()
        .map(|entry| {
            let s = entry.value();
            json!({
                "id": s.id,
                "object": "vector_store",
                "name": s.name,
                "description": s.description,
                "file_counts": { "total": s.file_count },
                "status": s.status,
                "created_at": s.created_at,
            })
        })
        .collect();

    Json(json!({
        "object": "list",
        "data": stores,
    }))
}

/// GET /v1/vector_stores/:id — Get vector store details
pub async fn get_vector_store(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    let registry = get_registry(&state.vector_store_db.clone());

    match registry.get(&id) {
        Some(entry) => {
            let s = entry.value();
            Json(json!({
                "id": s.id,
                "object": "vector_store",
                "name": s.name,
                "description": s.description,
                "file_counts": { "total": s.file_count, "completed": s.file_count },
                "status": s.status,
                "created_at": s.created_at,
                "metadata": s.metadata,
            }))
        }
        None => Json(json!({
            "error": { "message": format!("No vector store with id '{}'", id), "type": "not_found" }
        })),
    }
}

/// DELETE /v1/vector_stores/:id — Delete a vector store
pub async fn delete_vector_store(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    let db = state.vector_store_db.clone();
    let registry = get_registry(&db);

    match registry.remove(&id) {
        Some((_, store)) => {
            if let Err(e) = db.delete(&store.id) {
                tracing::warn!(
                    "Failed to delete vector store {} from SQLite: {e}",
                    store.id
                );
            }
            Json(json!({
                "id": store.id,
                "object": "vector_store.deleted",
                "deleted": true,
            }))
        }
        None => Json(json!({
            "error": { "message": format!("No vector store with id '{}'", id), "type": "not_found" }
        })),
    }
}

/// POST /v1/vector_stores/:id/search — Search within a vector store
pub async fn search_vector_store(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<SearchRequest>,
) -> Json<Value> {
    let state = state.read().await;
    let registry = get_registry(&state.vector_store_db.clone());

    if !registry.contains_key(&id) {
        return Json(json!({
            "error": { "message": format!("No vector store with id '{}'", id), "type": "not_found" }
        }));
    }

    let limit = req.limit.min(50);

    match &state.mnemosyne {
        Some(mn) => {
            // FTS search (vector/hybrid require embeddings — use FTS for now)
            let results = mn.search(&req.query, limit).await;

            match results {
                Ok(hits) => {
                    let data: Vec<Value> = hits
                        .iter()
                        .map(|r| {
                            json!({
                                "content": r.content,
                                "score": r.score,
                                "timestamp": r.timestamp,
                                "memory_type": format!("{:?}", r.memory_type),
                                "importance": r.importance,
                            })
                        })
                        .collect();

                    Json(json!({
                        "object": "list",
                        "data": data,
                        "search_query": req.query,
                        "mode": req.mode,
                    }))
                }
                Err(e) => Json(json!({
                    "error": { "message": format!("Search failed: {}", e), "type": "search_error" }
                })),
            }
        }
        None => Json(json!({
            "error": "Mnemosyne not configured. Enable [mnemosyne] in config.toml.",
        })),
    }
}

/// POST /v1/vector_stores/:id/files — Add file/content to a vector store
pub async fn add_file_to_store(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<AddFileRequest>,
) -> Json<Value> {
    let state = state.read().await;
    let db = state.vector_store_db.clone();
    let registry = get_registry(&db);

    if !registry.contains_key(&id) {
        return Json(json!({
            "error": { "message": format!("No vector store with id '{}'", id), "type": "not_found" }
        }));
    }

    match &state.mnemosyne {
        Some(mn) => {
            let file_id = format!(
                "file_{}",
                &Uuid::new_v4().to_string().replace('-', "")[..24]
            );

            // Store into Mnemosyne with source tracking
            let store = mn.store_ref().lock().await;
            match store.store_chunk_with_source(
                &id,
                &req.content,
                &req.source,
                zeus_mnemosyne::MemoryType::Semantic,
            ) {
                Ok(_) => {
                    // Update file count in memory and persist
                    let new_count = if let Some(mut entry) = registry.get_mut(&id) {
                        entry.file_count += 1;
                        entry.file_count
                    } else {
                        1
                    };
                    if let Err(e) = db.update_file_count(&id, new_count) {
                        tracing::warn!("Failed to persist file_count for store {id}: {e}");
                    }

                    Json(json!({
                        "id": file_id,
                        "object": "vector_store.file",
                        "vector_store_id": id,
                        "status": "completed",
                        "source": req.source,
                    }))
                }
                Err(e) => Json(json!({
                    "error": { "message": format!("Failed to index: {}", e), "type": "indexing_error" }
                })),
            }
        }
        None => Json(json!({
            "error": "Mnemosyne not configured.",
        })),
    }
}

/// GET /v1/vector_stores/:id/files — List files in a vector store
pub async fn list_store_files(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    let registry = get_registry(&state.vector_store_db.clone());

    match registry.get(&id) {
        Some(entry) => {
            let s = entry.value();
            Json(json!({
                "object": "list",
                "data": [],
                "vector_store_id": s.id,
                "file_count": s.file_count,
            }))
        }
        None => Json(json!({
            "error": { "message": format!("No vector store with id '{}'", id), "type": "not_found" }
        })),
    }
}

// ============================================================================
// Helper
// ============================================================================

/// Get the in-memory registry, loading from SQLite on the very first call.
fn get_registry(db: &VectorStoreDb) -> &'static DashMap<String, VectorStore> {
    static REGISTRY: OnceLock<DashMap<String, VectorStore>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let map = DashMap::new();
        if let Err(e) = db.load_into_registry(&map) {
            tracing::warn!("Failed to load vector stores from SQLite: {e}");
        }
        map
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_store_creation() {
        let store = VectorStore {
            id: "vs_test123".to_string(),
            name: "Test Store".to_string(),
            description: Some("A test".to_string()),
            file_count: 0,
            status: VectorStoreStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            metadata: None,
        };
        assert_eq!(store.name, "Test Store");
        assert_eq!(store.status, VectorStoreStatus::Active);
        assert_eq!(store.file_count, 0);
    }

    #[test]
    fn test_vector_store_status_serialization() {
        let active = serde_json::to_string(&VectorStoreStatus::Active).unwrap();
        assert_eq!(active, "\"active\"");

        let indexing = serde_json::to_string(&VectorStoreStatus::Indexing).unwrap();
        assert_eq!(indexing, "\"indexing\"");
    }

    #[test]
    fn test_search_request_defaults() {
        let req: SearchRequest = serde_json::from_str(r#"{"query":"test"}"#).unwrap();
        assert_eq!(req.limit, 10);
        assert_eq!(req.mode, "hybrid");
    }

    #[test]
    fn test_search_request_custom() {
        let req: SearchRequest =
            serde_json::from_str(r#"{"query":"hello","limit":5,"mode":"fts"}"#).unwrap();
        assert_eq!(req.query, "hello");
        assert_eq!(req.limit, 5);
        assert_eq!(req.mode, "fts");
    }

    #[test]
    fn test_create_request_deserialization() {
        let req: CreateVectorStoreRequest =
            serde_json::from_str(r#"{"name":"My Store","description":"Testing"}"#).unwrap();
        assert_eq!(req.name, "My Store");
        assert_eq!(req.description, Some("Testing".to_string()));
    }

    #[test]
    fn test_add_file_request() {
        let req: AddFileRequest =
            serde_json::from_str(r#"{"content":"Hello world","source":"test.txt"}"#).unwrap();
        assert_eq!(req.content, "Hello world");
        assert_eq!(req.source, "test.txt");
    }

    #[test]
    fn test_registry_operations() {
        let registry = DashMap::new();

        let store = VectorStore {
            id: "vs_abc".to_string(),
            name: "Store A".to_string(),
            description: None,
            file_count: 0,
            status: VectorStoreStatus::Active,
            created_at: "2026-02-19T00:00:00Z".to_string(),
            metadata: None,
        };

        registry.insert(store.id.clone(), store);
        assert_eq!(registry.len(), 1);
        assert!(registry.contains_key("vs_abc"));

        // Update file count
        if let Some(mut entry) = registry.get_mut("vs_abc") {
            entry.file_count += 1;
        }
        assert_eq!(registry.get("vs_abc").unwrap().file_count, 1);

        // Delete
        let removed = registry.remove("vs_abc");
        assert!(removed.is_some());
        assert!(registry.is_empty());
    }

    #[test]
    fn test_registry_multiple_stores() {
        let registry = DashMap::new();

        for i in 0..5 {
            registry.insert(
                format!("vs_{}", i),
                VectorStore {
                    id: format!("vs_{}", i),
                    name: format!("Store {}", i),
                    description: None,
                    file_count: i,
                    status: VectorStoreStatus::Active,
                    created_at: "2026-02-19T00:00:00Z".to_string(),
                    metadata: None,
                },
            );
        }

        assert_eq!(registry.len(), 5);
        assert_eq!(registry.get("vs_3").unwrap().file_count, 3);
    }

    #[test]
    fn test_vector_store_db_in_memory() {
        let db = VectorStoreDb::in_memory().expect("in-memory DB should work");
        let store = VectorStore {
            id: "vs_mem1".to_string(),
            name: "Mem Store".to_string(),
            description: None,
            file_count: 0,
            status: VectorStoreStatus::Active,
            created_at: "2026-02-27T00:00:00Z".to_string(),
            metadata: None,
        };
        db.save(&store).expect("save should succeed");
        db.update_file_count("vs_mem1", 3)
            .expect("update should succeed");
        db.delete("vs_mem1").expect("delete should succeed");
    }
}

// ============================================================================
// VectorStoreDb — SQLite persistence
// ============================================================================

/// SQLite-backed persistence for the vector store registry.
///
/// The in-memory `DashMap` is the live state; this struct provides
/// write-through persistence so stores survive gateway restarts.
#[derive(Clone)]
pub struct VectorStoreDb {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl VectorStoreDb {
    /// Open (or create) the vector_stores SQLite database at `path`.
    pub fn new(path: &FsPath) -> Result<Self> {
        let conn = rusqlite::Connection::open(path).context("Failed to open vector_stores.db")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vector_stores (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT,
                file_count  INTEGER NOT NULL DEFAULT 0,
                status      TEXT NOT NULL DEFAULT 'active',
                created_at  TEXT NOT NULL,
                metadata    TEXT
            );",
        )
        .context("Failed to create vector_stores table")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database (for tests / fallback).
    pub fn in_memory() -> Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()
            .context("Failed to open in-memory vector_stores DB")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vector_stores (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT,
                file_count  INTEGER NOT NULL DEFAULT 0,
                status      TEXT NOT NULL DEFAULT 'active',
                created_at  TEXT NOT NULL,
                metadata    TEXT
            );",
        )
        .context("Failed to create in-memory vector_stores table")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Load all rows from SQLite into `registry` (called once at startup).
    pub fn load_into_registry(&self, registry: &DashMap<String, VectorStore>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, file_count, status, created_at, metadata
                 FROM vector_stores",
            )
            .context("Failed to prepare SELECT for vector_stores")?;
        let rows = stmt
            .query_map([], |row| {
                let status_str: String = row.get(4)?;
                let metadata_str: Option<String> = row.get(6)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, usize>(3)?,
                    status_str,
                    row.get::<_, String>(5)?,
                    metadata_str,
                ))
            })
            .context("Failed to query vector_stores")?;

        for row in rows {
            let (id, name, description, file_count, status_str, created_at, metadata_str) =
                row.context("Failed to read vector_store row")?;
            let status = match status_str.as_str() {
                "expired" => VectorStoreStatus::Expired,
                "indexing" => VectorStoreStatus::Indexing,
                _ => VectorStoreStatus::Active,
            };
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
            registry.insert(
                id.clone(),
                VectorStore {
                    id,
                    name,
                    description,
                    file_count,
                    status,
                    created_at,
                    metadata,
                },
            );
        }
        Ok(())
    }

    /// Upsert a vector store into SQLite.
    pub fn save(&self, store: &VectorStore) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let status =
            serde_json::to_string(&store.status).unwrap_or_else(|_| "\"active\"".to_string());
        let status = status.trim_matches('"');
        let metadata = store
            .metadata
            .as_ref()
            .and_then(|m| serde_json::to_string(m).ok());
        conn.execute(
            "INSERT OR REPLACE INTO vector_stores
             (id, name, description, file_count, status, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                store.id,
                store.name,
                store.description,
                store.file_count as i64,
                status,
                store.created_at,
                metadata,
            ],
        )
        .context("Failed to upsert vector store")?;
        Ok(())
    }

    /// Update the file_count for an existing store.
    pub fn update_file_count(&self, id: &str, count: usize) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE vector_stores SET file_count = ?1 WHERE id = ?2",
            params![count as i64, id],
        )
        .context("Failed to update file_count")?;
        Ok(())
    }

    /// Delete a vector store by id.
    pub fn delete(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        conn.execute("DELETE FROM vector_stores WHERE id = ?1", params![id])
            .context("Failed to delete vector store")?;
        Ok(())
    }
}
