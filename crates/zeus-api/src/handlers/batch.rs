//! Batch API and Embeddings API handlers
//!
//! - POST /v1/batches — Queue multiple chat completions for batch processing
//! - GET  /v1/batches/:id — Get batch status
//! - GET  /v1/batches/:id/results — Get completed batch results
//! - POST /v1/embeddings — Generate embeddings (OpenAI-compatible)

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info, warn};
use uuid::Uuid;
use zeus_core::Message;
use zeus_llm::LlmClient;

use crate::SharedState;

// ============================================================================
// Batch Types
// ============================================================================

/// Status of a batch job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BatchStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

impl std::fmt::Display for BatchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchStatus::Queued => write!(f, "queued"),
            BatchStatus::Processing => write!(f, "processing"),
            BatchStatus::Completed => write!(f, "completed"),
            BatchStatus::Failed => write!(f, "failed"),
        }
    }
}

/// A single request within a batch.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BatchRequestItem {
    /// Custom ID for correlation
    pub custom_id: String,
    /// Messages for this completion
    pub messages: Vec<BatchMessage>,
    /// Optional model override
    #[serde(default)]
    pub model: Option<String>,
    /// Optional max tokens
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

/// A message within a batch request item.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BatchMessage {
    pub role: String,
    pub content: String,
}

/// POST /v1/batches request body.
#[derive(Debug, Deserialize)]
pub struct CreateBatchRequest {
    /// Array of completion requests
    pub requests: Vec<BatchRequestItem>,
    /// Optional model for all requests (individual items can override)
    #[serde(default)]
    pub model: Option<String>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Result for a single batch item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResultItem {
    pub custom_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<BatchItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response content for a completed batch item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItemResponse {
    pub content: String,
    pub model: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub finish_reason: String,
}

/// Internal batch state stored in DashMap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchState {
    pub id: String,
    pub status: BatchStatus,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub model: String,
    pub metadata: Option<Value>,
    pub requests: Vec<BatchRequestItem>,
    pub results: Vec<BatchResultItem>,
}

// ============================================================================
// Embeddings Types
// ============================================================================

/// POST /v1/embeddings request body (OpenAI-compatible).
#[derive(Debug, Deserialize)]
pub struct EmbeddingsRequest {
    /// Input text(s) to embed — string or array of strings
    pub input: EmbeddingsInput,
    /// Embedding model to use
    #[serde(default)]
    pub model: Option<String>,
}

/// Input can be a single string or array of strings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EmbeddingsInput {
    Single(String),
    Multiple(Vec<String>),
}

impl EmbeddingsInput {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            EmbeddingsInput::Single(s) => vec![s],
            EmbeddingsInput::Multiple(v) => v,
        }
    }
}

// ============================================================================
// Batch Handlers
// ============================================================================

/// POST /v1/batches — Queue a batch of chat completions.
///
/// Accepts an array of completion requests. Processes them sequentially
/// in a background task and stores results for retrieval.
pub async fn create_batch(
    State(state): State<SharedState>,
    Json(req): Json<CreateBatchRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    if req.requests.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(batch_error("requests array must not be empty")),
        ));
    }

    if req.requests.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(batch_error("maximum 100 requests per batch")),
        ));
    }

    let state_read = state.read().await;
    let model_name = req
        .model
        .as_deref()
        .unwrap_or(&state_read.config.model)
        .to_string();

    let batch_id = format!("batch_{}", Uuid::new_v4().simple());
    let total = req.requests.len();
    let created_at = Utc::now().timestamp();

    info!("Batch API: creating batch {batch_id} with {total} requests");

    let batch_state = BatchState {
        id: batch_id.clone(),
        status: BatchStatus::Queued,
        created_at,
        completed_at: None,
        total,
        completed: 0,
        failed: 0,
        model: model_name.clone(),
        metadata: req.metadata.clone(),
        requests: req.requests.clone(),
        results: Vec::new(),
    };

    // Store batch state
    let key = format!("batch:{}", batch_id);
    if let Ok(val) = serde_json::to_value(&batch_state) {
        state_read.delegations.insert(key.clone(), val);
    }

    // Spawn background processing task
    let state_clone = state.clone();
    let batch_id_clone = batch_id.clone();
    let requests = req.requests;

    tokio::spawn(async move {
        process_batch(state_clone, batch_id_clone, requests, model_name).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "id": batch_id,
            "object": "batch",
            "status": "queued",
            "created_at": created_at,
            "total": total,
            "completed": 0,
            "failed": 0,
        })),
    ))
}

