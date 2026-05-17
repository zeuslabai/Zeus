//! LLM Council API handlers

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::AppState;

#[derive(Deserialize)]
pub struct CouncilRequest {
    pub query: String,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub chairman: Option<String>,
}

#[derive(Serialize)]
pub struct CouncilResponse {
    pub final_answer: String,
    pub chairman: String,
    pub models: Vec<String>,
    pub responses: Vec<CouncilModelSummary>,
}

#[derive(Serialize)]
pub struct CouncilModelSummary {
    pub model_id: String,
    pub label: String,
    pub response: String,
    pub latency_ms: u64,
}

/// POST /v1/council/query — run a multi-model council deliberation
pub async fn council_query(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<CouncilRequest>,
) -> Result<Json<CouncilResponse>, (StatusCode, String)> {
    info!("Council query: {} chars", req.query.len());

    // Read council config from config.toml, fall back to defaults
    let mut config = {
        let sg = state.read().await;
        if let Some(ref cc) = sg.config.council {
            zeus_council::CouncilConfig {
                models: cc.models.clone(),
                chairman: cc.chairman.clone(),
                timeout_secs: cc.timeout_secs,
            }
        } else {
            zeus_council::CouncilConfig::default()
        }
    };
    if let Some(models) = req.models {
        config.models = models;
    }
    if let Some(chairman) = req.chairman {
        config.chairman = chairman;
    }

    let chairman = config.chairman.clone();
    let models = config.models.clone();

    match zeus_council::pipeline::run_council(&req.query, config).await {
        Ok(result) => {
            let responses: Vec<CouncilModelSummary> = result.session.results.iter().map(|r| {
                CouncilModelSummary {
                    model_id: r.model_id.clone(),
                    label: r.label.clone(),
                    response: r.response.clone(),
                    latency_ms: r.latency_ms,
                }
            }).collect();

            Ok(Json(CouncilResponse {
                final_answer: result.final_answer,
                chairman,
                models,
                responses,
            }))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
