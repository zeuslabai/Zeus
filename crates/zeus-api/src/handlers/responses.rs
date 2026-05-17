//! OpenAI Responses API compatibility layer
//!
//! Implements the OpenAI Responses API format:
//! - POST /v1/responses — Create a response
//! - GET  /v1/responses/:id — Retrieve a previous response
//! - DELETE /v1/responses/:id — Cancel/delete a response
//!
//! This gives Zeus drop-in compatibility with OpenAI Responses API clients.
//! Internally maps to our agent loop / LLM pipeline.

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
use zeus_session::Session;

use crate::SharedState;

// ============================================================================
// Request / Response Types
// ============================================================================

/// Input item in the Responses API — can be a simple string or a structured message.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InputItem {
    /// Simple text string (treated as user message)
    Text(String),
    /// Structured message with role and content
    Message(InputMessage),
}

/// A structured input message with role + content.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InputMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<InputContent>,
}

/// Content can be a plain string or an array of content parts.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InputContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// A content part (text or image reference).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default)]
    pub text: Option<String>,
    // Image URLs etc. can be added later
}

/// Tool definition in Responses API format.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<Value>,
    // For built-in tools like web_search, code_interpreter
}

/// POST /v1/responses request body.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CreateResponseRequest {
    /// The model to use (e.g. "gpt-4o", "claude-sonnet-4-20250514")
    #[serde(default)]
    pub model: Option<String>,

    /// Input messages — array of strings or message objects
    pub input: Vec<InputItem>,

    /// System instructions (equivalent to a system message)
    #[serde(default)]
    pub instructions: Option<String>,

    /// Tool definitions
    #[serde(default)]
    pub tools: Option<Vec<ResponseTool>>,

    /// Previous response ID for multi-turn conversations
    #[serde(default)]
    pub previous_response_id: Option<String>,

    /// Whether to store the response (default: true)
    #[serde(default = "default_true")]
    pub store: bool,

    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<Value>,

    /// Temperature
    #[serde(default)]
    pub temperature: Option<f64>,

    /// Max output tokens
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
}

fn default_true() -> bool {
    true
}

/// Output item in the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputItem {
    #[serde(rename = "type")]
    pub item_type: String,
    /// Unique ID for this output item
    pub id: String,
    /// Status: "completed", "in_progress", "failed"
    pub status: String,
    /// Role (for message outputs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Content array (for message outputs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<OutputContent>>,
}

/// Content within an output item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Token usage information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub total_tokens: usize,
}

/// Stored response object (persisted in-memory for retrieval).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredResponse {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<OutputItem>,
    pub usage: ResponseUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Internal: the session ID used for multi-turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /v1/responses — Create a new response (OpenAI Responses API format).
