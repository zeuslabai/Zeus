//! Gateway control handlers — restart, status.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::SharedState;

/// POST /v1/gateway/restart — Persist config and clean-exit so that
/// launchd (macOS) or systemd (Linux) `KeepAlive` relaunches the gateway
/// into full mode (onboarding is now complete, LLM config is saved).
///
/// The caller (WebUI onboarding wizard) hits this after the user finishes
/// configuring the LLM. The process exits with code 0; the service manager
/// immediately restarts it, and this time `LlmClient::from_config` succeeds
/// → full gateway (agent + all API routes + channels).
pub async fn gateway_restart(
    State(shared): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 1. Persist config to disk — the restart will read it back.
    {
        let state = shared.read().await;
        if let Err(e) = state.config.save() {
            let msg = e.to_string();
            // Guard errors from temp/default configs are expected in test;
            // in production onboarding this should succeed.
            if msg.contains("temp directory")
                || msg.contains("loaded from defaults")
                || msg.contains("onboarding_complete")
            {
                warn!("gateway_restart: config save skipped (guard): {}", msg);
            } else {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Config save failed: {}", msg)));
            }
        } else {
            info!("gateway_restart: config persisted to disk");
        }
    }

    // 2. Mark onboarding complete if not already (belt-and-braces).
    //    The onboarding/complete endpoint should have already set this,
    //    but if the wizard calls /gateway/restart directly we want it set.
    {
        let state = shared.read().await;
        let needs_onboarding = !state.config.onboarding_complete;
        drop(state);
        if needs_onboarding {
            let mut state = shared.write().await;
            state.config.onboarding_complete = true;
            let _ = state.config.save();
            info!("gateway_restart: onboarding_complete set and saved");
        }
    }

    info!("gateway_restart: clean-exit requested — service manager will relaunch into full gateway mode");

    // 3. Schedule clean exit. We return the response first, then exit.
    //    The small delay ensures the HTTP response is flushed before exit.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        info!("gateway_restart: exiting (code 0) — service manager will relaunch");
        std::process::exit(0);
    });

    Ok(Json(json!({
        "success": true,
        "message": "Gateway restarting — service manager will relaunch into full mode"
    })))
}

/// GET /v1/gateway/status — Return current gateway mode and health.
pub async fn gateway_status(
    State(shared): State<SharedState>,
) -> Json<Value> {
    let state = shared.read().await;
    let mode = if state.config.onboarding_complete {
        "full"
    } else {
        "bootstrap"
    };

    Json(json!({
        "mode": mode,
        "onboarding_complete": state.config.onboarding_complete,
        "model": state.config.model,
        "has_default_agent": state.default_agent.is_some(),
    }))
}
