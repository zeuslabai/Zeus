// Economy, wallets, marketplace, bounties, network

use super::*;

// Economy

pub async fn economy_earn(agent_id: &str, tools_used: usize, complexity: &str, note: Option<&str>) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/earn", &serde_json::json!({
        "agent_id": agent_id, "tools_used": tools_used, "complexity": complexity, "note": note,
    })).await
}

pub async fn economy_mint(agent_id: &str, amount: u64, reason: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/mint", &serde_json::json!({ "agent_id": agent_id, "amount": amount, "reason": reason })).await
}

pub async fn economy_stake(agent_id: &str, amount: u64, purpose: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/stake", &serde_json::json!({ "agent_id": agent_id, "amount": amount, "purpose": purpose })).await
}

pub async fn economy_unstake(agent_id: &str, stake_id: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/unstake", &serde_json::json!({ "agent_id": agent_id, "stake_id": stake_id })).await
}

pub async fn economy_transfer(from: &str, to: &str, amount: u64, note: Option<&str>) -> Result<serde_json::Value, String> {
    post_json("/v1/economy/transfer", &serde_json::json!({
        "from": from, "to": to, "amount": amount, "note": note,
    })).await
}

pub async fn fetch_wallets() -> Result<WalletsResponse, String> {
    fetch_json("/v1/economy/wallets").await
}

pub async fn fetch_wallet(agent_id: &str) -> Result<Wallet, String> {
    fetch_json(&format!("/v1/economy/wallets/{}", agent_id)).await
}

pub async fn fetch_transactions(limit: Option<usize>) -> Result<Vec<Transaction>, String> {
    let url = if let Some(lim) = limit {
        format!("/v1/economy/transactions?limit={}", lim)
    } else {
        "/v1/economy/transactions".to_string()
    };
    fetch_json(&url).await
}

pub async fn fetch_economy_wallets() -> Result<EconomyWalletsResponse, String> {
    fetch_json("/v1/economy/wallets").await
}

pub async fn fetch_economy_wallet(agent_id: &str) -> Result<EconomyWalletDetail, String> {
    fetch_json(&format!("/v1/economy/wallets/{}", agent_id)).await
}

// Marketplace

pub async fn fetch_marketplace_listings(
    capability: Option<&str>,
    tag: Option<&str>,
    query: Option<&str>,
    publisher: Option<&str>,
) -> Result<MarketplaceListingsResponse, String> {
    let mut url = String::from("/v1/marketplace/listings");
    let mut params = Vec::new();

    if let Some(cap) = capability {
        params.push(format!("capability={}", cap));
    }
    if let Some(t) = tag {
        params.push(format!("tag={}", t));
    }
    if let Some(q) = query {
        params.push(format!("q={}", q));
    }
    if let Some(p) = publisher {
        params.push(format!("publisher={}", p));
    }

    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }

    fetch_json(&url).await
}

pub async fn publish_marketplace_listing(
    request: &PublishListingRequest,
) -> Result<PublishListingResponse, String> {
    post_json("/v1/marketplace/listings", request).await
}

pub async fn marketplace_trade(request: &TradeRequest) -> Result<TradeResponse, String> {
    post_json("/v1/marketplace/trade", request).await
}

pub async fn fetch_marketplace_featured(limit: usize) -> Result<MarketplaceListingsResponse, String> {
    fetch_json(&format!("/v1/marketplace/featured?limit={}", limit)).await
}

pub async fn fetch_marketplace_stats() -> Result<MarketplaceStats, String> {
    fetch_json("/v1/marketplace/stats").await
}

pub async fn fetch_marketplace_categories() -> Result<MarketplaceCategoriesResponse, String> {
    fetch_json("/v1/marketplace/categories").await
}

pub async fn fetch_skill_ratings(skill_id: &str) -> Result<SkillRatingsResponse, String> {
    fetch_json(&format!("/v1/marketplace/ratings/{}", skill_id)).await
}

pub async fn submit_skill_rating(skill_id: &str, agent_id: &str, score: f64, comment: Option<&str>) -> Result<MsgResponse, String> {
    let mut body = serde_json::json!({ "agent_id": agent_id, "score": score });
    if let Some(c) = comment { body["comment"] = serde_json::json!(c); }
    post_json(&format!("/v1/marketplace/ratings/{}", skill_id), &body).await
}

pub async fn fetch_marketplace_ledger(agent_id: &str) -> Result<LedgerResponse, String> {
    fetch_json(&format!("/v1/marketplace/ledger/{}", agent_id)).await
}

pub async fn fetch_marketplace_reputation(agent_id: &str) -> Result<ReputationResponse, String> {
    fetch_json(&format!("/v1/marketplace/reputation/{}", agent_id)).await
}

// Bounties

pub async fn fetch_bounties(status: Option<&str>) -> Result<BountiesResponse, String> {
    let q = status.map(|s| format!("?status={}", s)).unwrap_or_default();
    fetch_json(&format!("/v1/marketplace/bounties{}", q)).await
}

pub async fn claim_bounty(bounty_id: &str, agent_id: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/marketplace/bounties/{}/claim", bounty_id),
        &serde_json::json!({ "agent_id": agent_id })).await
}

// Network

pub async fn fetch_network_discover() -> Result<NetworkDiscoverResponse, String> {
    fetch_json("/v1/network/discover").await
}

pub async fn fetch_network_messages() -> Result<NetworkMessagesResponse, String> {
    fetch_json("/v1/network/messages").await
}

pub async fn network_send(host: &str, port: Option<u16>, from_agent: &str, to_agent: Option<&str>, content: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/network/send", &serde_json::json!({
        "host": host, "port": port, "from_agent": from_agent, "to_agent": to_agent, "content": content,
    })).await
}

pub async fn network_broadcast(from_agent: &str, content: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/network/broadcast", &serde_json::json!({ "from_agent": from_agent, "content": content })).await
}
