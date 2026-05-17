//! Analytics endpoint handlers
//!
//! Extracted from mod.rs — cost, token, provider, budget, session, daily, and model analytics.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;
use super::PaginationParams;

// ============================================================================
// Analytics Endpoints
// ============================================================================

/// GET /v1/analytics/costs — Cost aggregation from CostRouter
pub async fn analytics_costs(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let summary = state.cost_router.summary();

    Json(json!({
        "total_cost": summary.total_cost,
        "budget_remaining": summary.budget_remaining,
        "top_models": summary.top_models,
        "period_start": summary.period_start,
        "total_requests": state.cost_router.total_requests(),
        "total_input_tokens": state.cost_router.total_input_tokens(),
        "total_output_tokens": state.cost_router.total_output_tokens(),
        "currency": "USD"
    }))
}

/// GET /v1/analytics/tokens — Token usage from CostRouter
pub async fn analytics_tokens(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let tokens = state.cost_router.model_tokens();

    let by_model: serde_json::Map<String, Value> = tokens
        .iter()
        .map(|(model, (input, output))| {
            (
                model.clone(),
                json!({
                    "input_tokens": input,
                    "output_tokens": output,
                    "total_tokens": input + output,
                }),
            )
        })
        .collect();

    Json(json!({
        "total_input_tokens": state.cost_router.total_input_tokens(),
        "total_output_tokens": state.cost_router.total_output_tokens(),
        "total_tokens": state.cost_router.total_input_tokens() + state.cost_router.total_output_tokens(),
        "total_requests": state.cost_router.total_requests(),
        "by_model": by_model,
    }))
}

/// GET /v1/analytics/providers — All providers with configuration status
pub async fn analytics_providers(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let (active_provider, _) = state.config.parse_model();

    let providers = vec![
        (
            "Anthropic",
            "ANTHROPIC_API_KEY",
            zeus_core::Provider::Anthropic,
        ),
        ("OpenAI", "OPENAI_API_KEY", zeus_core::Provider::OpenAI),
        (
            "OpenRouter",
            "OPENROUTER_API_KEY",
            zeus_core::Provider::OpenRouter,
        ),
        ("Google", "GOOGLE_API_KEY", zeus_core::Provider::Google),
        ("Groq", "GROQ_API_KEY", zeus_core::Provider::Groq),
        ("Mistral", "MISTRAL_API_KEY", zeus_core::Provider::Mistral),
        (
            "Together",
            "TOGETHER_API_KEY",
            zeus_core::Provider::Together,
        ),
        (
            "Fireworks",
            "FIREWORKS_API_KEY",
            zeus_core::Provider::Fireworks,
        ),
        ("Azure", "AZURE_OPENAI_API_KEY", zeus_core::Provider::Azure),
        ("Bedrock", "AWS_ACCESS_KEY_ID", zeus_core::Provider::Bedrock),
        ("Ollama", "", zeus_core::Provider::Ollama),
    ];

    let provider_list: Vec<Value> = providers
        .iter()
        .map(|(name, env_var, provider)| {
            let configured = if env_var.is_empty() {
                // Ollama: check if it's the active provider or if OLLAMA_HOST is set
                std::mem::discriminant(&active_provider) == std::mem::discriminant(provider)
                    || std::env::var("OLLAMA_HOST").is_ok()
            } else {
                std::env::var(env_var).is_ok()
            };
            let is_active =
                std::mem::discriminant(&active_provider) == std::mem::discriminant(provider);
            json!({
                "name": name,
                "configured": configured,
                "active": is_active,
                "requests": 0,
                "tokens": 0,
                "cost": 0.0
            })
        })
        .collect();

    Json(json!({ "providers": provider_list }))
}

/// GET /v1/analytics/budgets — Budget overview from economy ledger + cost router
pub async fn analytics_budgets(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    let total_supply = state.ledger.total_supply().unwrap_or(0);
    let (total_minted, total_burned) = state.ledger.mint_burn_summary().unwrap_or((0, 0));

    let cost_summary = state.cost_router.summary();

    let wallets = state.ledger.all_wallets().unwrap_or_default();
    let agent_budgets: Vec<serde_json::Value> = wallets
        .iter()
        .map(|w| {
            let utilization = if total_supply > 0 {
                w.total_spent as f64 / total_supply as f64 * 100.0
            } else {
                0.0
            };
            json!({
                "agent_id": w.agent_id,
                "balance": w.balance,
                "total_spent": w.total_spent,
                "total_earned": w.total_earned,
                "utilization_pct": (utilization * 100.0).round() / 100.0,
            })
        })
        .collect();

    let mut alerts: Vec<serde_json::Value> = Vec::new();
    for w in &wallets {
        if w.balance == 0 && w.total_spent > 0 {
            alerts.push(json!({
                "type": "depleted",
                "agent_id": w.agent_id,
                "message": format!("Agent {} has 0 balance (spent {})", w.agent_id, w.total_spent),
            }));
        } else if w.balance > 0 && w.balance < 100 && w.total_spent > 0 {
            alerts.push(json!({
                "type": "low_balance",
                "agent_id": w.agent_id,
                "message": format!("Agent {} low balance: {}", w.agent_id, w.balance),
                "balance": w.balance,
            }));
        }
    }

    Json(json!({
        "implemented": true,
        "total_supply": total_supply,
        "total_minted": total_minted,
        "total_burned": total_burned,
        "llm_cost_usd": cost_summary.total_cost,
        "budget_remaining_usd": cost_summary.budget_remaining,
        "budgets": agent_budgets,
        "alerts": alerts,
    }))
}

