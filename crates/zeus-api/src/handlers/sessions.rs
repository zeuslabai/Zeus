//! Session-related API handlers.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;
use zeus_core::Role;
use zeus_session::{SearchQuery, Session, SessionSearcher};

use crate::SharedState;

use futures::future::join_all;

// ============================================================================
// Query Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SessionListQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

// ============================================================================
// Sessions
// ============================================================================

pub async fn list_sessions(
    State(state): State<SharedState>,
    Query(params): Query<SessionListQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let all_sessions = Session::list(&state.config.sessions)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = all_sessions.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(zeus_core::MAX_PAGE_LIMIT);

    let page_ids: Vec<String> = all_sessions
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(id, _)| id)
        .collect();

    // Load metadata concurrently — avoids N+1 sequential Session::load()
    let sessions_dir = state.config.sessions.clone();
    let futs: Vec<_> = page_ids
        .iter()
        .map(|id| {
            let d = sessions_dir.clone();
            let id = id.clone();
            async move { Session::quick_metadata(&d, &id).await }
        })
        .collect();

    let results = join_all(futs).await;
    let list: Vec<_> = results
        .into_iter()
        .filter_map(|r| r.ok())
        .map(|m| {
            json!({
                "id": m.id,
                "created": m.created.to_rfc3339(),
                "message_count": m.message_count,
                "est_tokens": m.est_tokens,
                "last_preview": m.last_preview,
            })
        })
        .collect();

    Ok(Json(json!({ "sessions": list, "total": total })))
}

pub async fn create_session(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::new(&state.config.sessions);
    session
        .init()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Run session maintenance after create
    let maintenance_config = state.config.session_maintenance.clone().unwrap_or_default();
    let maintenance = crate::session_maintenance::SessionMaintenance::new(
        maintenance_config,
        &state.config.sessions,
    );
    let maint_result = maintenance.run();
    if !maint_result.errors.is_empty() {
        debug!("Session maintenance errors: {:?}", maint_result.errors);
    }

    Ok(Json(json!({
        "id": session.id,
        "created": session.created.to_rfc3339()
    })))
}

/// POST /v1/sessions/:id/clear — Clear all messages from a session (keeps the file)
pub async fn clear_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let path = std::path::Path::new(&state.config.sessions).join(format!("{}.jsonl", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Session not found: {}", id)));
    }

    // Truncate the file to clear all messages
    tokio::fs::write(&path, b"")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": "Session cleared"
    })))
}

/// POST /v1/sessions/:id/compact — Force compaction of session history
pub async fn compact_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let mut session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let before_count = session.messages.len();

    // Phase 1: Strip tool result details from older messages (cheap, high savings)
    let preserve = 5; // keep last 5 messages verbatim
    let strip_before = before_count.saturating_sub(preserve);
    for msg in session.messages.iter_mut().take(strip_before) {
        if msg.role == Role::Tool {
            // Truncate long tool outputs to first 200 chars (UTF-8 safe)
            if msg.content.len() > 200 {
                msg.content = format!(
                    "{}... [compacted]",
                    zeus_core::truncate_str(&msg.content, 200)
                );
            }
        }
    }

    // Save compacted session
    let path = std::path::Path::new(&state.config.sessions).join(format!("{}.jsonl", id));
    let mut lines = Vec::new();
    for msg in &session.messages {
        if let Ok(json) = serde_json::to_string(msg) {
            lines.push(json);
        }
    }
    tokio::fs::write(&path, lines.join("\n"))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let after_count = session.messages.len();

    Ok(Json(json!({
        "success": true,
        "id": id,
        "before_messages": before_count,
        "after_messages": after_count,
        "message": "Session compacted"
    })))
}

/// DELETE /v1/sessions/:id — Delete session
pub async fn delete_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let path = std::path::Path::new(&state.config.sessions).join(format!("{}.jsonl", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Session not found: {}", id)));
    }

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": "Session deleted"
    })))
}

