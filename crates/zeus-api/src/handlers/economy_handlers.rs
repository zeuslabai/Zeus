//! Economy handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Economy
// ============================================================================

/// GET /v1/economy/wallets — list all agent wallets
pub async fn economy_wallets(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let wallets = state
        .ledger
        .all_wallets()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let total_supply = state
        .ledger
        .total_supply()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "wallets": wallets,
        "total_supply": total_supply,
    })))
}

/// GET /v1/economy/wallets/:agent_id — get single agent wallet + recent txs
pub async fn economy_wallet(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let wallet = state
        .ledger
        .wallet(&agent_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let transactions = state
        .ledger
        .transactions_for(&agent_id, 50)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "wallet": wallet,
        "transactions": transactions,
    })))
}

/// GET /v1/economy/transactions — recent global transactions
pub async fn economy_transactions(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let limit: usize = params
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(50);
    let transactions = state
        .ledger
        .all_transactions(limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (minted, burned) = state
        .ledger
        .mint_burn_summary()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "transactions": transactions,
        "total_minted": minted,
        "total_burned": burned,
    })))
}