///
/// Accepts an input array with messages, optional instructions (system prompt),
/// model selection, tools, and previous_response_id for multi-turn conversations.
/// Maps internally to our LLM pipeline.
pub async fn create_response(
    State(state): State<SharedState>,
    Json(req): Json<CreateResponseRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    info!(
        "Responses API: create, {} input items, model={:?}, prev={:?}",
        req.input.len(),
        req.model,
        req.previous_response_id
    );

    let state_read = state.read().await;

    // Build Zeus messages from input items
    let mut zeus_messages: Vec<Message> = Vec::new();

    // If previous_response_id is provided, load the prior session context
    let mut session_id: Option<String> = None;
    if let Some(ref prev_id) = req.previous_response_id {
        // Look up stored response to find its session
        if let Some(prev_resp) = state_read.workflow_states.get(&format!("resp:{}", prev_id))
            && let Ok(stored) = serde_json::from_value::<StoredResponse>(
                serde_json::to_value(&*prev_resp).unwrap_or_default(),
            )
        {
            session_id = stored.session_id;
        }

        // Try loading session messages for context continuity
        if let Some(ref sid) = session_id
            && let Ok(prev_session) = Session::load(&state_read.config.sessions, sid).await
        {
            zeus_messages.extend(prev_session.messages.clone());
        }
    }

    // Add instructions as system message if provided
    if let Some(ref instructions) = req.instructions {
        // Insert system message at beginning (after any loaded history)
        if zeus_messages.is_empty() || zeus_messages[0].role != zeus_core::Role::System {
            zeus_messages.insert(0, Message::system(instructions));
        }
    }

    // Convert input items to Zeus messages
    for item in &req.input {
        match item {
            InputItem::Text(text) => {
                zeus_messages.push(Message::user(text));
            }
            InputItem::Message(msg) => {
                let content_str = match &msg.content {
                    Some(InputContent::Text(t)) => t.clone(),
                    Some(InputContent::Parts(parts)) => {
                        // Concatenate text parts
                        parts
                            .iter()
                            .filter_map(|p| p.text.as_deref())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                    None => String::new(),
                };

                let m = match msg.role.as_str() {
                    "system" | "developer" => Message::system(&content_str),
                    "assistant" => Message::assistant(&content_str),
                    _ => Message::user(&content_str),
                };
                zeus_messages.push(m);
            }
        }
    }

    if zeus_messages.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(responses_error(
                "input array must contain at least one message",
            )),
        ));
    }

    // Create LLM client
    let llm = LlmClient::from_config(&state_read.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(responses_error(&e.to_string())),
        )
    })?;

    // Get workspace context as fallback system prompt
    let system_prompt = if req.instructions.is_some() {
        // Already added as a message, use empty to avoid duplication
        String::new()
    } else {
        state_read.workspace.get_context().await.unwrap_or_default()
    };

    let system_prompt_opt = if system_prompt.is_empty() {
        None
    } else {
        Some(system_prompt.as_str())
    };

    // Convert tool definitions to Zeus tool schemas (if any)
    let tool_schemas = if req.tools.is_some() {
        // Use registered tool schemas — the Responses API tools map to our tool registry
        state_read.tools.schemas()
    } else {
        vec![]
    };

    // Call LLM
    let response = llm
        .complete(&zeus_messages, &tool_schemas, system_prompt_opt)
        .await
        .map_err(|e| {
            warn!("Responses API LLM error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(responses_error(&e.to_string())),
            )
        })?;

    // Record economy spend
    let model_name = req.model.as_deref().unwrap_or(&state_read.config.model);
    let cost = super::model_tier_cost(model_name);
    if let Err(e) = state_read.ledger.spend(
        "default",
        cost,
        zeus_economy::TransactionReason::LlmCall,
        format!("responses_api: {}", model_name),
    ) {
        debug!("Economy spend failed (non-fatal): {e}");
    }

    // Build response ID
    let response_id = format!("resp_{}", Uuid::new_v4().simple());
    let created_at = Utc::now().timestamp();

    // Build output items
    let mut output_items = Vec::new();

    // Add message output
    let msg_item_id = format!("msg_{}", Uuid::new_v4().simple());
    output_items.push(OutputItem {
        item_type: "message".to_string(),
        id: msg_item_id,
        status: "completed".to_string(),
        role: Some("assistant".to_string()),
        content: Some(vec![OutputContent {
            content_type: "output_text".to_string(),
            text: Some(response.content.clone()),
        }]),
    });

    // If there were tool calls, add them as output items
    for tc in &response.tool_calls {
        let tc_item_id = format!("tc_{}", Uuid::new_v4().simple());
        output_items.push(OutputItem {
            item_type: "function_call".to_string(),
            id: tc_item_id,
            status: "completed".to_string(),
            role: None,
            content: Some(vec![OutputContent {
                content_type: "function_call".to_string(),
                text: Some(
                    json!({
                        "name": tc.name,
                        "arguments": tc.arguments.to_string(),
                    })
                    .to_string(),
                ),
            }]),
        });
    }

    let usage = ResponseUsage {
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        total_tokens: response.input_tokens + response.output_tokens,
    };

    // Determine status based on stop reason
    let status = match response.stop_reason {
        zeus_llm::StopReason::EndTurn => "completed",
        zeus_llm::StopReason::MaxTokens => "incomplete",
        zeus_llm::StopReason::ToolUse => "completed",
        zeus_llm::StopReason::Error => "failed",
    };

    // Persist session for multi-turn if store=true
    let final_session_id = if req.store {
        let mut session = if let Some(ref sid) = session_id {
            Session::load(&state_read.config.sessions, sid)
                .await
                .unwrap_or_else(|_| Session::new(&state_read.config.sessions))
        } else {
            let s = Session::new(&state_read.config.sessions);
            if let Err(e) = s.init().await {
                warn!("Failed to init session for responses API: {e}");
            }
            s
        };

        // Add the new messages to session
        for item in &req.input {
            match item {
                InputItem::Text(text) => {
                    let _ = session.add(Message::user(text)).await;
                }
                InputItem::Message(msg) => {
                    let content_str = match &msg.content {
                        Some(InputContent::Text(t)) => t.clone(),
                        Some(InputContent::Parts(parts)) => parts
                            .iter()
                            .filter_map(|p| p.text.as_deref())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        None => String::new(),
                    };
                    let m = match msg.role.as_str() {
                        "system" | "developer" => Message::system(&content_str),
                        "assistant" => Message::assistant(&content_str),
                        _ => Message::user(&content_str),
                    };
                    let _ = session.add(m).await;
                }
            }
        }

        // Add assistant response
        let assistant_msg =
            Message::assistant(&response.content).with_tool_calls(response.tool_calls);
        let _ = session.add(assistant_msg).await;

        Some(session.id.clone())
    } else {
        None
    };

    // Build stored response
    let stored = StoredResponse {
        id: response_id.clone(),
        object: "response".to_string(),
        created_at,
        status: status.to_string(),
        model: model_name.to_string(),
        output: output_items.clone(),
        usage: usage.clone(),
        instructions: req.instructions.clone(),
        metadata: req.metadata.clone(),
        previous_response_id: req.previous_response_id.clone(),
        session_id: final_session_id.clone(),
    };

    // Store response for retrieval via GET /v1/responses/:id
    if req.store
        && let Ok(val) = serde_json::to_value(&stored)
    {
        state_read.workflow_states.insert(
            format!("resp:{}", response_id),
            crate::WorkflowState {
                workflow_id: response_id.clone(),
                status: status.to_string(),
                message: format!("OpenAI Responses API — {}", model_name),
                nodes: vec![],
                progress_percentage: 100.0,
                created_at: Utc::now().to_rfc3339(),
                started_at: Some(Utc::now().to_rfc3339()),
                completed_at: Some(Utc::now().to_rfc3339()),
                total_nodes: 0,
                completed_nodes: 0,
                failed_nodes: 0,
            },
        );
        // Also store the full response data under a separate key
        state_read
            .delegations
            .insert(format!("resp:{}", response_id), val);
    }

    // Build OpenAI Responses API response format
    let resp_body = json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": model_name,
        "output": output_items,
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        },
        "instructions": req.instructions,
        "metadata": req.metadata,
        "previous_response_id": req.previous_response_id,
    });

    Ok((StatusCode::OK, Json(resp_body)))
}

