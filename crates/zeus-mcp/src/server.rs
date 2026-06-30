//! MCP HTTP Server
//!
//! Axum-based HTTP server for MCP protocol.
//! Supports three transports:
//! - **HTTP POST** (`/mcp`) — plain JSON-RPC request/response
//! - **SSE** (`/sse`) — MCP SSE transport for Claude Code integration
//! - **Stdio** — via `McpStdio::run()` for `zeus mcp` CLI mode

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use zeus_memory::Workspace;

use crate::handlers::ToolHandler;
use crate::protocol::{McpRequest, McpResponse};

/// MCP Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Host to bind to
    pub host: String,
    /// Port to bind to
    pub port: u16,
    /// Enable CORS for browser access
    pub cors: bool,
    /// Workspace path (optional)
    pub workspace: Option<String>,
    /// Bearer token for authentication. If None, auth is disabled.
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: crate::DEFAULT_PORT,
            cors: true,
            workspace: None,
            auth_token: None,
        }
    }
}

/// MCP Server state
struct ServerState {
    handler: ToolHandler,
    /// Active SSE sessions: session_id → sender for pushing events to client
    sse_sessions: HashMap<String, mpsc::Sender<McpResponse>>,
}

/// MCP Server
pub struct McpServer {
    config: McpConfig,
    state: Arc<RwLock<ServerState>>,
}

impl McpServer {
    /// Create a new MCP server with default configuration
    pub fn new() -> Self {
        Self::with_config(McpConfig::default())
    }

    /// Create a new MCP server with custom configuration
    pub fn with_config(config: McpConfig) -> Self {
        let handler = if let Some(ref workspace_path) = config.workspace {
            let workspace = Workspace::new(workspace_path);
            ToolHandler::with_workspace(workspace)
        } else {
            ToolHandler::new()
        };

        Self {
            config,
            state: Arc::new(RwLock::new(ServerState {
                handler,
                sse_sessions: HashMap::new(),
            })),
        }
    }

    /// Create server with workspace
    pub fn with_workspace(workspace: Workspace) -> Self {
        let config = McpConfig::default();
        let handler = ToolHandler::with_workspace(workspace);
        Self {
            config,
            state: Arc::new(RwLock::new(ServerState {
                handler,
                sse_sessions: HashMap::new(),
            })),
        }
    }

    /// Create server with full Zeus config — registers Talos tools and optional agent manager
    pub fn with_full_config(
        mcp_config: McpConfig,
        zeus_config: &zeus_core::Config,
        channels: Option<Arc<zeus_channels::ChannelManager>>,
    ) -> Self {
        let mcp_server_cfg = zeus_config.mcp_server.clone().unwrap_or_default();

        // Build tool registry with optional Talos tools
        let mut registry = if mcp_server_cfg.enable_talos {
            zeus_agent::ToolRegistry::with_talos(zeus_talos::TalosRegistry::with_defaults())
        } else {
            zeus_agent::ToolRegistry::with_defaults()
        };

        // Surface D(a): wire the shared ChannelManager into the MCP-served
        // ToolRegistry so the `message` tool can dispatch to platform adapters
        // (Discord/Telegram/Slack/X/etc.) when invoked over MCP HTTP/SSE.
        // Mirrors Cut D-real (229dbce2) and Surface E on the agent-loop side.
        if let Some(ref ch) = channels {
            registry.set_channels(ch.clone());
        }

        let workspace = mcp_config.workspace.as_ref().map(Workspace::new);

        let mut handler = ToolHandler::with_registry(registry, workspace);

        // Optionally attach agent manager
        if mcp_server_cfg.enable_agents {
            let mgr = crate::agents::McpAgentManager::new(zeus_config.clone());
            let mgr = Arc::new(tokio::sync::Mutex::new(mgr));
            handler = handler.with_agents(mgr);
        }

        // Optionally attach Mnemosyne memory store for graph tools
        if mcp_server_cfg.enable_mnemosyne
            && let Some(ref mn_cfg) = zeus_config.mnemosyne
        {
            match zeus_mnemosyne::MemoryStore::new(&mn_cfg.db_path, mn_cfg.enable_fts, false) {
                Ok(store) => {
                    let store = Arc::new(std::sync::Mutex::new(store));
                    handler = handler.with_mnemosyne(store);
                    info!("MCP: Mnemosyne graph memory tools enabled");
                }
                Err(e) => {
                    warn!(
                        "MCP: Failed to open Mnemosyne DB, graph tools disabled: {}",
                        e
                    );
                }
            }
        }

        // Use auth_token from McpServerConfig if not already set in McpConfig
        let mut config = mcp_config;
        if config.auth_token.is_none() {
            config.auth_token = mcp_server_cfg.auth_token;
        }

        Self {
            config,
            state: Arc::new(RwLock::new(ServerState {
                handler,
                sse_sessions: HashMap::new(),
            })),
        }
    }

