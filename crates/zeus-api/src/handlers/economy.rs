//! Economy and Marketplace API handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Marketplace
// ============================================================================

/// GET /v1/marketplace/listings — List marketplace skill listings
pub async fn marketplace_list(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let marketplace = &state_guard.marketplace;

    let listings = if let Some(cap) = params.get("capability") {
        marketplace.registry.search_by_capability(cap).await
    } else if let Some(tag) = params.get("tag") {
        marketplace.registry.search_by_tag(tag).await
    } else if let Some(query) = params.get("q") {
        marketplace.registry.search_by_name(query).await
    } else if let Some(publisher) = params.get("publisher") {
        marketplace.registry.search_by_publisher(publisher).await
    } else {
        marketplace.registry.list_active().await
    };

    let entries: Vec<zeus_marketplace::MarketplaceListingResponse> =
        listings.into_iter().map(Into::into).collect();
    let total = entries.len();
    let resp = zeus_marketplace::MarketplaceListResponse {
        listings: entries,
        total,
    };
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// POST /v1/marketplace/listings — publish a new skill listing
pub async fn marketplace_publish(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name'".to_string()))?;
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'description'".to_string()))?;
    let publisher_id = body
        .get("publisher_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'publisher_id'".to_string()))?;

    let mut listing =
        zeus_marketplace::SkillListing::new(name, description, publisher_id);

    if let Some(caps) = body.get("capabilities").and_then(|v| v.as_array()) {
        let cap_vec: Vec<String> = caps
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        listing = listing.with_capabilities(cap_vec);
    }
    if let Some(tags) = body.get("tags").and_then(|v| v.as_array()) {
        let tag_vec: Vec<String> = tags
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        listing = listing.with_tags(tag_vec);
    }
    if let Some(price) = body.get("price").and_then(|v| v.as_u64()) {
        listing = listing.with_price(price);
    }
    if let Some(version) = body.get("version").and_then(|v| v.as_str()) {
        listing = listing.with_version(version);
    }

    let state_guard = state.read().await;
    let skill_id = state_guard
        .marketplace
        .registry
        .publish(listing)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Publish failed: {e}"),
            )
        })?;

    let resp = zeus_marketplace::PublishResponse {
        skill_id,
        status: "published".into(),
    };
    let val = serde_json::to_value(resp)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    Ok((StatusCode::CREATED, Json(val)))
}

/// POST /v1/marketplace/trade — initiate a trade
pub async fn marketplace_trade(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let buyer_id = body
        .get("buyer_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'buyer_id'".to_string()))?;
    let skill_id = body
        .get("skill_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'skill_id'".to_string()))?;
    let offered_price = body
        .get("offered_price")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'offered_price'".to_string()))?;

    // Look up the listing to get seller_id
    let state_guard = state.read().await;
    let listing = state_guard
        .marketplace
        .registry
        .get(skill_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Skill not found: {e}")))?;

    let mut trade =
        zeus_marketplace::Trade::new(buyer_id, &listing.publisher_id, skill_id, offered_price);
    if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
        trade = trade.with_message(msg);
    }

    let trade_id = state_guard
        .marketplace
        .trades
        .propose(trade)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Trade proposal failed: {e}"),
            )
        })?;

    let resp = zeus_marketplace::TradeResponse {
        trade_id,
        status: "proposed".into(),
        buyer_id: buyer_id.to_string(),
        seller_id: listing.publisher_id.clone(),
        skill_id: skill_id.to_string(),
        price: offered_price,
        timestamp: chrono::Utc::now(),
    };
    let val = serde_json::to_value(resp)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    Ok((StatusCode::CREATED, Json(val)))
}

/// GET /v1/marketplace/ledger/:agent_id — get token balance + transactions
pub async fn marketplace_ledger(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let balance = state_guard.marketplace.ledger.balance(&agent_id).await;
    let transactions = state_guard
        .marketplace
        .ledger
        .transactions_for(&agent_id)
        .await;

    let txns: Vec<zeus_marketplace::TransactionResponse> =
        transactions.into_iter().map(Into::into).collect();
    let resp = zeus_marketplace::LedgerResponse {
        agent_id,
        balance,
        transactions: txns,
    };
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// GET /v1/marketplace/reputation/:agent_id — get reputation score
pub async fn marketplace_reputation(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let rep = state_guard
        .marketplace
        .reputation
        .get_or_create(&agent_id)
        .await;

    let resp = zeus_marketplace::ReputationResponse {
        agent_id,
        score: rep.trust_score,
        total_trades: rep.total_trades,
        successful_trades: rep.successful_trades,
        ratings: rep.avg_skill_rating,
        trade_success_rate: rep.trade_success_rate(),
    };
    Json(serde_json::to_value(resp).unwrap_or_default())
}

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
