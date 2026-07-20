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

/// POST /v1/daemon/install — Install Zeus as a system service.
///
/// #383: WebUI onboarding parity with TUI's service picker. The TUI calls
/// `zeus daemon install` as a subprocess; this endpoint does the same from
/// the API. The `service_id` field selects the install method:
/// "launchd" (macOS), "systemd" (Linux), "rcd" (FreeBSD), "schtasks" (Windows),
/// "manual" (no-op — user starts zeus manually).
///
/// Returns the install path and status. Non-fatal failures (e.g., need sudo)
/// return success=false with guidance rather than an HTTP error.
#[derive(Debug, serde::Deserialize)]
pub struct DaemonInstallRequest {
    /// Service id: "launchd" | "systemd" | "rcd" | "schtasks" | "manual"
    pub service_id: String,
}

pub async fn daemon_install(
    State(shared): State<SharedState>,
    Json(req): Json<DaemonInstallRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    use tracing::warn;

    // "manual" = no service install needed
    if req.service_id == "manual" {
        return Ok(Json(json!({
            "success": true,
            "service_id": "manual",
            "message": "Manual start — no service installed. Run `zeus gateway` to start.",
            "path": null,
        })));
    }

    // Persist service_id to config so the gateway knows the install mode
    {
        let mut state = shared.write().await;
        state.config.gateway.get_or_insert_with(Default::default).service_id =
            Some(req.service_id.clone());
        if let Err(e) = state.config.save() {
            warn!("daemon_install: config save failed: {e}");
        }
    }

    // Run `zeus daemon install` as a subprocess (same as TUI awaken.rs)
    let exe = std::env::current_exe()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Cannot find zeus binary: {e}")))?;

    let result = std::process::Command::new(&exe)
        .args(["daemon", "install"])
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if output.status.success() {
                info!("daemon_install: service installed ({})", req.service_id);
                Ok(Json(json!({
                    "success": true,
                    "service_id": req.service_id,
                    "message": stdout,
                    "path": service_install_path(&req.service_id),
                })))
            } else {
                // Non-fatal: service install may need sudo or platform support
                let guidance = match req.service_id.as_str() {
                    "launchd" => format!("{stdout} {stderr} — run `sudo zeus daemon install` to enable boot persistence"),
                    "systemd" => format!("{stdout} {stderr} — run `systemctl --user daemon-reload` after install"),
                    "rcd" => format!("{stdout} {stderr} — run `sudo service zeus_gateway enable` to enable at boot"),
                    "schtasks" => format!("{stdout} {stderr} — Task Scheduler may require elevation"),
                    _ => format!("{stdout} {stderr}"),
                };
                Ok(Json(json!({
                    "success": false,
                    "service_id": req.service_id,
                    "message": guidance,
                    "path": service_install_path(&req.service_id),
                })))
            }
        }
        Err(e) => {
            warn!("daemon_install: failed to spawn `zeus daemon install`: {e}");
            Ok(Json(json!({
                "success": false,
                "service_id": req.service_id,
                "message": format!("Could not run `zeus daemon install`: {e}"),
                "path": service_install_path(&req.service_id),
            })))
        }
    }
}

/// Return the expected install path for a given service id.
fn service_install_path(service_id: &str) -> Option<&'static str> {
    match service_id {
        "launchd" => Some("~/Library/LaunchAgents/ai.zeuslab.gateway.plist"),
        "systemd" => Some("/etc/systemd/system/zeus-gateway.service"),
        "rcd" => Some("/usr/local/etc/rc.d/zeus_gateway"),
        "schtasks" => Some("Task Scheduler: ZeusGateway"),
        "manual" => None,
        _ => None,
    }
}