pub async fn get_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let messages: Vec<Value> = session
        .messages
        .iter()
        .map(|m| {
            let mut msg = json!({
                "role": format!("{:?}", m.role),
                "content": zeus_core::sanitize::redact_secrets(&m.content),
                "timestamp": m.timestamp.to_rfc3339()
            });
            if let Some(ref cs) = m.channel_source {
                msg["channel_source"] = json!({
                    "channel_type": cs.channel_type,
                    "channel_id": cs.channel_id,
                    "channel_name": cs.channel_name,
                    "sender_name": cs.sender_name,
                    "sender_id": cs.sender_id,
                });
            }
            msg
        })
        .collect();

    Ok(Json(json!({
        "id": session.id,
        "created": session.created.to_rfc3339(),
        "messages": messages
    })))
}

// ============================================================================
// Session Search
// ============================================================================

/// POST /v1/sessions/search — Full-text search across sessions
pub async fn search_sessions(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let query: SearchQuery = serde_json::from_value(body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid search query: {}", e),
        )
    })?;

    let searcher = SessionSearcher::new(&state.config.sessions);

    let results = searcher
        .search(&query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total_matches = results.len();
    let results_json: Vec<Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "session_id": r.session_id,
                "message_index": r.message_index,
                "role": r.role,
                "content_snippet": r.content_snippet,
                "match_offset": r.match_offset,
                "timestamp": r.timestamp.map(|ts| ts.to_rfc3339())
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "results": results_json,
        "total_matches": total_matches
    })))
}

// ============================================================================
// Session Stats
// ============================================================================

