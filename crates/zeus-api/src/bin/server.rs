//! Zeus API Server Binary
//!
//! REST API server for programmatic access to Zeus

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use zeus_api::{ApiConfig, AppState, InboundConfig, create_router_with_auth, start_inbound_loop};
use zeus_core::Config;
use zeus_memory::Workspace;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zeus_api=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse command line args
    let args: Vec<String> = std::env::args().collect();

    let mut api_config = ApiConfig::default();

    // Simple arg parsing
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    api_config.port = args[i + 1].parse().unwrap_or(8080);
                    i += 1;
                }
            }
            "--host" | "-h" => {
                if i + 1 < args.len() {
                    api_config.host = args[i + 1].clone();
                    i += 1;
                }
            }
            "--no-cors" => {
                api_config.cors = false;
            }
            "--no-rate-limit" => {
                api_config.rate_limit = None;
            }
            "--help" => {
                print_help();
                return Ok(());
            }
            "--version" | "-V" => {
                println!("zeus-api {}", zeus_api::VERSION);
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                print_help();
                return Ok(());
            }
        }
        i += 1;
    }

    // Load Zeus config
    let config = Config::load().unwrap_or_default();

    // Initialize workspace
    let workspace = Workspace::from_config(&config);
    workspace.init().await?;

    // Create shared state
    let state = Arc::new(RwLock::new(
        AppState::new(config).expect("AppState initialization failed"),
    ));

    // Initialize key rotation if auth token is configured
    if let Some(ref token) = api_config.auth_token {
        let mut s = state.write().await;
        s.key_rotation = Some(zeus_api::middleware::KeyRotation::new(token.clone()));
    }

    // Start inbound channel message processor if channels are configured
    let has_channels = {
        let s = state.read().await;
        s.config.channels.is_some()
    };
    let _inbound_handle = if has_channels {
        let channel_mgr = {
            let s = state.read().await;
            s.channel_manager.clone()
        };
        // Start adapters
        if let Err(e) = channel_mgr.start_all().await {
            tracing::warn!("Failed to start channel adapters: {}", e);
        }
        let handle = start_inbound_loop(state.clone(), channel_mgr, InboundConfig::default()).await;
        tracing::info!("Inbound channel processor started");
        Some(handle)
    } else {
        None
    };

    // Auto-join Star Office Pantheon room if configured
    {
        let s = state.read().await;
        if let Some(ref star_office) = s.config.star_office {
            if let Some(ref room_id) = star_office.room_id {
                let room_id = room_id.clone();
                let gateway_url = s.config.gateway.as_ref()
                    .map(|g| format!("http://{}:{}", if g.host == "0.0.0.0" { "127.0.0.1" } else { &g.host }, g.port))
                    .unwrap_or_else(|| "http://localhost:8080".to_string());
                let agent_id = "zeus".to_string();
                drop(s);
                tracing::info!("Auto-joining Star Office Pantheon room: {}", room_id);
                let client = reqwest::Client::new();
                match client
                    .post(format!("{}/v1/office/join", gateway_url))
                    .json(&serde_json::json!({ "agentId": agent_id, "roomId": room_id }))
                    .send()
                    .await
                {
                    Ok(_) => tracing::info!("Joined Star Office room {}", room_id),
                    Err(e) => tracing::warn!("Failed to auto-join Star Office room: {}", e),
                }
            }
        }
    }

    // Create router with rate limiting enabled by default
    let router = create_router_with_auth(
        state,
        api_config.cors,
        api_config.auth_token,
        &api_config.allowed_origins,
        api_config.rate_limit,
    );

    // Start server
    let addr: SocketAddr = format!("{}:{}", api_config.host, api_config.port).parse()?;

    tracing::info!("Zeus API server listening on http://{}", addr);
    tracing::info!("API docs: http://{}/", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Setup graceful shutdown
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
            tracing::info!("Shutting down...");
        })
        .await?;

    Ok(())
}

fn print_help() {
    println!(
        r#"zeus-api - REST API Server for Zeus

USAGE:
    zeus-api [OPTIONS]

OPTIONS:
    -p, --port <PORT>     Port to listen on (default: 8080)
    -h, --host <HOST>     Host to bind to (default: 127.0.0.1)
        --no-cors         Disable CORS headers
        --no-rate-limit   Disable rate limiting (enabled by default)
        --help            Print help information
    -V, --version         Print version information

RATE LIMITING:
    Rate limiting is enabled by default with the following limits:
    - Global: 120 requests/minute per IP
    - LLM endpoints: 20 requests/minute per IP
    - Health endpoints (/ and /health) are exempt
    - LLM endpoints: /v1/chat, /v1/chat/completions, /v1/tools/*,
                     /v1/sandbox/execute, /v1/tts/*, /v1/images/generate

ENDPOINTS:
    GET  /health              Health check
    GET  /v1/status           Server status

    POST /v1/chat             Send message to agent
      Body: {{"message": "...", "session_id": "..."}}

    GET  /v1/sessions         List all sessions
    POST /v1/sessions         Create new session
    GET  /v1/sessions/:id     Get session details

    GET  /v1/tools            List available tools
    POST /v1/tools/:name      Execute a tool
      Body: {{"arguments": {{...}}}}

    GET  /v1/memory           Get memory context
    POST /v1/memory/remember  Add fact to memory
      Body: {{"fact": "..."}}
    POST /v1/memory/note      Add daily note
      Body: {{"content": "..."}}

    GET  /v1/webhooks         Webhook health check
    POST /v1/webhooks         Receive inbound webhook
      Body: {{"message": "...", "source": "...", "sender": "..."}}
    POST /v1/webhooks/:source Receive from specific source
      Body: {{"message": "...", "sender": "..."}}

EXAMPLES:
    zeus-api                           # Start with defaults
    zeus-api --port 3000               # Custom port

    # Chat with agent
    curl -X POST http://localhost:8080/v1/chat \
      -H "Content-Type: application/json" \
      -d '{{"message": "Hello"}}'

    # Execute a tool
    curl -X POST http://localhost:8080/v1/tools/list_dir \
      -H "Content-Type: application/json" \
      -d '{{"arguments": {{"path": "."}}}}'
"#
    );
}