/// GET /v1/responses/:id — Retrieve a stored response.
pub async fn get_response(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    info!("Responses API: get {id}");

    let state_read = state.read().await;
    let key = format!("resp:{}", id);

    // Look up in delegations store (full response data)
    if let Some(entry) = state_read.delegations.get(&key) {
        return Ok(Json(entry.value().clone()));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(responses_error(&format!("Response '{}' not found", id))),
    ))
}

/// DELETE /v1/responses/:id — Delete/cancel a stored response.
pub async fn delete_response(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    info!("Responses API: delete {id}");

    let state_read = state.read().await;
    let key = format!("resp:{}", id);

    // Remove from both stores
    let removed = state_read.delegations.remove(&key).is_some();
    state_read.workflow_states.remove(&key);

    if removed {
        Ok(Json(json!({
            "id": id,
            "object": "response",
            "deleted": true,
        })))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(responses_error(&format!("Response '{}' not found", id))),
        ))
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Format an error in OpenAI Responses API format.
fn responses_error(message: &str) -> Value {
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

    /// Helper: build a test app state + router
    async fn test_app() -> axum::Router {
        let config = zeus_core::Config::default();
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::AppState::new(config).unwrap(),
        ));
        create_test_router(state)
    }

    #[tokio::test]
    async fn test_create_response_missing_input() {
        let app = test_app().await;

        let body = serde_json::json!({
            "input": []
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_response_invalid_body() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required `input` field
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_get_response_not_found() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/responses/resp_nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_response_not_found() {
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/responses/resp_nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_input_item_deserialization() {
        // Test string input
        let json_str = r#""Hello, world!""#;
        let item: InputItem = serde_json::from_str(json_str).unwrap();
        match item {
            InputItem::Text(t) => assert_eq!(t, "Hello, world!"),
            _ => panic!("Expected Text variant"),
        }

        // Test structured message input
        let json_msg = r#"{"role": "user", "content": "Hello"}"#;
        let item: InputMessage = serde_json::from_str(json_msg).unwrap();
        assert_eq!(item.role, "user");
        match item.content {
            Some(InputContent::Text(t)) => assert_eq!(t, "Hello"),
            _ => panic!("Expected Text content"),
        }

        // Test message with content parts
        let json_parts = r#"{
            "role": "user",
            "content": [
                {"type": "input_text", "text": "What is in this image?"}
            ]
        }"#;
        let item: InputMessage = serde_json::from_str(json_parts).unwrap();
        assert_eq!(item.role, "user");
        match item.content {
            Some(InputContent::Parts(parts)) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].text.as_deref(), Some("What is in this image?"));
            }
            _ => panic!("Expected Parts content"),
        }
    }

    #[tokio::test]
    async fn test_create_response_request_deserialization() {
        let json_req = r#"{
            "model": "claude-sonnet-4-20250514",
            "input": [
                "What is the capital of France?"
            ],
            "instructions": "You are a helpful assistant.",
            "store": true
        }"#;

        let req: CreateResponseRequest = serde_json::from_str(json_req).unwrap();
        assert_eq!(req.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(req.input.len(), 1);
        assert_eq!(
            req.instructions.as_deref(),
            Some("You are a helpful assistant.")
        );
        assert!(req.store);
        assert!(req.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn test_multi_turn_request_deserialization() {
        let json_req = r#"{
            "model": "gpt-4o",
            "input": [
                {"role": "user", "content": "Tell me more about that."}
            ],
            "previous_response_id": "resp_abc123"
        }"#;

        let req: CreateResponseRequest = serde_json::from_str(json_req).unwrap();
        assert_eq!(req.previous_response_id.as_deref(), Some("resp_abc123"));
        assert_eq!(req.input.len(), 1);
    }

    #[tokio::test]
    async fn test_stored_response_serialization() {
        let resp = StoredResponse {
            id: "resp_test123".to_string(),
            object: "response".to_string(),
            created_at: 1700000000,
            status: "completed".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            output: vec![OutputItem {
                item_type: "message".to_string(),
                id: "msg_test456".to_string(),
                status: "completed".to_string(),
                role: Some("assistant".to_string()),
                content: Some(vec![OutputContent {
                    content_type: "output_text".to_string(),
                    text: Some("Paris is the capital of France.".to_string()),
                }]),
            }],
            usage: ResponseUsage {
                input_tokens: 10,
                output_tokens: 8,
                total_tokens: 18,
            },
            instructions: Some("Be helpful".to_string()),
            metadata: None,
            previous_response_id: None,
            session_id: Some("sess_test".to_string()),
        };

        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["id"], "resp_test123");
        assert_eq!(val["object"], "response");
        assert_eq!(val["status"], "completed");
        assert_eq!(val["output"][0]["role"], "assistant");
        assert_eq!(
            val["output"][0]["content"][0]["text"],
            "Paris is the capital of France."
        );
        assert_eq!(val["usage"]["total_tokens"], 18);
    }

    #[tokio::test]
    async fn test_developer_role_as_system() {
        // The "developer" role in OpenAI Responses API maps to system
        let json_req = r#"{
            "input": [
                {"role": "developer", "content": "You are a coding assistant."},
                {"role": "user", "content": "Write hello world in Python."}
            ]
        }"#;

        let req: CreateResponseRequest = serde_json::from_str(json_req).unwrap();
        assert_eq!(req.input.len(), 2);

        // Verify the first item is a Message with developer role
        match &req.input[0] {
            InputItem::Message(msg) => assert_eq!(msg.role, "developer"),
            _ => panic!("Expected Message variant"),
        }
    }
}
