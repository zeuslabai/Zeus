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
            let web_router = axum::Router::new().fallback_service(
                tower_http::services::ServeDir::new(&dist_path).fallback(
                    tower_http::services::ServeFile::new(dist_path.join("index.html")),
                ),
            );
            Some(tokio::spawn(async move {
                axum::serve(listener, web_router)
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
