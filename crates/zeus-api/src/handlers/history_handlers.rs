use axum::extract::{Query, State};
use axum::Json;
use serde_json::{json, Value};

use crate::SharedState;

// ============================================================================
// Discord History
// ============================================================================

pub async fn discord_history(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let channel_id = params.get("channel_id").map(|s| s.as_str()).unwrap_or("");
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);

    if channel_id.is_empty() {
        return Json(json!({"error": "channel_id is required", "messages": []}));
    }

    let guard = state.read().await;
    let messages = guard.discord_history.get_history(channel_id, limit).await;
    Json(json!({"messages": messages, "count": messages.len(), "channel_id": channel_id}))
}

pub async fn discord_history_search(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let query = params.get("q").or(params.get("query")).map(|s| s.as_str()).unwrap_or("");
    let channel_id = params.get("channel_id").map(|s| s.as_str());
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    if query.is_empty() {
        return Json(json!({"error": "q or query parameter is required", "messages": []}));
    }

    let guard = state.read().await;
    let messages = guard.discord_history.search(query, channel_id, limit).await;
    Json(json!({"messages": messages, "count": messages.len()}))
}

pub async fn discord_history_stats(
    State(state): State<SharedState>,
) -> Json<Value> {
    let guard = state.read().await;
    let total = guard.discord_history.count().await;
    let by_channel = guard.discord_history.count_by_channel().await;
    Json(json!({"total_messages": total, "channels": by_channel}))
}

// ============================================================================
// Slack History
// ============================================================================

pub async fn slack_history(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let channel_id = params.get("channel_id").map(|s| s.as_str()).unwrap_or("");
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);

    if channel_id.is_empty() {
        return Json(json!({"error": "channel_id is required", "messages": []}));
    }

    let guard = state.read().await;
    let messages = guard.slack_history.get_history(channel_id, limit).await;
    Json(json!({"messages": messages, "count": messages.len(), "channel_id": channel_id}))
}

pub async fn slack_history_thread(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let channel_id = params.get("channel_id").map(|s| s.as_str()).unwrap_or("");
    let thread_ts = params.get("thread_ts").map(|s| s.as_str()).unwrap_or("");
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);

    if channel_id.is_empty() || thread_ts.is_empty() {
        return Json(json!({"error": "channel_id and thread_ts are required", "messages": []}));
    }

    let guard = state.read().await;
    let messages = guard.slack_history.get_thread(channel_id, thread_ts, limit).await;
    Json(json!({"messages": messages, "count": messages.len(), "channel_id": channel_id, "thread_ts": thread_ts}))
}

pub async fn slack_history_search(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let query = params.get("q").or(params.get("query")).map(|s| s.as_str()).unwrap_or("");
    let channel_id = params.get("channel_id").map(|s| s.as_str());
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    if query.is_empty() {
        return Json(json!({"error": "q or query parameter is required", "messages": []}));
    }

    let guard = state.read().await;
    let messages = guard.slack_history.search(query, channel_id, limit).await;
    Json(json!({"messages": messages, "count": messages.len()}))
}

pub async fn slack_history_stats(
    State(state): State<SharedState>,
) -> Json<Value> {
    let guard = state.read().await;
    let total = guard.slack_history.count().await;
    let by_channel = guard.slack_history.count_by_channel().await;
    Json(json!({"total_messages": total, "channels": by_channel}))
}