/// Background batch processor — runs completions sequentially and updates state.
async fn process_batch(
    state: SharedState,
    batch_id: String,
    requests: Vec<BatchRequestItem>,
    default_model: String,
) {
    let key = format!("batch:{}", batch_id);

    // Mark as processing
    {
        let st = state.read().await;
        if let Some(mut entry) = st.delegations.get_mut(&key)
            && let Some(obj) = entry.value_mut().as_object_mut()
        {
            obj.insert("status".to_string(), json!("processing"));
        }
    }

    let mut results: Vec<BatchResultItem> = Vec::with_capacity(requests.len());
    let mut completed_count: usize = 0;
    let mut failed_count: usize = 0;

    for item in &requests {
        let model = item.model.as_deref().unwrap_or(&default_model);

        // Build Zeus messages
        let zeus_messages: Vec<Message> = item
            .messages
            .iter()
            .map(|m| match m.role.as_str() {
                "system" => Message::system(&m.content),
                "assistant" => Message::assistant(&m.content),
                _ => Message::user(&m.content),
            })
            .collect();

        // Get LLM client and run completion
        let result = {
            let st = state.read().await;
            let llm = match LlmClient::from_config(&st.config) {
                Ok(l) => l,
                Err(e) => {
                    failed_count += 1;
                    results.push(BatchResultItem {
                        custom_id: item.custom_id.clone(),
                        status: "failed".to_string(),
                        response: None,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };

            let system_prompt = st.workspace.get_context().await.unwrap_or_default();
            let system_opt = if system_prompt.is_empty() {
                None
            } else {
                Some(system_prompt.as_str())
            };

            llm.complete(&zeus_messages, &[], system_opt).await
        };

        match result {
            Ok(resp) => {
                let finish_reason = match resp.stop_reason {
                    zeus_llm::StopReason::EndTurn => "stop",
                    zeus_llm::StopReason::MaxTokens => "length",
                    zeus_llm::StopReason::ToolUse => "tool_calls",
                    zeus_llm::StopReason::Error => "error",
                };

                completed_count += 1;
                results.push(BatchResultItem {
                    custom_id: item.custom_id.clone(),
                    status: "completed".to_string(),
                    response: Some(BatchItemResponse {
                        content: resp.content,
                        model: model.to_string(),
                        input_tokens: resp.input_tokens,
                        output_tokens: resp.output_tokens,
                        finish_reason: finish_reason.to_string(),
                    }),
                    error: None,
                });

                // Record economy spend
                let st = state.read().await;
                let cost = super::model_tier_cost(model);
                if let Err(e) = st.ledger.spend(
                    "default",
                    cost,
                    zeus_economy::TransactionReason::LlmCall,
                    format!("batch: {}", model),
                ) {
                    debug!("Economy spend failed (non-fatal): {e}");
                }
            }
            Err(e) => {
                warn!("Batch item '{}' failed: {e}", item.custom_id);
                failed_count += 1;
                results.push(BatchResultItem {
                    custom_id: item.custom_id.clone(),
                    status: "failed".to_string(),
                    response: None,
                    error: Some(e.to_string()),
                });
            }
        }

        // Update progress in store
        {
            let st = state.read().await;
            if let Some(mut entry) = st.delegations.get_mut(&key)
                && let Some(obj) = entry.value_mut().as_object_mut()
            {
                obj.insert("completed".to_string(), json!(completed_count));
                obj.insert("failed".to_string(), json!(failed_count));
            }
        }
    }

    // Finalize batch
    let final_status = if failed_count == requests.len() {
        BatchStatus::Failed
    } else {
        BatchStatus::Completed
    };

    let completed_at = Utc::now().timestamp();

    let st = state.read().await;
    if let Some(mut entry) = st.delegations.get_mut(&key)
        && let Some(obj) = entry.value_mut().as_object_mut()
    {
        obj.insert("status".to_string(), json!(final_status.to_string()));
        obj.insert("completed_at".to_string(), json!(completed_at));
        obj.insert("completed".to_string(), json!(completed_count));
        obj.insert("failed".to_string(), json!(failed_count));
        if let Ok(results_val) = serde_json::to_value(&results) {
            obj.insert("results".to_string(), results_val);
        }
    }

    info!("Batch {batch_id} finished: {completed_count} completed, {failed_count} failed");
}

/// GET /v1/batches/:id — Get batch status.
pub async fn get_batch(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    info!("Batch API: get {id}");

    let state_read = state.read().await;
    let key = format!("batch:{}", id);

    if let Some(entry) = state_read.delegations.get(&key) {
        let val = entry.value().clone();
        // Return without results array for status endpoint
        let mut resp = val.clone();
        if let Some(obj) = resp.as_object_mut() {
            obj.remove("results");
            obj.remove("requests");
        }
        return Ok(Json(resp));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(batch_error(&format!("Batch '{}' not found", id))),
    ))
}

/// GET /v1/batches/:id/results — Get completed batch results.
pub async fn get_batch_results(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    info!("Batch API: get results for {id}");

    let state_read = state.read().await;
    let key = format!("batch:{}", id);

    if let Some(entry) = state_read.delegations.get(&key) {
        let val = entry.value();
        let status = val
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if status != "completed" && status != "failed" {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": {
                        "message": format!("Batch is still {status}. Results not yet available."),
                        "type": "batch_not_ready",
                    }
                })),
            ));
        }

        let results = val.get("results").cloned().unwrap_or(json!([]));
        return Ok(Json(json!({
            "id": id,
            "status": status,
            "results": results,
        })));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(batch_error(&format!("Batch '{}' not found", id))),
    ))
}

