//! Agora — agent skill marketplace HTTP handlers.
//!
//! Routes:
//!   GET    /v1/agora/listings                      — all skill listings
//!   POST   /v1/agora/listings                      — register a skill listing
//!   GET    /v1/agora/listings/:agent_id             — listings for a specific agent
//!   DELETE /v1/agora/listings/:agent_id/:skill      — remove a listing
//!   GET    /v1/agora/search                         — search listings (query params)
//!   GET    /v1/agora/wallets/:agent_id              — credit balance for agent
//!   POST   /v1/agora/wallets/:agent_id              — register agent wallet
//!   POST   /v1/agora/buy                            — purchase a skill execution
//!   GET    /v1/agora/transactions                   — list transactions
//!   GET    /v1/agora/reputation/:agent_id           — reputation score for agent

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use zeus_agora::{SearchQuery, SkillListing, TransactionFilter};

use crate::SharedState;

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterWalletBody {
    pub initial_balance: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BuyBody {
    pub buyer_agent_id: String,
    pub seller_agent_id: String,
    pub skill_name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub agent_id: Option<String>,
    pub max_price: Option<i64>,
    pub min_success_rate: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct TransactionParams {
    pub agent_id: Option<String>,
    pub skill_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgoraErrorResponse {
    pub error: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn agora_err(msg: impl std::fmt::Display) -> (StatusCode, Json<AgoraErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(AgoraErrorResponse {
            error: msg.to_string(),
        }),
    )
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /v1/agora/listings — return all skill listings
pub async fn agora_listings(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let listings: Vec<&SkillListing> = s.agora.all_listings();
    Json(listings).into_response()
}

/// POST /v1/agora/listings — register a new skill listing
pub async fn agora_list_skill(
    State(state): State<SharedState>,
    Json(listing): Json<SkillListing>,
) -> impl IntoResponse {
    let mut s = state.write().await;
    match s.agora.list_skill(listing) {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => agora_err(e).into_response(),
    }
}

/// GET /v1/agora/listings/:agent_id — listings for a specific agent
pub async fn agora_agent_listings(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let all = s.agora.all_listings();
    let filtered: Vec<&SkillListing> = all.into_iter().filter(|l| l.agent_id == agent_id).collect();
    Json(filtered).into_response()
}

/// DELETE /v1/agora/listings/:agent_id/:skill — remove a listing
pub async fn agora_delist_skill(
    State(state): State<SharedState>,
    Path((agent_id, skill)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut s = state.write().await;
    match s.agora.delist_skill(&agent_id, &skill) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => agora_err(e).into_response(),
    }
}

/// GET /v1/agora/search — search listings
pub async fn agora_search(
    State(state): State<SharedState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let s = state.read().await;
    let query = SearchQuery {
        text: params.q,
        agent_id: params.agent_id,
        max_price: params.max_price,
        min_success_rate: params.min_success_rate,
        tags: vec![],
        capabilities: vec![],
        limit: None,
    };
    let results = s.agora.search(&query);
    Json(results).into_response()
}

/// GET /v1/agora/wallets/:agent_id — credit balance
pub async fn agora_wallet(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    match s.agora.balance(&agent_id) {
        Some(balance) => {
            Json(serde_json::json!({ "agent_id": agent_id, "balance": balance })).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(AgoraErrorResponse {
                error: format!("No wallet for agent {agent_id}"),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/agora/wallets/:agent_id — register wallet
pub async fn agora_register_wallet(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
    Json(body): Json<RegisterWalletBody>,
) -> impl IntoResponse {
    let mut s = state.write().await;
    let balance = body.initial_balance.unwrap_or(1000);
    s.agora.register_wallet(&agent_id, balance);
    Json(serde_json::json!({ "agent_id": agent_id, "balance": balance })).into_response()
}

/// POST /v1/agora/buy — purchase a skill execution
pub async fn agora_buy(
    State(state): State<SharedState>,
    Json(body): Json<BuyBody>,
) -> impl IntoResponse {
    let mut s = state.write().await;
    match s
        .agora
        .purchase(&body.buyer_agent_id, &body.seller_agent_id, &body.skill_name)
    {
        Ok(tx) => (StatusCode::CREATED, Json(tx)).into_response(),
        Err(e) => agora_err(e).into_response(),
    }
}

/// GET /v1/agora/transactions — list transactions
pub async fn agora_transactions(
    State(state): State<SharedState>,
    Query(params): Query<TransactionParams>,
) -> impl IntoResponse {
    let s = state.read().await;
    let filter = TransactionFilter {
        agent_id: params.agent_id,
        skill_name: params.skill_name,
        ..Default::default()
    };
    let txs = s.agora.list_transactions(&filter);
    Json(txs).into_response()
}

/// GET /v1/agora/reputation/:agent_id
pub async fn agora_reputation(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let score = s.agora.reputation(&agent_id);
    Json(score).into_response()
}