/// GET /v1/analytics/sessions — Per-session token counts and cost estimates
///
/// Scans all sessions, counts tokens, and estimates cost using the configured model.
/// Supports `?limit=N` (default 20) and `?offset=N` query parameters.
pub async fn analytics_sessions(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let sessions_dir = &state.config.sessions;

    let sessions = zeus_session::Session::list(sessions_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let limit = params
        .limit
        .unwrap_or(20)
        .min(zeus_core::MAX_PAGE_LIMIT_SMALL as u32) as usize;
    let offset = params.offset.unwrap_or(0) as usize;
    let total = sessions.len();
    let estimator = zeus_session::CostEstimator::with_defaults();
    let model = &state.config.model;

    let mut results = Vec::new();
    for (id, created) in sessions.iter().skip(offset).take(limit) {
        match zeus_session::Session::load(sessions_dir, id).await {
            Ok(session) => {
                let usage = zeus_session::count_session_tokens(&session.messages);
                let cost = estimator.estimate_from_usage(&usage, model);
                let last_activity = session
                    .messages
                    .last()
                    .map(|m| m.timestamp.to_rfc3339())
                    .unwrap_or_default();

                results.push(json!({
                    "session_id": id,
                    "created": created.to_rfc3339(),
                    "last_activity": last_activity,
                    "message_count": session.messages.len(),
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "total_tokens": usage.total_tokens,
                    "estimated_cost": cost.total_cost,
                    "model": model,
                }));
            }
            Err(_) => continue,
        }
    }

    Ok(Json(json!({
        "sessions": results,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

/// GET /v1/analytics/daily — Daily cost and token aggregation
///
/// Aggregates session data by day (based on session creation date).
/// Returns the last `?days=N` days (default 7, max 90).
pub async fn analytics_daily(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let sessions_dir = &state.config.sessions;
    let days: usize = params
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(7)
        .min(90);

    let sessions = zeus_session::Session::list(sessions_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let estimator = zeus_session::CostEstimator::with_defaults();
    let model = &state.config.model;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    // Aggregate by date string (YYYY-MM-DD)
    let mut daily: std::collections::BTreeMap<String, (usize, usize, usize, f64, usize)> =
        std::collections::BTreeMap::new();

    for (id, created) in &sessions {
        if *created < cutoff {
            continue;
        }
        let date_key = created.format("%Y-%m-%d").to_string();

        match zeus_session::Session::load(sessions_dir, id).await {
            Ok(session) => {
                let usage = zeus_session::count_session_tokens(&session.messages);
                let cost = estimator.estimate_from_usage(&usage, model);
                let entry = daily.entry(date_key).or_insert((0, 0, 0, 0.0, 0));
                entry.0 += usage.input_tokens;
                entry.1 += usage.output_tokens;
                entry.2 += usage.total_tokens;
                entry.3 += cost.total_cost;
                entry.4 += 1; // session count
            }
            Err(_) => continue,
        }
    }

    let daily_data: Vec<Value> = daily
        .into_iter()
        .map(|(date, (input, output, total, cost, sessions))| {
            json!({
                "date": date,
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": total,
                "estimated_cost": cost,
                "session_count": sessions,
            })
        })
        .collect();

    Ok(Json(json!({
        "days": days,
        "model": model,
        "daily": daily_data,
    })))
}

/// GET /v1/analytics/models — Per-model usage from CostRouter
///
/// Returns token counts, request counts, and costs for each model
/// that has been used during the current billing period.
pub async fn analytics_models(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let tokens = state.cost_router.model_tokens();
    let requests = state.cost_router.model_requests();
    let estimator = zeus_session::CostEstimator::with_defaults();

    let mut models: Vec<Value> = tokens
        .iter()
        .map(|(model, (input, output))| {
            let req_count = requests.get(model).copied().unwrap_or(0);
            let cost = estimator.estimate(*input as usize, *output as usize, model);
            json!({
                "model": model,
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": input + output,
                "requests": req_count,
                "estimated_cost": cost.total_cost,
                "avg_tokens_per_request": if req_count > 0 {
                    (input + output) / req_count
                } else {
                    0
                },
            })
        })
        .collect();

    // Sort by total tokens descending
    models.sort_by(|a, b| {
        let a_tokens = a["total_tokens"].as_u64().unwrap_or(0);
        let b_tokens = b["total_tokens"].as_u64().unwrap_or(0);
        b_tokens.cmp(&a_tokens)
    });

    Json(json!({
        "models": models,
        "total_models": models.len(),
        "period_start": state.cost_router.summary().period_start,
    }))
}
