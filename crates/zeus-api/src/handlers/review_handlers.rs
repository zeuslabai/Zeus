use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;

pub async fn submit_review(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let task_id = body
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'task_id'".to_string()))?;
    let agent_id = body
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'agent_id'".to_string()))?;
    let output = body
        .get("output")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'output'".to_string()))?;

    let submission = zeus_orchestra::peer_review::WorkSubmission::new(task_id, agent_id, output);
    let submission_id = submission.id.clone();

    let mut state_guard = state.write().await;
    let reviewers = state_guard
        .peer_review_mut()
        .submit(submission, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Submit failed: {e}"),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "submission_id": submission_id,
            "reviewers_assigned": reviewers,
            "status": "submitted",
        })),
    ))
}

/// GET /v1/reviews — list recent review log entries
pub async fn list_reviews(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);

    let state_guard = state.read().await;
    let log = state_guard.peer_review().log();
    let all_entries = log.entries();
    let start = if all_entries.len() > limit {
        all_entries.len() - limit
    } else {
        0
    };
    let entries: Vec<Value> = all_entries[start..]
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();
    let total = entries.len();

    Json(json!({
        "reviews": entries,
        "total": total,
    }))
}

/// GET /v1/reviews/:id — get reviews for a specific submission
pub async fn get_review(State(state): State<SharedState>, Path(id): Path<String>) -> Json<Value> {
    let state_guard = state.read().await;
    let log = state_guard.peer_review().log();

    let entries: Vec<Value> = log
        .by_submission(&id)
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();
    let reviews: Vec<Value> = log
        .reviews_for_submission(&id)
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();

    Json(json!({
        "submission_id": id,
        "entries": entries,
        "reviews": reviews,
        "review_count": reviews.len(),
    }))
}

/// POST /v1/reviews/:id/approve — record an approval review
pub async fn approve_review(
    State(state): State<SharedState>,
    Path(submission_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let reviewer_id = body
        .get("reviewer_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'reviewer_id'".to_string()))?;
    let score = body.get("score").and_then(|v| v.as_f64()).unwrap_or(0.8);
    let comments = body.get("comments").and_then(|v| v.as_str());

    let mut review = zeus_orchestra::peer_review::PeerReview::new(
        &submission_id,
        reviewer_id,
        score,
        zeus_orchestra::peer_review::ReviewVerdict::Approve,
    );
    if let Some(c) = comments {
        review = review.with_comments(c);
    }

    let mut state_guard = state.write().await;
    let consensus = state_guard.peer_review_mut().record_review(review, None);

    Ok(Json(json!({
        "submission_id": submission_id,
        "verdict": "approve",
        "score": score,
        "consensus": consensus.map(|c| serde_json::to_value(c).unwrap_or_default()),
    })))
}

/// POST /v1/reviews/:id/reject — record a rejection review
pub async fn reject_review(
    State(state): State<SharedState>,
    Path(submission_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let reviewer_id = body
        .get("reviewer_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'reviewer_id'".to_string()))?;
    let score = body.get("score").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let comments = body.get("comments").and_then(|v| v.as_str());
    let feedback = body.get("feedback").and_then(|v| v.as_str());

    let mut review = zeus_orchestra::peer_review::PeerReview::new(
        &submission_id,
        reviewer_id,
        score,
        zeus_orchestra::peer_review::ReviewVerdict::Reject,
    );
    if let Some(c) = comments.or(feedback) {
        review = review.with_comments(c);
    }

    let mut state_guard = state.write().await;
    let consensus = state_guard.peer_review_mut().record_review(review, None);

    Ok(Json(json!({
        "submission_id": submission_id,
        "verdict": "reject",
        "score": score,
        "consensus": consensus.map(|c| serde_json::to_value(c).unwrap_or_default()),
    })))
}
