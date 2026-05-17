//! Bounty handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use crate::SharedState;
use crate::handlers::marketplace_store;

pub async fn bounty_create(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let poster_id = body
        .get("poster_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'poster_id'".to_string()))?;
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'title'".to_string()))?;
    let reward = body
        .get("reward_credits")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'reward_credits'".to_string(),
            )
        })?;

    let id = format!(
        "bounty-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    let now = chrono::Utc::now().to_rfc3339();
    let bounty = marketplace_store::BountyRow {
        id: id.clone(),
        poster_id: poster_id.to_string(),
        title: title.to_string(),
        description: body
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        reward_credits: reward,
        skill_tags_json: body
            .get("skill_tags")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "[]".to_string()),
        deadline: body
            .get("deadline")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        status: "open".to_string(),
        claimer_id: None,
        claimed_at: None,
        completed_at: None,
        verifier_id: None,
        verified_at: None,
        room_id: body
            .get("room_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now,
    };

    let state_guard = state.read().await;
    match state_guard.marketplace_store.post_bounty(&bounty).await {
        Ok(_) => Ok((
            StatusCode::CREATED,
            Json(json!({ "id": id, "status": "open", "reward_credits": reward })),
        )),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

pub async fn bounty_list(
    State(state): State<SharedState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let status = params.get("status").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(50);
    let state_guard = state.read().await;
    let bounties = state_guard
        .marketplace_store
        .list_bounties(status, limit)
        .await;
    let responses: Vec<marketplace_store::BountyResponse> =
        bounties.into_iter().map(|b| b.into()).collect();
    Json(json!({ "bounties": responses, "total": responses.len() }))
}

pub async fn bounty_get(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    match state_guard.marketplace_store.get_bounty(&id).await {
        Some(b) => {
            let resp: marketplace_store::BountyResponse = b.into();
            Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
        }
        None => Err((StatusCode::NOT_FOUND, "Bounty not found".to_string())),
    }
}

pub async fn bounty_claim(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let claimer_id = body
        .get("claimer_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'claimer_id'".to_string()))?;
    let state_guard = state.read().await;
    match state_guard
        .marketplace_store
        .claim_bounty(&id, claimer_id)
        .await
    {
        Ok(true) => Ok(Json(
            json!({ "status": "claimed", "claimer_id": claimer_id }),
        )),
        Ok(false) => Err((
            StatusCode::CONFLICT,
            "Bounty not claimable (already claimed or doesn't exist)".to_string(),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn bounty_submit(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    match state_guard.marketplace_store.submit_bounty(&id).await {
        Ok(true) => Ok(Json(json!({ "status": "submitted" }))),
        Ok(false) => Err((StatusCode::CONFLICT, "Bounty not submittable".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn bounty_verify(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let verifier_id = body
        .get("verifier_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'verifier_id'".to_string()))?;
    let state_guard = state.read().await;
    match state_guard
        .marketplace_store
        .verify_bounty(&id, verifier_id)
        .await
    {
        Ok(true) => Ok(Json(json!({ "status": "verified", "paid": true }))),
        Ok(false) => Err((StatusCode::CONFLICT, "Verification failed".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn bounty_cancel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    match state_guard.marketplace_store.cancel_bounty(&id).await {
        Ok(true) => Ok(Json(json!({ "status": "cancelled", "refunded": true }))),
        Ok(false) => Err((StatusCode::CONFLICT, "Bounty not cancellable".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn marketplace_reputation_badge(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let rep = state_guard
        .marketplace_store
        .get_reputation_with_badge(&agent_id)
        .await;
    Json(serde_json::to_value(rep).unwrap_or_default())
}

