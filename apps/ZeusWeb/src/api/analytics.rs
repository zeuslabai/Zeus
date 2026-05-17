// Analytics: costs, budgets, daily, per-model, tokens, provider costs, routing

use super::*;

pub async fn fetch_costs() -> Result<CostsResponse, String> {
    fetch_json("/v1/analytics/costs").await
}

pub async fn fetch_tokens() -> Result<TokensResponse, String> {
    fetch_json("/v1/analytics/tokens").await
}

pub async fn fetch_provider_costs() -> Result<ProviderCostsResponse, String> {
    fetch_json("/v1/analytics/providers").await
}

pub async fn fetch_budgets() -> Result<BudgetsResponse, String> {
    fetch_json("/v1/analytics/budgets").await
}

pub async fn fetch_daily_analytics(days: u32) -> Result<DailyAnalyticsResponse, String> {
    fetch_json(&format!("/v1/analytics/daily?days={}", days)).await
}

pub async fn fetch_model_analytics() -> Result<ModelsAnalyticsResponse, String> {
    fetch_json("/v1/analytics/models").await
}

pub async fn fetch_task_costs() -> Result<Vec<TaskCost>, String> {
    let resp: serde_json::Value = fetch_json("/v1/analytics/sessions").await?;
    let sessions = resp.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    Ok(sessions.iter().filter_map(|s| {
        Some(TaskCost {
            task_id: s.get("session_id")?.as_str()?.to_string(),
            task_name: String::new(),
            agent_id: String::new(),
            model: s.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string(),
            tokens: s.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
            cost: s.get("estimated_cost").and_then(|c| c.as_f64()).unwrap_or(0.0),
            timestamp: s.get("created").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        })
    }).collect())
}

pub async fn fetch_fallback_chain() -> Result<Vec<FallbackChainEntry>, String> {
    let resp: ProviderCostsResponse = fetch_json("/v1/analytics/providers").await?;
    Ok(resp.providers.iter().enumerate().map(|(i, p)| FallbackChainEntry {
        provider: p.provider.clone(),
        model: String::new(),
        priority: i as u32,
        enabled: p.tokens > 0 || p.requests > 0,
    }).collect())
}

// Routing / Cost

pub async fn fetch_routing_costs() -> Result<RoutingCostsResponse, String> {
    fetch_json("/v1/routing/costs").await
}

pub async fn fetch_routing_budget() -> Result<RoutingBudgetResponse, String> {
    fetch_json("/v1/routing/budget").await
}

pub async fn fetch_routing_recommend(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/routing/recommend", body).await
}

pub async fn fetch_cost_recommend(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json("/v1/routing/cost-recommend", body).await
}
