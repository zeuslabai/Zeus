//! Security-related handlers — approvals for tool execution.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DenyRequest {
    pub reason: Option<String>,
}

// ============================================================================
// Approvals
// ============================================================================

/// GET /v1/approvals — list pending approvals
pub async fn list_approvals(State(state): State<SharedState>) -> Json<Value> {
    let guard = state.read().await;
    let pending = guard.approvals.list_pending();
    Json(json!(pending))
}

/// POST /v1/approvals/:id/approve — approve a pending tool execution
pub async fn approve_execution(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.write().await;
    match guard.approvals.approve(&id) {
        Ok(()) => (StatusCode::OK, Json(json!({"approved": true, "id": id}))),
        Err(msg) => (StatusCode::NOT_FOUND, Json(json!({"error": msg}))),
    }
}

/// POST /v1/approvals/:id/deny — deny a pending tool execution
pub async fn deny_execution(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    body: Option<Json<DenyRequest>>,
) -> (StatusCode, Json<Value>) {
    let reason = body.and_then(|b| b.0.reason);
    let mut guard = state.write().await;
    match guard.approvals.deny(&id, reason) {
        Ok(()) => (StatusCode::OK, Json(json!({"denied": true, "id": id}))),
        Err(msg) => (StatusCode::NOT_FOUND, Json(json!({"error": msg}))),
    }
}

/// GET /v1/aegis/approvals/stream — Server-Sent Events stream of approval events.
///
/// Mirrors the broadcast→SSE pattern used by `agent_status_stream`
/// (`agent_handlers.rs`) and the `ChatBroadcast` surface: best-effort delivery,
/// slow consumers are dropped (tokio broadcast semantics) so back-pressure never
/// blocks the response.
///
/// On connect the handler emits a `snapshot` event carrying the current pending
/// queue, then streams each subsequent [`ApprovalEvent`](crate::ApprovalEvent)
/// (create/resolve) as an `approval_event` SSE event. The subscriber is dropped
/// automatically when the client disconnects (the stream future is cancelled).
pub async fn approvals_stream(
    State(state): State<SharedState>,
) -> axum::response::sse::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let guard = state.read().await;
    let mut rx = guard.approvals.subscribe();
    // Capture the current pending queue for the initial snapshot event.
    let snapshot: Value = json!({
        "type": "snapshot",
        "pending": guard.approvals.list_pending(),
    });
    drop(guard);

    let stream = async_stream::stream! {
        // Emit the current pending state first so a fresh consumer is in sync
        // without needing a separate poll.
        if let Ok(data) = serde_json::to_string(&snapshot) {
            yield Ok(axum::response::sse::Event::default()
                .event("snapshot")
                .data(data));
        }
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok(axum::response::sse::Event::default()
                            .event("approval_event")
                            .data(data));
                    }
                }
                // Slow consumer fell behind — log and keep streaming live events.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("Approvals SSE stream lagged {} messages", n);
                }
                // Sender dropped (shutdown) — end the stream.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    axum::response::sse::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
}