// ============================================================================
// Embeddings Handler
// ============================================================================

/// POST /v1/embeddings — Generate embeddings (OpenAI-compatible).
///
/// Accepts a single string or array of strings, returns embedding vectors.
pub async fn create_embeddings(
    State(state): State<SharedState>,
    Json(req): Json<EmbeddingsRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let input = req.input.into_vec();

    if input.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(embeddings_error("input must not be empty")),
        ));
    }

    if input.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(embeddings_error("maximum 256 input strings per request")),
        ));
    }

    let state_read = state.read().await;

    let llm = LlmClient::from_config(&state_read.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(embeddings_error(&e.to_string())),
        )
    })?;

    let model = req.model.as_deref();

    info!(
        "Embeddings API: {} input(s), model={:?}",
        input.len(),
        model
    );

    let result = llm.embed(&input, model).await.map_err(|e| {
        warn!("Embeddings error: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(embeddings_error(&e.to_string())),
        )
    })?;

    // Record economy spend
    let cost = 1u64; // Embeddings are cheap
    if let Err(e) = state_read.ledger.spend(
        "default",
        cost,
        zeus_economy::TransactionReason::LlmCall,
        format!("embeddings: {}", result.model),
    ) {
        debug!("Economy spend failed (non-fatal): {e}");
    }

    // Build OpenAI-compatible response
    let data: Vec<Value> = result
        .data
        .iter()
        .map(|e| {
            json!({
                "object": "embedding",
                "index": e.index,
                "embedding": e.embedding,
            })
        })
        .collect();

    Ok(Json(json!({
        "object": "list",
        "data": data,
        "model": result.model,
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "total_tokens": result.usage.total_tokens,
        }
    })))
}

// ============================================================================
// Helpers
// ============================================================================

fn batch_error(message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": "invalid_request_error",
            "code": null,
        }
    })
}