pub async fn get_session_stats(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let message_count = session.messages.len();
    let user_messages = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .count();
    let assistant_messages = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    let tool_call_count: usize = session.messages.iter().map(|m| m.tool_calls.len()).sum();

    let created = session.created;
    let last_activity = session
        .messages
        .last()
        .map(|m| m.timestamp)
        .unwrap_or(created);
    let duration_seconds = (last_activity - created).num_seconds().max(0);
    let duration_ms = (last_activity - created).num_milliseconds().max(0);

    // Collect unique tool names used
    let mut tools_used: Vec<String> = session
        .messages
        .iter()
        .flat_map(|m| m.tool_calls.iter().map(|tc| tc.name.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    tools_used.sort();

    // Estimate total tokens (chars / 4 heuristic)
    let total_tokens: usize = session.messages.iter().map(estimate_message_tokens).sum();

    // Cost estimate based on model — per-provider pricing
    let (input_cost, output_cost) = cost_per_million_tokens(&state.config.model);
    let input_tokens = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(estimate_message_tokens)
        .sum::<usize>();
    let output_tokens = total_tokens.saturating_sub(input_tokens);
    let cost_estimate = (input_tokens as f64 / 1_000_000.0) * input_cost
        + (output_tokens as f64 / 1_000_000.0) * output_cost;

    Ok(Json(json!({
        "id": session.id,
        "total_turns": message_count,
        "message_count": message_count,
        "user_messages": user_messages,
        "assistant_messages": assistant_messages,
        "tool_calls": tool_call_count,
        "tools_used": tools_used,
        "total_tokens": total_tokens,
        "model_used": state.config.model,
        "cost_estimate": format!("{:.4}", cost_estimate),
        "created": created.to_rfc3339(),
        "last_activity": last_activity.to_rfc3339(),
        "duration_seconds": duration_seconds,
        "duration_ms": duration_ms
    })))
}

// ============================================================================
// Phase 3: Session Detail Endpoints
// ============================================================================

/// GET /v1/sessions/:id/raw — Raw JSONL session data
pub async fn get_session_raw(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Verify session exists
    let _session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Read raw JSONL file and redact secrets before serving
    let jsonl_path = state.config.sessions.join(format!("{}.jsonl", id));
    let raw_content = tokio::fs::read_to_string(&jsonl_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let content = zeus_core::sanitize::redact_secrets(&raw_content);
    let size_bytes = content.len();

    Ok(Json(json!({
        "id": id,
        "format": "jsonl",
        "content": content,
        "size_bytes": size_bytes
    })))
}

/// GET /v1/sessions/:id/audit — Audit trail
pub async fn get_session_audit(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let mut events = Vec::new();

    for msg in &session.messages {
        // Extract tool calls from assistant messages
        if msg.role == Role::Assistant {
            for tc in &msg.tool_calls {
                let args_str = match &tc.arguments {
                    Value::Object(map) => {
                        // Summarize args for audit display
                        map.iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                    other => other.to_string(),
                };

                let event_type = if tc.name == "write_file" || tc.name == "edit_file" {
                    "memory_write"
                } else {
                    "tool_call"
                };

                events.push(json!({
                    "type": event_type,
                    "tool": tc.name,
                    "args": zeus_core::sanitize::redact_secrets(&args_str),
                    "timestamp": msg.timestamp.to_rfc3339(),
                    "approved": true
                }));
            }
        }
    }

    Ok(Json(json!({
        "id": id,
        "events": events
    })))
}

/// GET /v1/sessions/:id/tools — Tool call chain
pub async fn get_session_tools(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let mut tool_calls = Vec::new();
    let mut index = 0usize;

    // Build a map of tool call id -> result
    let mut result_map: std::collections::HashMap<String, &zeus_core::ToolResult> =
        std::collections::HashMap::new();
    for msg in &session.messages {
        for tr in &msg.tool_results {
            result_map.insert(tr.call_id.clone(), tr);
        }
    }

    for msg in &session.messages {
        if msg.role == Role::Assistant {
            for tc in &msg.tool_calls {
                let (success, output_preview) = if let Some(tr) = result_map.get(&tc.id) {
                    let preview: String = tr.output.chars().take(200).collect();
                    (tr.success, zeus_core::sanitize::redact_secrets(&preview))
                } else {
                    (true, String::new())
                };

                let redacted_args = {
                    let args_str = tc.arguments.to_string();
                    let redacted_str = zeus_core::sanitize::redact_secrets(&args_str);
                    serde_json::from_str::<Value>(&redacted_str)
                        .unwrap_or_else(|_| Value::String(redacted_str))
                };

                tool_calls.push(json!({
                    "index": index,
                    "name": tc.name,
                    "args": redacted_args,
                    "success": success,
                    "output_preview": output_preview
                }));
                index += 1;
            }
        }
    }

    Ok(Json(json!({
        "id": id,
        "tool_calls": tool_calls
    })))
}

// ============================================================================
// Session Replay
// ============================================================================

/// Estimate token count for a message (chars / 4 heuristic)
fn estimate_message_tokens(msg: &zeus_core::Message) -> usize {
    let mut chars = msg.content.len();
    for tc in &msg.tool_calls {
        chars += tc.name.len();
        chars += tc.arguments.to_string().len();
    }
    for tr in &msg.tool_results {
        chars += tr.output.len();
    }
    chars / 4
}

/// Per-provider pricing: returns (input_cost, output_cost) per million tokens.
/// Model string format: "provider/model-name" (e.g. "anthropic/claude-sonnet-4-20250514").
fn cost_per_million_tokens(model_string: &str) -> (f64, f64) {
    let lower = model_string.to_lowercase();
    let provider = lower.split('/').next().unwrap_or("");
    let model = lower.split('/').nth(1).unwrap_or(&lower);

    match provider {
        "ollama" => (0.0, 0.0),
        "anthropic" => {
            if model.contains("opus") {
                (15.0, 75.0)
            } else if model.contains("haiku") {
                (0.25, 1.25)
            } else {
                // Sonnet-class default
                (3.0, 15.0)
            }
        }
        "openai" => {
            if model.contains("gpt-4o-mini") {
                (0.15, 0.60)
            } else if model.contains("gpt-4o") {
                (2.50, 10.0)
            } else if model.contains("gpt-4-turbo") {
                (10.0, 30.0)
            } else if model.contains("gpt-4") {
                (30.0, 60.0)
            } else if model.contains("o1") {
                (15.0, 60.0)
            } else {
                (2.50, 10.0)
            }
        }
        "openrouter" => {
            if lower.contains("claude") {
                if lower.contains("opus") {
                    (15.0, 75.0)
                } else if lower.contains("haiku") {
                    (0.25, 1.25)
                } else {
                    (3.0, 15.0)
                }
            } else if lower.contains("gpt-4o") {
                (2.50, 10.0)
            } else {
                (3.0, 15.0)
            }
        }
        "google" => {
            if model.contains("flash") {
                (0.075, 0.30)
            } else if model.contains("pro") {
                (1.25, 5.0)
            } else {
                (0.075, 0.30)
            }
        }
        "groq" => (0.05, 0.10),
        "mistral" => {
            if model.contains("large") {
                (2.0, 6.0)
            } else if model.contains("small") {
                (0.2, 0.6)
            } else {
                (2.0, 6.0)
            }
        }
        "together" => (0.90, 0.90),
        "fireworks" => (0.90, 0.90),
        "azure" => (2.50, 10.0),
        "bedrock" => {
            if lower.contains("claude") {
                if lower.contains("opus") {
                    (15.0, 75.0)
                } else if lower.contains("haiku") {
                    (0.25, 1.25)
                } else {
                    (3.0, 15.0)
                }
            } else {
                (3.0, 15.0)
            }
        }
        // Fallback to Claude Sonnet-class pricing
        _ => (3.0, 15.0),
    }
}

/// Build a replay entry from a message
fn build_replay_entry(index: usize, msg: &zeus_core::Message) -> Value {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    };

    let tool_name: Value = if !msg.tool_calls.is_empty() {
        json!(msg.tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>())
    } else {
        Value::Null
    };

    let tool_calls: Value = if !msg.tool_calls.is_empty() {
        json!(
            msg.tool_calls
                .iter()
                .map(|tc| {
                    let args_str = tc.arguments.to_string();
                    let redacted = zeus_core::sanitize::redact_secrets(&args_str);
                    let redacted_args = serde_json::from_str::<Value>(&redacted)
                        .unwrap_or_else(|_| Value::String(redacted));
                    json!({
                        "id": tc.id,
                        "name": tc.name,
                        "arguments": redacted_args,
                    })
                })
                .collect::<Vec<_>>()
        )
    } else {
        Value::Null
    };

    let tool_results: Value = if !msg.tool_results.is_empty() {
        json!(
            msg.tool_results
                .iter()
                .map(|tr| json!({
                    "call_id": tr.call_id,
                    "success": tr.success,
                    "output": zeus_core::sanitize::redact_secrets(&tr.output),
                }))
                .collect::<Vec<_>>()
        )
    } else {
        Value::Null
    };

    // Extract thinking/reasoning from content (prefixed with <thinking> tags)
    let thinking: Value = if msg.role == Role::Assistant && msg.content.contains("<thinking>") {
        if let Some(start) = msg.content.find("<thinking>") {
            if let Some(end) = msg.content.find("</thinking>") {
                json!(msg.content[start + 10..end].trim())
            } else {
                Value::Null
            }
        } else {
            Value::Null
        }
    } else {
        Value::Null
    };

    let token_count = estimate_message_tokens(msg);

    json!({
        "index": index,
        "timestamp": msg.timestamp.to_rfc3339(),
        "role": role,
        "content": zeus_core::sanitize::redact_secrets(&msg.content),
        "tool_calls": tool_calls,
        "tool_name": tool_name,
        "tool_results": tool_results,
        "thinking": thinking,
        "token_count": token_count,
    })
}

/// GET /v1/sessions/:id/replay — Full session replay data
pub async fn session_replay(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let entries: Vec<Value> = session
        .messages
        .iter()
        .enumerate()
        .map(|(i, msg)| build_replay_entry(i, msg))
        .collect();

    let total_tokens: usize = session.messages.iter().map(estimate_message_tokens).sum();

    Ok(Json(json!({
        "id": session.id,
        "created": session.created.to_rfc3339(),
        "total_turns": entries.len(),
        "total_tokens": total_tokens,
        "entries": entries,
    })))
}

/// GET /v1/sessions/:id/replay/:turn — Single turn by index
pub async fn session_replay_turn(
    State(state): State<SharedState>,
    Path((id, turn)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let session = Session::load(&state.config.sessions, &id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let msg = session.messages.get(turn).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!(
                "Turn {} not found (session has {} turns)",
                turn,
                session.messages.len()
            ),
        )
    })?;

    Ok(Json(build_replay_entry(turn, msg)))
}