    /// Get the server address
    pub fn address(&self) -> String {
        format!("{}:{}", self.config.host, self.config.port)
    }

    /// Build the router with auth and CORS
    fn build_router(&self) -> Router {
        let mut router = Router::new()
            .route("/", get(health_check))
            .route("/health", get(health_check))
            .route("/mcp", post(handle_mcp_request))
            .route("/mcp/batch", post(handle_mcp_batch))
            .route("/v1/mcp", post(handle_mcp_request))
            .route("/v1/mcp/batch", post(handle_mcp_batch))
            // SSE transport: client GETs /sse, server streams events
            .route("/sse", get(handle_sse_connect))
            // SSE message endpoint: client POSTs JSON-RPC here with ?sessionId=
            .route("/sse/message", post(handle_sse_message))
            .layer(TraceLayer::new_for_http())
            .with_state(self.state.clone());

        // Add bearer token auth middleware if configured
        if let Some(ref token) = self.config.auth_token {
            let token = token.clone();
            router = router.layer(middleware::from_fn(move |req, next| {
                let token = token.clone();
                mcp_auth_middleware(req, next, token)
            }));
        }

        if self.config.cors {
            let cors_layer = CorsLayer::new()
                .allow_origin(AllowOrigin::list([
                    HeaderValue::from_static("http://127.0.0.1"),
                    HeaderValue::from_static("http://localhost"),
                ]))
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
            router = router.layer(cors_layer);
        }

        router
    }

    /// Run the server
    pub async fn run(&self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.address().parse()?;
        let router = self.build_router();

        info!("Zeus MCP server listening on http://{}", addr);
        info!("MCP endpoint: http://{}/mcp", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }

    /// Run the server with graceful shutdown
    pub async fn run_with_shutdown(
        &self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let addr: SocketAddr = self.address().parse()?;
        let router = self.build_router();

        info!("Zeus MCP server listening on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;

        Ok(())
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Health check endpoint
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "server": "zeus-mcp",
        "version": crate::VERSION
    }))
}

/// Handle MCP JSON-RPC request
async fn handle_mcp_request(
    State(state): State<Arc<RwLock<ServerState>>>,
    Json(request): Json<McpRequest>,
) -> Result<Json<McpResponse>, StatusCode> {
    let state = state.read().await;
    let response = state.handler.handle(request).await;
    Ok(Json(response))
}