fn embeddings_error(message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": "invalid_request_error",
            "code": null,
        }
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_test_router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn test_app() -> axum::Router {
        let config = zeus_core::Config::default();
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::AppState::new(config).unwrap(),
        ));
        create_test_router(state)
    }

    // ── Batch tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_batch_empty_requests() {
        let app = test_app().await;

        let body = json!({ "requests": [] });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/batches")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_batch_invalid_body() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/batches")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_get_batch_not_found() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/batches/batch_nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_batch_results_not_found() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/batches/batch_nonexistent/results")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_batch_valid_request() {
        let app = test_app().await;

        let body = json!({
            "requests": [
                {
                    "custom_id": "req-1",
                    "messages": [
                        {"role": "user", "content": "Hello"}
                    ]
                }
            ]
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/batches")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should accept (202) even if LLM call will fail (no provider configured)
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["status"], "queued");
        assert_eq!(json["total"], 1);
        assert!(json["id"].as_str().unwrap().starts_with("batch_"));
    }

    // ── Embeddings tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_embeddings_empty_input() {
        let app = test_app().await;

        let body = json!({ "input": [] });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_embeddings_invalid_body() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // ── Serialization tests ──────────────────────────────────────────────

    #[test]
    fn test_batch_status_display() {
        assert_eq!(BatchStatus::Queued.to_string(), "queued");
        assert_eq!(BatchStatus::Processing.to_string(), "processing");
        assert_eq!(BatchStatus::Completed.to_string(), "completed");
        assert_eq!(BatchStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_batch_status_serialization() {
        let s = BatchStatus::Completed;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"completed\"");

        let deserialized: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, BatchStatus::Completed);
    }

    #[test]
    fn test_batch_request_deserialization() {
        let json = r#"{
            "requests": [
                {
                    "custom_id": "req-1",
                    "messages": [
                        {"role": "system", "content": "You are helpful."},
                        {"role": "user", "content": "Hello"}
                    ],
                    "model": "gpt-4o"
                },
                {
                    "custom_id": "req-2",
                    "messages": [
                        {"role": "user", "content": "World"}
                    ]
                }
            ],
            "model": "claude-sonnet-4-20250514"
        }"#;

        let req: CreateBatchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.requests.len(), 2);
        assert_eq!(req.requests[0].custom_id, "req-1");
        assert_eq!(req.requests[0].messages.len(), 2);
        assert_eq!(req.requests[0].model.as_deref(), Some("gpt-4o"));
        assert_eq!(req.requests[1].custom_id, "req-2");
        assert!(req.requests[1].model.is_none());
        assert_eq!(req.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_embeddings_input_single() {
        let json = r#"{"input": "hello world"}"#;
        let req: EmbeddingsRequest = serde_json::from_str(json).unwrap();
        let vec = req.input.into_vec();
        assert_eq!(vec, vec!["hello world"]);
    }

    #[test]
    fn test_embeddings_input_multiple() {
        let json = r#"{"input": ["hello", "world"]}"#;
        let req: EmbeddingsRequest = serde_json::from_str(json).unwrap();
        let vec = req.input.into_vec();
        assert_eq!(vec, vec!["hello", "world"]);
    }

    #[test]
    fn test_batch_result_item_serialization() {
        let item = BatchResultItem {
            custom_id: "req-1".to_string(),
            status: "completed".to_string(),
            response: Some(BatchItemResponse {
                content: "Hello!".to_string(),
                model: "gpt-4o".to_string(),
                input_tokens: 5,
                output_tokens: 3,
                finish_reason: "stop".to_string(),
            }),
            error: None,
        };

        let val = serde_json::to_value(&item).unwrap();
        assert_eq!(val["custom_id"], "req-1");
        assert_eq!(val["response"]["content"], "Hello!");
        assert!(val.get("error").is_none()); // skip_serializing_if omits None
    }

    #[test]
    fn test_batch_state_serialization() {
        let state = BatchState {
            id: "batch_test".to_string(),
            status: BatchStatus::Queued,
            created_at: 1700000000,
            completed_at: None,
            total: 2,
            completed: 0,
            failed: 0,
            model: "gpt-4o".to_string(),
            metadata: None,
            requests: vec![],
            results: vec![],
        };

        let val = serde_json::to_value(&state).unwrap();
        assert_eq!(val["id"], "batch_test");
        assert_eq!(val["status"], "queued");
        assert_eq!(val["total"], 2);
    }
}
