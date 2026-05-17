//! Local HTTP callback server for OAuth redirect capture.
//!
//! Starts a temporary HTTP server on 127.0.0.1:1455 to capture the
//! authorization code from the OAuth provider's redirect.

use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// The captured authorization response from the OAuth callback.
#[derive(Debug, Clone)]
pub struct AuthCallback {
    /// The authorization code to exchange for tokens.
    pub code: String,
    /// The state parameter (must match the one sent in the auth request).
    pub state: String,
}

/// Start a local HTTP server to capture the OAuth callback.
///
/// Returns a oneshot receiver that resolves when the callback is received.
/// The server automatically shuts down after receiving one request.
///
/// Listens on `127.0.0.1:{port}` (default 1455).
pub async fn start_callback_server(
    port: u16,
) -> anyhow::Result<oneshot::Receiver<AuthCallback>> {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    info!("OAuth callback server listening on 127.0.0.1:{}", port);

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = stream;
            // 16KB buffer — OAuth callbacks are small but we need margin for
            // long state params and authorization codes
            let mut buf = vec![0u8; 16384];
            let n = match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                stream.read(&mut buf),
            ).await {
                Ok(Ok(n)) => n,
                _ => 0,
            };
            if n == 0 { return; }
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            // Parse the GET request for code and state parameters
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");

            let params: std::collections::HashMap<String, String> = url::Url::parse(
                &format!("http://localhost{}", path),
            )
            .map(|u| u.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect())
            .unwrap_or_default();

            let code = params.get("code").cloned().unwrap_or_default();
            let state = params.get("state").cloned().unwrap_or_default();

            // Send success response to browser
            let html = if !code.is_empty() {
                "<html><body><h1>Authorization successful!</h1><p>You can close this window and return to Zeus.</p></body></html>"
            } else {
                "<html><body><h1>Authorization failed</h1><p>No authorization code received.</p></body></html>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;

            if !code.is_empty() {
                info!("OAuth callback received authorization code");
                let mut guard = tx.lock().await;
                if let Some(sender) = guard.take() {
                    let _ = sender.send(AuthCallback { code, state });
                }
            } else {
                warn!("OAuth callback received no authorization code");
            }
        }
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn callback_server_starts() {
        let rx = start_callback_server(0).await;
        assert!(rx.is_ok(), "Callback server should start on ephemeral port");
    }
}