/// Bearer token authentication middleware for MCP server
async fn mcp_auth_middleware(
    req: Request,
    next: Next,
    expected_token: String,
) -> Result<Response, StatusCode> {
    // Allow health check endpoints without auth
    let path = req.uri().path();
    if path == "/" || path == "/health" {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") => {
            let token = &value[7..];
            if token == expected_token {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Handle batch MCP requests
async fn handle_mcp_batch(
    State(state): State<Arc<RwLock<ServerState>>>,
    Json(requests): Json<Vec<McpRequest>>,
) -> Result<Json<Vec<McpResponse>>, StatusCode> {
    let state = state.read().await;
    let mut responses = Vec::new();

    for request in requests {
        let response = state.handler.handle(request).await;
        responses.push(response);
    }

    Ok(Json(responses))
}

// ─── SSE Transport (MCP SSE spec) ──────────────────────────────────────────

/// SSE connect endpoint.
/// Client GETs /sse → server responds with SSE stream.
/// First event is `endpoint` with the POST URL for sending messages.
async fn handle_sse_connect(
    State(state): State<Arc<RwLock<ServerState>>>,
) -> Sse<ReceiverStream<Result<Event, std::convert::Infallible>>> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    // Create a channel for MCP responses that will be forwarded as SSE events
    let (resp_tx, mut resp_rx) = mpsc::channel::<McpResponse>(64);

    // Register the session
    {
        let mut st = state.write().await;
        st.sse_sessions.insert(session_id.clone(), resp_tx);
    }

    info!("SSE session connected: {}", session_id);

    // Send the endpoint event telling the client where to POST messages
    let endpoint_url = format!("/sse/message?sessionId={}", session_id);
    let _ = tx
        .send(Ok(Event::default().event("endpoint").data(endpoint_url)))
        .await;

    // Spawn a task that forwards MCP responses → SSE events
    let tx_clone = tx.clone();
    let session_id_clone = session_id.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(response) = resp_rx.recv().await {
            let json = match serde_json::to_string(&response) {
                Ok(j) => j,
                Err(e) => {
                    warn!("Failed to serialize MCP response: {}", e);
                    continue;
                }
            };
            if tx_clone
                .send(Ok(Event::default().event("message").data(json)))
                .await
                .is_err()
            {
                break; // Client disconnected
            }
        }
        // Clean up session on disconnect
        let mut st = state_clone.write().await;
        st.sse_sessions.remove(&session_id_clone);
        info!("SSE session disconnected: {}", session_id_clone);
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// SSE message query params
#[derive(Deserialize)]
struct SseMessageParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// SSE message endpoint.
/// Client POSTs JSON-RPC to /sse/message?sessionId=<id>
/// Response is pushed back via the SSE stream as a `message` event.
async fn handle_sse_message(
    State(state): State<Arc<RwLock<ServerState>>>,
    Query(params): Query<SseMessageParams>,
    Json(request): Json<McpRequest>,
) -> StatusCode {
    // Look up the session's response sender
    let (handler_response, resp_tx) = {
        let st = state.read().await;
        let resp_tx = match st.sse_sessions.get(&params.session_id) {
            Some(tx) => tx.clone(),
            None => {
                warn!("SSE message for unknown session: {}", params.session_id);
                return StatusCode::NOT_FOUND;
            }
        };
        let response = st.handler.handle(request).await;
        (response, resp_tx)
    };

    // Send response via SSE stream
    if resp_tx.send(handler_response).await.is_err() {
        warn!("SSE session {} disconnected during send", params.session_id);
        return StatusCode::GONE;
    }

    StatusCode::ACCEPTED
}

// ─── Stdio Transport ───────────────────────────────────────────────────────

/// Stdio MCP transport for `zeus mcp` CLI mode.
/// Reads JSON-RPC lines from stdin, writes responses to stdout.
pub struct McpStdio;

impl McpStdio {
    /// Run the stdio MCP server with full Zeus config (all tools including Talos).
    ///
    /// Architecture: async stdin reader → mpsc → concurrent handler tasks → mpsc → stdout writer.
    /// This prevents slow tool executions from blocking stdin reads, which caused
    /// Claude Code to consider the MCP server dead and disconnect.
    pub async fn run(config: &zeus_core::Config) -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Arc;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        // Log PID + version on start for crash traceability.
        eprintln!(
            "[zeus-mcp] start pid={} version={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        );

        let mcp_server_cfg = config.mcp_server.clone().unwrap_or_default();

        // Build tool registry
        let registry = if mcp_server_cfg.enable_talos {
            zeus_agent::ToolRegistry::with_talos(zeus_talos::TalosRegistry::with_defaults())
        } else {
            zeus_agent::ToolRegistry::with_defaults()
        };

        let workspace_path = config.workspace.display().to_string();
        let workspace = Workspace::new(&workspace_path);

        let mut handler = ToolHandler::with_registry(registry, Some(workspace));

        // Optionally attach agent manager
        if mcp_server_cfg.enable_agents {
            let mgr = crate::agents::McpAgentManager::new(config.clone());
            let mgr = Arc::new(tokio::sync::Mutex::new(mgr));
            handler = handler.with_agents(mgr);
        }

        // Optionally attach Mnemosyne memory store for graph tools
        if mcp_server_cfg.enable_mnemosyne
            && let Some(ref mn_cfg) = config.mnemosyne
            && let Ok(store) =
                zeus_mnemosyne::MemoryStore::new(&mn_cfg.db_path, mn_cfg.enable_fts, false)
        {
            let store = Arc::new(std::sync::Mutex::new(store));
            handler = handler.with_mnemosyne(store);
        }

        let handler = Arc::new(handler);

        // Auto-initialize all services on MCP boot.
        // Runs in background so STDIO handshake isn't blocked.
        tokio::spawn(async {
            // Small delay to let MCP handshake complete first
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            use zeus_talos::TalosTool;

            // 1. Start all configured relays (Telegram + Discord)
            let relay_tool = zeus_talos::relay::AutoStartRelayTool;
            match relay_tool.execute(serde_json::json!({})).await {
                Ok(msg) => eprintln!("[zeus-mcp] Relays: {}", msg),
                Err(e) => eprintln!("[zeus-mcp] Relays failed: {}", e),
            }

            // 2. Load persistent memory context
            let memory_tool = zeus_talos::memory_tools::MemoryRecallTool;
            match memory_tool.execute(serde_json::json!({})).await {
                Ok(msg) => {
                    let lines = msg.lines().count();
                    eprintln!("[zeus-mcp] Memory loaded: {} lines", lines);
                }
                Err(e) => eprintln!("[zeus-mcp] Memory recall failed: {}", e),
            }

            eprintln!("[zeus-mcp] Boot sequence complete");
        });

        // Idle-timeout watchdog: if stdin stalls because the parent crashed without
        // closing the pipe, read_line() blocks forever.  This background thread
        // exits the process after 5 minutes of no traffic, ensuring we never
        // stay alive as an invisible zombie.  Exit code 1 distinguishes a timeout
        // from a normal stdin-close (exit 0).
        let now_secs = || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        };
        let last_msg_secs = Arc::new(AtomicU64::new(now_secs()));
        let watchdog_last = Arc::clone(&last_msg_secs);
        std::thread::spawn(move || {
            const IDLE_TIMEOUT_SECS: u64 = 31_536_000; // 365 days — the operator: "do not crash or kill the mcp" — 5min was too aggressive, killed server during idle conversation periods
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                let idle = now_secs().saturating_sub(watchdog_last.load(Ordering::Relaxed));
                if idle > IDLE_TIMEOUT_SECS {
                    eprintln!("[zeus-mcp] idle timeout ({}s) — exiting", idle);
                    std::process::exit(1);
                }
            }
        });

        // Channel for sending responses back to the stdout writer task.
        // Responses include the original request ID for ordering.
        let (resp_tx, mut resp_rx) = mpsc::channel::<String>(64);

        // Stdout writer task — serializes all output through a single writer
        // so concurrent handler tasks don't interleave JSON lines.
        let writer_handle = tokio::spawn(async move {
            let mut stdout = tokio::io::stdout();
            while let Some(json_line) = resp_rx.recv().await {
                if let Err(e) = stdout.write_all(json_line.as_bytes()).await {
                    eprintln!("[zeus-mcp] stdout write error: {}", e);
                    break;
                }
                if let Err(e) = stdout.write_all(b"\n").await {
                    eprintln!("[zeus-mcp] stdout write error: {}", e);
                    break;
                }
                if let Err(e) = stdout.flush().await {
                    eprintln!("[zeus-mcp] stdout flush error: {}", e);
                    break;
                }
            }
        });

        // Async stdin reader — never blocks on tool execution
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(_) => break, // read error
            };

            if bytes_read == 0 {
                break; // EOF — client disconnected
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Heartbeat: reset idle watchdog on every non-empty message.
            last_msg_secs.store(now_secs(), Ordering::Relaxed);

            let request: McpRequest = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    let error_resp =
                        McpResponse::error(None, crate::protocol::McpError::parse_error());
                    if let Ok(json) = serde_json::to_string(&error_resp) {
                        let _ = resp_tx.send(json).await;
                    }
                    eprintln!("Parse error: {}", e);
                    continue;
                }
            };

            // JSON-RPC notifications have no `id` — per spec, no response should be sent.
            // Silently acknowledge and continue.
            if request.id.is_none() {
                continue;
            }

            // Spawn each request handler concurrently so slow tools don't block
            // stdin reads. The response is sent through the channel to the
            // single stdout writer task.
            //
            // A keepalive task sends periodic `notifications/progress` while the
            // handler runs.  Claude Code's MCP client may time out if it
            // receives nothing for ~60 s; the 15-second heartbeat prevents that.
            let handler = Arc::clone(&handler);
            let tx = resp_tx.clone();
            tokio::spawn(async move {
                let req_id = request.id.clone();
                let is_tool_call = request.method == "tools/call";

                let done = Arc::new(AtomicBool::new(false));
                let done2 = Arc::clone(&done);

                // Progress keepalive — only for tool calls (which can take minutes)
                let keepalive_tx = tx.clone();
                let keepalive_handle = if is_tool_call {
                    Some(tokio::spawn(async move {
                        let mut tick = 0u64;
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            if done2.load(Ordering::Relaxed) {
                                break;
                            }
                            tick += 1;
                            // MCP progress notification (no id = notification, won't confuse client)
                            let note = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/progress",
                                "params": {
                                    "progressToken": req_id,
                                    "progress": tick,
                                    "total": 0
                                }
                            });
                            if let Ok(json) = serde_json::to_string(&note)
                                && keepalive_tx.send(json).await.is_err() {
                                    break;
                                }
                        }
                    }))
                } else {
                    None
                };

                let response = handler.handle(request).await;
                done.store(true, Ordering::Relaxed);
                if let Some(h) = keepalive_handle {
                    h.abort();
                }
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = tx.send(json).await;
                }
            });
        }

        // stdin closed (Claude Code session ended) — force-exit so background tasks
        // (relay, memory recall, etc.) don't keep this process alive as an orphan.
        drop(resp_tx);
        let _ = writer_handle.await;
        eprintln!("[zeus-mcp] stdin closed — exiting");
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let cfg = McpConfig::default();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, crate::DEFAULT_PORT);
        assert!(cfg.cors);
        assert!(cfg.workspace.is_none());
        assert!(cfg.auth_token.is_none());
    }

    #[test]
    fn test_config_serialization() {
        let cfg = McpConfig {
            host: "0.0.0.0".to_string(),
            port: 4000,
            cors: false,
            workspace: Some("/var/db/zeus/workspace".to_string()),
            auth_token: Some("secret-token".to_string()),
        };
        let json = serde_json::to_string(&cfg).expect("should serialize to JSON");
        let de: McpConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.host, "0.0.0.0");
        assert_eq!(de.port, 4000);
        assert!(!de.cors);
        assert_eq!(de.workspace.as_deref(), Some("/var/db/zeus/workspace"));
        assert_eq!(de.auth_token.as_deref(), Some("secret-token"));
    }

    #[test]
    fn test_config_from_partial_json() {
        let json = r#"{"host":"localhost","port":5000,"cors":true}"#;
        let cfg: McpConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.port, 5000);
        assert!(cfg.auth_token.is_none());
    }

    #[test]
    fn test_server_address() {
        let server = McpServer::new();
        assert_eq!(
            server.address(),
            format!("127.0.0.1:{}", crate::DEFAULT_PORT)
        );
    }

    #[test]
    fn test_server_custom_address() {
        let cfg = McpConfig {
            host: "0.0.0.0".to_string(),
            port: 9999,
            ..Default::default()
        };
        let server = McpServer::with_config(cfg);
        assert_eq!(server.address(), "0.0.0.0:9999");
    }

    #[test]
    fn test_server_default() {
        let server = McpServer::default();
        assert_eq!(
            server.address(),
            format!("127.0.0.1:{}", crate::DEFAULT_PORT)
        );
    }

    #[test]
    fn test_server_with_workspace() {
        let ws = Workspace::new("/tmp/test-workspace");
        let server = McpServer::with_workspace(ws);
        assert_eq!(
            server.address(),
            format!("127.0.0.1:{}", crate::DEFAULT_PORT)
        );
    }

    #[test]
    fn test_server_builds_router() {
        let server = McpServer::new();
        let _router = server.build_router(); // should not panic
    }

    #[test]
    fn test_server_with_auth_builds_router() {
        let cfg = McpConfig {
            auth_token: Some("my-token".to_string()),
            ..Default::default()
        };
        let server = McpServer::with_config(cfg);
        let _router = server.build_router(); // should not panic
    }

    #[test]
    fn test_server_with_full_config_default() {
        let mcp_cfg = McpConfig::default();
        let zeus_cfg = zeus_core::Config::default();
        let server = McpServer::with_full_config(mcp_cfg, &zeus_cfg, None);
        assert_eq!(
            server.address(),
            format!("127.0.0.1:{}", crate::DEFAULT_PORT)
        );
        let _router = server.build_router(); // should not panic
    }

    #[test]
    fn test_server_with_full_config_talos_enabled() {
        let mcp_cfg = McpConfig::default();
        let mut zeus_cfg = zeus_core::Config::default();
        zeus_cfg.mcp_server = Some(zeus_core::McpServerConfig {
            enable_talos: true,
            ..Default::default()
        });
        let server = McpServer::with_full_config(mcp_cfg, &zeus_cfg, None);
        let _router = server.build_router();
    }

    #[test]
    fn test_server_with_full_config_agents_enabled() {
        let mcp_cfg = McpConfig::default();
        let mut zeus_cfg = zeus_core::Config::default();
        zeus_cfg.mcp_server = Some(zeus_core::McpServerConfig {
            enable_agents: true,
            ..Default::default()
        });
        let server = McpServer::with_full_config(mcp_cfg, &zeus_cfg, None);
        let _router = server.build_router();
    }

    #[test]
    fn test_server_with_full_config_auth_from_mcp_server() {
        let mcp_cfg = McpConfig::default(); // no auth_token
        let mut zeus_cfg = zeus_core::Config::default();
        zeus_cfg.mcp_server = Some(zeus_core::McpServerConfig {
            auth_token: Some("from-mcp-server".to_string()),
            ..Default::default()
        });
        let server = McpServer::with_full_config(mcp_cfg, &zeus_cfg, None);
        assert_eq!(server.config.auth_token.as_deref(), Some("from-mcp-server"));
    }
}
