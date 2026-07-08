//! Web Frontend Server — serves the Leptos/WASM SPA on a separate port.

use anyhow::Result;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Attempt to start the web frontend server. Returns a task handle if successful.
pub async fn spawn_web_server(
    host: &str,
    web_port: u16,
    web_dist_config: Option<&str>,
    api_router: axum::Router,
    shutdown_token: CancellationToken,
) -> Option<JoinHandle<Result<()>>> {
    let web_dist = web_dist_config
        .map(|p| {
            // Expand ~ to home directory
            if let Some(stripped) = p.strip_prefix("~/") {
                dirs::home_dir().unwrap_or_default().join(stripped)
            } else {
                std::path::PathBuf::from(p)
            }
        })
        .or_else(|| {
            // Auto-detect: check repo dist/ and ~/.zeus/web/
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(exe) = std::env::current_exe()
                && let Some(bin_dir) = exe.parent()
            {
                for ancestor in bin_dir.ancestors().take(5) {
                    let candidate = ancestor.join("apps/ZeusWeb/dist");
                    candidates.push(candidate);
                }
            }
            if let Some(home) = dirs::home_dir() {
                candidates.push(home.join(".zeus/web"));
            }
            candidates.push(std::path::PathBuf::from("apps/ZeusWeb/dist"));
            candidates
                .into_iter()
                .find(|p| p.join("index.html").exists())
        });

    let dist_path = match web_dist {
        Some(p) if p.join("index.html").exists() => p,
        Some(p) => {
            warn!(
                "web_dist path {} missing index.html — web UI disabled",
                p.display()
            );
            return None;
        }
        None => {
            warn!("No WebUI files found. To enable WebUI, run: ./scripts/install.sh --with-webui");
            warn!("Or manually: cd apps/ZeusWeb && trunk build --release && cp -r dist/* ~/.zeus/web/");
            return None;
        }
    };

    let web_addr = format!("{}:{}", host, web_port);
    match tokio::net::TcpListener::bind(&web_addr).await {
        Ok(listener) => {
            info!(
                "Web UI serving from {} on http://{}",
                dist_path.display(),
                web_addr
            );
            // Mount the gateway's API router (same routes as the :8080 API
            // server, built from the same AppState) and fall back to the
            // static SPA dist for any non-API path. This makes the WebUI port
            // serve SPA + API same-origin, so the WASM client's relative
            // /v1/... calls resolve here instead of 405'ing against a
            // static-only server.
            //
            // #212: the API router registers `GET /` (health), which shadowed
            // the SPA index on this port — axum's fallback only catches
            // *unmatched* routes. Wrap in an outer router where `/` serves
            // index.html explicitly; everything else falls through to the
            // API router, then static assets, then the SPA index (deep
            // links). API health stays reachable at /health here and at `/`
            // on the API port.
            let spa_index =
                tower_http::services::ServeFile::new(dist_path.join("index.html"));
            let api_with_static = api_router.fallback_service(
                tower_http::services::ServeDir::new(&dist_path)
                    .fallback(spa_index.clone()),
            );
            let web_router = axum::Router::new()
                .route_service("/", spa_index)
                .fallback_service(api_with_static);
            Some(tokio::spawn(async move {
                axum::serve(
                    listener,
                    web_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                        shutdown_token.cancelled().await
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Web server error: {}", e))
            }))
        }
        Err(e) => {
            warn!("Could not bind web UI on {}: {}", web_addr, e);
            None
        }
    }
}
