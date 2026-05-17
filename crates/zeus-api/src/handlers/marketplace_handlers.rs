//! Marketplace handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use crate::SharedState;
use crate::handlers::marketplace_store;

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
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'publisher_id'".to_string(),
            )
        })?;

    let mut listing = zeus_marketplace::SkillListing::new(name, description, publisher_id);

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

    // Write-through to SQLite persistence
    let row = marketplace_store::SkillListingRow {
        id: skill_id.clone(),
        name: name.to_string(),
        description: description.to_string(),
        publisher_id: publisher_id.to_string(),
        capabilities_json: body
            .get("capabilities")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "[]".to_string()),
        tags_json: body
            .get("tags")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "[]".to_string()),
        price: body.get("price").and_then(|v| v.as_u64()).unwrap_or(0),
        version: body
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0")
            .to_string(),
        rating: 0.0,
        rating_count: 0,
        downloads: 0,
        active: true,
        source: body
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .to_string(),
        metadata_json: body
            .get("metadata")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    state_guard.marketplace_store.publish_listing(&row).await;

    let resp = zeus_marketplace::PublishResponse {
        skill_id,
        status: "published".into(),
    };
    let val = serde_json::to_value(resp).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    Ok((StatusCode::CREATED, Json(val)))
}

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
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'offered_price'".to_string(),
            )
        })?;

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
    let val = serde_json::to_value(resp).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    Ok((StatusCode::CREATED, Json(val)))
}

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

pub async fn marketplace_sync(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let skills_dir = state_guard.config.workspace.join("skills");
    let client = zeus_skills::ClawHubClient::new(skills_dir);

    // Fetch remote registry (builtins + remote, with offline fallback)
    let remote_skills = client.list_all().await;

    let mut synced = 0;
    for skill in &remote_skills {
        let source = if skill.author == "zeus" {
            "builtin"
        } else {
            "clawhub"
        };
        let policy = zeus_skills::SkillPermissionPolicy::for_source(&skill.name, source);
        let row = marketplace_store::SkillListingRow {
            id: skill.name.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            publisher_id: skill.author.clone(),
            capabilities_json: serde_json::to_string(&skill.categories)
                .unwrap_or_else(|_| "[]".to_string()),
            tags_json: serde_json::to_string(&skill.tags).unwrap_or_else(|_| "[]".to_string()),
            price: 0,
            version: skill.version.clone(),
            rating: 0.0,
            rating_count: 0,
            downloads: skill.downloads,
            active: true,
            source: source.to_string(),
            metadata_json: serde_json::json!({
                "trust_level": policy.trust_level,
                "sandbox": if source == "builtin" { "trusted" } else { "marketplace_restricted" },
            })
            .to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if state_guard.marketplace_store.publish_listing(&row).await {
            synced += 1;
        }
    }

    Ok(Json(json!({
        "synced": synced,
        "total_available": remote_skills.len(),
        "message": format!("Synced {} skills from ClawHub registry", synced),
    })))
}

pub async fn sync_builtins_to_marketplace(store: &marketplace_store::MarketplaceStore) {
    let client = zeus_skills::ClawHubClient::default();
    let builtins = client.search_builtins("");
    let mut count = 0;
    for skill in builtins {
        let row = marketplace_store::SkillListingRow {
            id: skill.name.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            publisher_id: skill.author.clone(),
            capabilities_json: serde_json::to_string(&skill.categories)
                .unwrap_or_else(|_| "[]".to_string()),
            tags_json: serde_json::to_string(&skill.tags).unwrap_or_else(|_| "[]".to_string()),
            price: 0,
            version: skill.version.clone(),
            rating: 0.0,
            rating_count: 0,
            downloads: skill.downloads,
            active: true,
            source: "builtin".to_string(),
            metadata_json: r#"{"trust_level":2,"sandbox":"trusted"}"#.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if store.publish_listing(&row).await {
            count += 1;
        }
    }
    if count > 0 {
        tracing::info!("Synced {} builtin skills to marketplace store", count);
    }
}

pub async fn marketplace_stats(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let stats = state_guard.marketplace_store.stats().await;
    Json(serde_json::to_value(stats).unwrap_or_default())
}

pub async fn marketplace_featured(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10);
    let state_guard = state.read().await;
    let rows = state_guard.marketplace_store.featured_listings(limit).await;
    let listings: Vec<marketplace_store::SkillListingResponse> =
        rows.into_iter().map(Into::into).collect();
    let total = listings.len();
    Json(serde_json::json!({ "listings": listings, "total": total }))
}

pub async fn marketplace_categories(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let categories = state_guard.marketplace_store.list_categories().await;
    Json(serde_json::to_value(categories).unwrap_or_default())
}

pub async fn marketplace_search(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let query = params.get("q").map(|s| s.as_str()).unwrap_or("");
    let state_guard = state.read().await;
    let rows = state_guard.marketplace_store.search_listings(query).await;
    let listings: Vec<marketplace_store::SkillListingResponse> =
        rows.into_iter().map(Into::into).collect();
    let total = listings.len();
    Json(serde_json::json!({ "listings": listings, "total": total }))
}

pub async fn marketplace_ratings(
    State(state): State<SharedState>,
    Path(skill_id): Path<String>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let ratings = state_guard.marketplace_store.get_ratings(&skill_id).await;
    Json(serde_json::json!({ "skill_id": skill_id, "ratings": ratings, "total": ratings.len() }))
}

pub async fn marketplace_add_rating(
    State(state): State<SharedState>,
    Path(skill_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let agent_id = body
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'agent_id'".to_string()))?;
    let agent_name = body
        .get("agent_name")
        .and_then(|v| v.as_str())
        .unwrap_or(agent_id);
    let score = body
        .get("score")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'score'".to_string()))?;
    let comment = body.get("comment").and_then(|v| v.as_str()).unwrap_or("");

    if !(0.0..=5.0).contains(&score) {
        return Err((StatusCode::BAD_REQUEST, "Score must be 0.0-5.0".to_string()));
    }

    let state_guard = state.read().await;
    state_guard
        .marketplace_store
        .add_rating(&skill_id, agent_id, agent_name, score, comment)
        .await;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "rated", "skill_id": skill_id, "score": score })),
    ))
}

