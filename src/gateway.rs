//! Zeus Gateway - Unified daemon combining API, channels, and heartbeat
//!
//! Runs as a single process that exposes the HTTP API, connects to
//! messaging channels, and runs heartbeat/cron tasks.

use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info, info_span, warn};
use tracing::Instrument;
use zeus_core::{Config, GatewayConfig, TriggerExecutor};
use base64::Engine as _;
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_wallet::WalletKeypair;

/// RAII guard that cancels its token on drop. Used by the typing-indicator
/// helper so that any early return / `?` / panic path stops the heartbeat.
pub struct TypingHeartbeatGuard(tokio_util::sync::CancellationToken);
impl Drop for TypingHeartbeatGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Convert a `zeus_core::ChannelSource` (gateway-side) into the
/// `zeus_channels::ChannelSource` shape expected by `ChannelManager::send_typing`.
/// The two structs have different field names (`channel_id`/`sender_id` vs
/// `chat_id`/`user_id`) but represent the same logical entity.
/// Reverse of `core_source_to_channels` — used by dispatch paths where
/// `msg.source` is already a `zeus_channels::ChannelSource`.
fn channels_source_to_core(
    src: &zeus_channels::ChannelSource,
) -> zeus_core::ChannelSource {
    zeus_core::ChannelSource {
        channel_type: src.channel_type.clone(),
        channel_id: src.chat_id.clone(),
        channel_name: None,
        sender_name: None,
        sender_id: Some(src.user_id.clone()),
    }
}

fn core_source_to_channels(
    src: &zeus_core::ChannelSource,
) -> zeus_channels::ChannelSource {
    zeus_channels::ChannelSource {
        channel_type: src.channel_type.clone(),
        user_id: src.sender_id.clone().unwrap_or_default(),
        chat_id: src.channel_id.clone(),
        account_id: None,
        thread_id: None,
        reply_to_message_id: None,
        sender_type: zeus_core::SenderType::default(),
    }
}

/// Spawn a background task that sends typing indicators every 8 seconds
/// until the returned guard is dropped. Used to keep channels like Discord
/// and Telegram showing "typing..." during long-running cooking loops.
///
/// - Discord typing auto-expires ~10s, so 8s cadence keeps it continuous.
/// - Channels without typing support (IRC, etc.) silently no-op inside
///   `ChannelManager::send_typing` via their `supports_typing() == false`.
/// - First typing indicator is sent immediately so users see feedback fast.
pub fn spawn_typing_heartbeat(
    channels: Option<Arc<zeus_channels::ChannelManager>>,
    source: Option<zeus_core::ChannelSource>,
) -> TypingHeartbeatGuard {
    let stop = tokio_util::sync::CancellationToken::new();
    let guard = TypingHeartbeatGuard(stop.clone());
    let Some(channels) = channels else {
        return guard;
    };
    let Some(source) = source else {
        return guard;
    };
    let channel_source = core_source_to_channels(&source);
    tokio::spawn(async move {
        // Immediate first tick — don't wait 8s to show feedback.
        if let Err(e) = channels.send_typing(&channel_source).await {
            debug!("Typing indicator send failed (non-fatal): {}", e);
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(8));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // consume immediate tick
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(e) = channels.send_typing(&channel_source).await {
                        debug!("Typing indicator send failed (non-fatal): {}", e);
                    }
                }
            }
        }
    });
    guard
}

/// LLM-powered task detection — understands context instead of keyword matching.
/// Uses a lightweight LLM call to classify if a message is a task assignment.
/// Falls back to keyword detection if LLM call fails.
async fn detect_task_with_llm(
    llm: &zeus_llm::LlmClient,
    message: &str,
    agent_name: &str,
) -> Option<String> {
    // Pre-filter: must mention this agent (fast, zero-cost check)
    let msg_lower = message.to_lowercase();
    let name_lower = agent_name.to_lowercase();
    // #296: word-boundary `@name` so `@zeus` doesn't fire on `@zeus100`.
    let mentions_agent = crate::gateway_consumer::mentions_name_at(&msg_lower, &name_lower);
    if !mentions_agent {
        return None;
    }
    // Skip very short messages (acknowledgments, emoji reactions)
    if message.len() < 15 {
        return None;
    }

    let prompt = format!(
        "Analyze this Discord message directed at agent '{agent_name}'.\n\
        Message: \"{message}\"\n\n\
        Is this a TASK ASSIGNMENT (asking the agent to do work)?\n\
        If YES: reply with ONLY a 1-2 sentence task summary (no JSON, no preamble).\n\
        If NO (praise, chat, status update, acknowledgment): reply with exactly \"NOT_TASK\"."
    );

    // Lightweight LLM call — no tools, no history, just classification
    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        llm.complete(&[zeus_core::Message::user(&prompt)], &[], None),
    ).await {
        Ok(Ok(response)) => {
            let text = response.content.trim();
            if text == "NOT_TASK" || text.is_empty() || text.len() < 5 {
                debug!("LLM task detection: not a task assignment");
                None
            } else {
                info!("LLM detected task assignment: {}", &text[..text.len().min(80)]);
                Some(text.to_string())
            }
        }
        Ok(Err(e)) => {
            warn!("LLM task detection failed, falling back to keywords: {}", e);
            detect_task_assignment_keyword(message, agent_name)
        }
        Err(_) => {
            warn!("LLM task detection timed out, falling back to keywords");
            detect_task_assignment_keyword(message, agent_name)
        }
    }
}

/// Keyword-based task detection (fallback when LLM is unavailable).
fn detect_task_assignment_keyword(message: &str, agent_name: &str) -> Option<String> {
    let msg_lower = message.to_lowercase();
    let name_lower = agent_name.to_lowercase();
    // #296: word-boundary `@name` so `@zeus` doesn't fire on `@zeus100`.
    let mentions_agent = crate::gateway_consumer::mentions_name_at(&msg_lower, &name_lower);
    if !mentions_agent {
        return None;
    }
    let task_verbs = [
        "fix", "implement", "build", "add", "create", "ship", "push",
        "write", "update", "refactor", "audit", "review", "test",
        "deploy", "research", "design", "investigate", "check",
        "make", "attach", "setup", "configure", "migrate", "port",
        "your task", "assigned to you", "take this", "work on",
    ];
    let has_task_intent = task_verbs.iter().any(|v| msg_lower.contains(v));
    let has_technical = message.contains(".rs") || message.contains(".ts")
        || message.contains("crates/") || message.contains("src/")
        || msg_lower.contains("branch:") || msg_lower.contains("branch ");
    if !has_task_intent && !has_technical {
        return None;
    }
    Some(message.to_string())
}
use zeus_session::{ChannelSessionRouter, ContextScope, Session};

/// Names of channels enabled by config (with env-var auto-enable fallbacks),
/// for the boot banner. Mirrors the adapter-creation logic in
/// `build_channel_manager_from_config` without instantiating anything.
fn enabled_channel_names(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let env_set =
        |k: &str| -> bool { std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false) };
    let ch = config.channels.as_ref();
    let mut add = |name: &str, cfg_on: bool, env_on: bool| {
        if cfg_on || env_on {
            names.push(name.to_string());
        }
    };
    add(
        "discord",
        ch.map(|c| c.discord.is_some()).unwrap_or(false),
        env_set("DISCORD_BOT_TOKEN"),
    );
    add(
        "telegram",
        ch.map(|c| c.telegram.is_some()).unwrap_or(false),
        env_set("TELEGRAM_BOT_TOKEN"),
    );
    add(
        "slack",
        ch.map(|c| c.slack.is_some()).unwrap_or(false),
        env_set("SLACK_BOT_TOKEN"),
    );
    add("email", ch.map(|c| c.email.is_some()).unwrap_or(false), false);
    add(
        "whatsapp",
        ch.map(|c| c.whatsapp.is_some()).unwrap_or(false),
        false,
    );
    add("signal", ch.map(|c| c.signal.is_some()).unwrap_or(false), false);
    add("matrix", ch.map(|c| c.matrix.is_some()).unwrap_or(false), false);
    add("mqtt", ch.map(|c| c.mqtt.is_some()).unwrap_or(false), false);
    add(
        "mattermost",
        ch.map(|c| c.mattermost.is_some()).unwrap_or(false),
        false,
    );
    add("irc", ch.map(|c| c.irc.is_some()).unwrap_or(false), false);
    add("twitch", ch.map(|c| c.twitch.is_some()).unwrap_or(false), false);
    add(
        "x_twitter",
        ch.map(|c| c.x_twitter.is_some()).unwrap_or(false),
        false,
    );
    names
}

use crate::agent_executor::AgentToolExecutor;
use crate::gateway_lock::GatewayLock;

/// Run the unified gateway daemon
pub async fn run_gateway(mut config: Config, gateway: GatewayConfig) -> Result<()> {
    info!("Starting Zeus Gateway on {}:{}", gateway.host, gateway.port);

    // ── Boot banner (P2 observability) ──────────────────────────────────
    // One greppable INFO line with a stable `boot` target carrying the full
    // runtime identity: version + git sha + instance + config source +
    // enabled channels + all three ports. `zeus logs | grep boot` answers
    // "what exactly is running" without archaeology.
    {
        let instance = std::env::var("ZEUS_INSTANCE")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string());
        let config_source = if config.loaded_from_default {
            "builtin-default".to_string()
        } else {
            crate::zeus_paths::zeus_home()
                .join("config.toml")
                .display()
                .to_string()
        };
        let channels = if gateway.enable_channels {
            enabled_channel_names(&config).join(",")
        } else {
            "(disabled)".to_string()
        };
        info!(
            target: "boot",
            version = env!("CARGO_PKG_VERSION"),
            git_sha = env!("GIT_SHA"),
            instance = %instance,
            config_source = %config_source,
            channels = %channels,
            api_port = gateway.port,
            web_port = gateway.web_port,
            mcp_port = gateway.mcp_port,
            "gateway boot"
        );
    }

    // Sync the *effective* enable_channels (config.toml AND --no-channels CLI
    // flag, already folded into `gateway` by main.rs) into `config.gateway` so
    // downstream consumers of Config — Agent::with_subsystems →
    // build_channel_manager_from_config — see the truth. Without this,
    // `--no-channels` only gated the message consumer while the builder still
    // created live adapters from config/env (duplicate Discord sessions off
    // one inherited DISCORD_BOT_TOKEN under multi-instance).
    if let Some(g) = config.gateway.as_mut() {
        g.enable_channels = gateway.enable_channels;
    } else if !gateway.enable_channels {
        // Only materialize a [gateway] section when we need to carry a
        // non-default (disabled) flag — avoids changing `is_some()` behavior
        // elsewhere for the enabled default case.
        config.gateway = Some(GatewayConfig {
            enable_channels: false,
            ..Default::default()
        });
    }

    // Fleet channel ID — from config.toml [gateway].fleet_channel_id,
    // falling back to the first Discord binding's channel_id.
    // Never fall back to a hardcoded channel — it may be nuked.
    let fleet_ch_global = gateway.fleet_channel_id.clone()
        .or_else(|| {
            config.bindings.first()
                .and_then(|b| b.channel_id.clone())
        })
        .unwrap_or_default();

    // Acquire PID lock — prevents duplicate gateway processes.
    // Held for the lifetime of the gateway; Drop removes PID file on exit.
    let _gateway_lock = GatewayLock::acquire(gateway.port)?;

    // Log which config we're using (aids debugging config corruption issues)
    if config.loaded_from_default {
        warn!("Using DEFAULT config (no config.toml found) — agent may misbehave");
    } else {
        info!("Using config from ~/.zeus/config.toml");
    }

    // Decode Discord bot identity once at startup (cached for all messages)
    let bot_snowflake = crate::gateway_consumer::decode_bot_snowflake();
    // Resolve agent name: [agent].name (onboarding) > top-level name > [network].agent_name.
    // #296: NO match-all "zeus" default. A nameless agent gets a non-matching
    // sentinel (contains a space → can never appear in an `@token`) so it never
    // steals another agent's mentions, and we warn loudly at boot.
    let agent_name = config.agent.as_ref().and_then(|a| a.name.as_deref())
        .or_else(|| config.name.as_deref())
        .or_else(|| config.network.as_ref().and_then(|n| n.agent_name.as_deref()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            warn!(
                "No agent name configured ([agent].name / name / [network].agent_name) — \
                 using a non-matching sentinel; this agent will NOT respond to @mentions. \
                 Run 'zeus onboard' or set [agent].name in config.toml."
            );
            "<unnamed agent>".to_string()
        });
    if !bot_snowflake.is_empty() {
        info!("Discord bot identity: {} (snowflake: {})", agent_name, bot_snowflake);
    }

    // Create workspace and session
    let workspace = Workspace::from_config(&config);
    workspace.init().await?;

    // ── Defensive wallet auto-create ──────────────────────────────────────
    // If [wallet] config exists but keypair file is missing (e.g. manual config
    // edit, migration from older install), auto-generate the keypair so the
    // agent can transact. Belt-and-braces with the onboarding bootstrap.
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let zeus_dir = std::path::Path::new(&home).join(".zeus");
        let wallet_dir = zeus_dir.join("wallet");
        let key_file = wallet_dir.join("secret.key");

        if !key_file.exists() {
            let agent_id = config.agent.as_ref().and_then(|a| a.name.as_deref())
                .or_else(|| config.name.as_deref())
                .unwrap_or("zeus");
            match WalletKeypair::generate(&wallet_dir, agent_id, "solana-devnet") {
                Ok(kp) => info!(
                    pubkey = %kp.public_key_hex(),
                    "Auto-created wallet keypair (was missing)"
                ),
                Err(e) => warn!("Failed to auto-create wallet keypair: {}", e),
            }
        }
    }

    // Write Discord identity to workspace so all models know their own bot ID.
    // This goes into IDENTITY.md which is read by get_context() → system prompt.
    if !bot_snowflake.is_empty() {
        // #89.4: When allow_peer_tagging=true, relax the peer-mention restriction.
        // Safety: max 1 peer-tag per response enforced at prompt level.
        let peer_tagging_instruction = if config.gateway.as_ref().map_or(false, |g| g.allow_peer_tagging) {
            "- You MAY mention other bot IDs in your responses to delegate tasks to peers (max 1 peer mention per response)\n"
        } else {
            "- Do NOT respond to messages mentioning other bot IDs — those are for other agents\n"
        };
        let identity_content = format!(
            "# Discord Identity\n\n\
             - **Agent name**: {}\n\
             - **Discord bot ID**: {}\n\
             - **Discord mention format**: <@{}>\n\
             - When you see <@{}> in a message, that's someone mentioning YOU\n\
             {}",
            agent_name, bot_snowflake, bot_snowflake, bot_snowflake, peer_tagging_instruction
        );
        // Read existing IDENTITY.md and append Discord section if not already present
        let existing = workspace.read("IDENTITY.md").await.unwrap_or_default();
        if !existing.contains("Discord bot ID") {
            let updated = if existing.trim().is_empty() {
                identity_content
            } else {
                format!("{}\n\n{}", existing.trim(), identity_content)
            };
            if let Err(e) = workspace.write("IDENTITY.md", &updated).await {
                warn!("Failed to write Discord identity to IDENTITY.md: {}", e);
            } else {
                info!("Discord identity written to IDENTITY.md (bot ID: {})", bot_snowflake);
            }
        }
    }
    // Session key follows OpenClaw pattern: agent:{id}:main
    // Single source of truth — all channels feed into this session
    let default_agent_id = config.agents.first()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| "main".to_string());
    let session_key = format!("agent:{}:main", default_agent_id);
    // Use a fresh fallback session on gateway startup.
    //
    // Per-channel traffic already gets deterministic session IDs via
    // ChannelSessionRouter, so resuming the fallback `agent:{id}:main` session
    // here can resurrect stale coordinator/task context after a restart.
    // Keeping the key for logging/debugging preserves the naming convention
    // without reloading old work into the startup session.
    let session = Session::new(&config.sessions);
    info!(fallback_session_key = %session_key, fallback_session_id = %session.id, "Initialized fresh fallback session for gateway startup");

    // Context session router. Legacy per-channel derivation remains available,
    // but gateway channel traffic resolves through a Titan identity scope so the
    // same Titan keeps continuous context across Discord/Slack/Telegram/etc.
    // The default `session` above remains the fallback for sources without a
    // chat_id (e.g. internal API, direct CLI input).
    let channel_session_router = Arc::new(ChannelSessionRouter::new());
    info!("ChannelSessionRouter initialized — titan-scoped channel context enabled");
    // Auto-import CLI credentials (Codex CLI for OpenAI, Gemini CLI for Google)
    // before LLM client init so tokens are available in the credential store.
    {
        let (provider, _) = config.parse_model();
        match provider {
            zeus_core::Provider::Anthropic => {
                // Check for existing Anthropic OAuth tokens in credential store
                if let Ok(Some(token)) = zeus_llm::OAuthManager::get_stored_token("anthropic") {
                    info!("Found stored Anthropic OAuth token");
                }
                // Also check legacy oauth_tokens.json
                let home = std::env::var("HOME").unwrap_or_default();
                let legacy = format!("{}/.zeus/oauth_tokens.json", home);
                if std::path::Path::new(&legacy).exists() {
                    info!("Found legacy Anthropic OAuth tokens at {}", legacy);
                }
            }
            zeus_core::Provider::OpenAI => {
                // Codex CLI OAuth tokens have limited scopes (api.connectors only) —
                // can't be used for chat completions. Only import real API keys.
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    if !key.is_empty() {
                        info!("Using OPENAI_API_KEY from environment");
                        let _ = zeus_llm::OAuthManager::login_with_api_key("openai", &key);
                    }
                }
            }
            zeus_core::Provider::Google => {
                // Gemini CLI tokens (cloud-platform scope) are incompatible with
                // generativelanguage.googleapis.com (requires generative-language scope).
                // Only import if GOOGLE_API_KEY env var is set.
                if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
                    if !key.is_empty() {
                        info!("Using GOOGLE_API_KEY from environment");
                        let _ = zeus_llm::OAuthManager::login_with_api_key("google", &key);
                    }
                }
            }
            _ => {}
        }
    }

    let llm_result = LlmClient::from_config(&config);
    let bootstrap_mode = llm_result.is_err();

    // S70: LLM health probe — verify credentials work before accepting messages.
    // Skip for Ollama — the "ping" triggers a cold model load (16GB+ into GPU)
    // which blocks everything for 20-60s. Ollama health is checked via /api/tags instead.
    let (boot_provider, _) = config.parse_model();
    if boot_provider != zeus_core::Provider::Ollama {
        if let Ok(ref llm) = llm_result {
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                llm.complete(&[zeus_core::Message::user("ping")], &[], None),
            ).await {
                Ok(Ok(_)) => info!("LLM health probe: OK — provider is responding"),
                Ok(Err(e)) => warn!("LLM health probe FAILED: {} — check API key/credentials", e),
                Err(_) => warn!("LLM health probe timed out (10s) — provider may be slow or unreachable"),
            }
        }
    } else {
        info!("LLM health probe: skipped for Ollama (cold model load would block startup)");
    }

    if bootstrap_mode {
        warn!("No LLM configured — running in bootstrap mode (WebUI only)");
        warn!("Complete onboarding at http://{}:{}/", 
            config.gateway.as_ref().map(|g| g.host.as_str()).unwrap_or("0.0.0.0"),
            config.gateway.as_ref().map(|g| g.web_port).unwrap_or(8081));
    }

    // In bootstrap mode, create a dummy agent that can't do LLM calls
    // but allows the web server + onboarding API to function.
    // #178: Mount the FULL API router (not the ~5-route subset) so the
    // onboarding wizard's models/personalities/skills/channels/auth/perms
    // endpoints all work. The /v1/gateway/restart handler lets the wizard
    // trigger a clean-exit → service-manager relaunch into full gateway mode.
    let llm = match llm_result {
        Ok(llm) => llm,
        Err(_) => {
                    // Bootstrap mode — serve WebUI + full API (no agent/LLM)
                    warn!("No LLM configured — starting bootstrap server (full API, no agent)");
                    let gateway = config.gateway.clone().unwrap_or_default();
                    let web_dist = dirs::home_dir().map(|h| h.join(".zeus/web"));
                    if let Some(dist_path) = web_dist.filter(|p| p.join("index.html").exists()) {
                        let web_addr = format!("{}:{}", gateway.host, gateway.web_port);
                        let listener = tokio::net::TcpListener::bind(&web_addr).await?;
                        info!("Bootstrap WebUI on http://{}", web_addr);
                        info!("Complete onboarding at http://{} to configure the LLM", web_addr);

                        let bootstrap_state = match zeus_api::AppState::new(config.clone()) {
                            Ok(s) => Arc::new(RwLock::new(s)),
                            Err(e) => {
                                warn!("Bootstrap AppState init failed ({}), serving static files only", e);
                                axum::serve(
                                    listener,
                                    axum::Router::new()
                                        .fallback_service(
                                            tower_http::services::ServeDir::new(&dist_path).fallback(
                                                tower_http::services::ServeFile::new(dist_path.join("index.html")),
                                            ),
                                        )
                                        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
                                )
                                .await?;
                                anyhow::bail!("Bootstrap failed");
                            }
                        };

                        // Full API router — all endpoints available during onboarding.
                        // The wizard can hit /v1/gateway/restart when done to trigger
                        // a clean-exit → service-manager relaunch into full gateway mode.
                        let web_router = zeus_api::create_router(bootstrap_state, true);
                        axum::serve(
                            listener,
                            web_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                        )
                        .await?;
                    }
                    anyhow::bail!("No LLM configured and no WebUI available. Run 'zeus' for TUI setup.");
        }
    };

    // Default agent always gets full config (including channels) so the gateway
    // message consumer has a channel receiver to read from.  When [[agents]] are
    // configured, *registry* agents get channels stripped (in spawn_from_config)
    // to avoid duplicate Discord connections — the gateway consumer routes
    // inbound messages to the correct registry agent itself.
    let agent = zeus_agent::Agent::with_subsystems(config.clone(), llm, workspace, session).await?;
    let agent = Arc::new(RwLock::new(agent));

    // Initialize Prometheus with AgentToolExecutor for the cooking loop.
    // Always init (with defaults if no [prometheus] config section) so the
    // cooking loop is available for complex Discord-dispatched tasks.
    let prom_config = if config.prometheus.is_some() {
        config.clone()
    } else {
        let mut cfg = config.clone();
        cfg.prometheus = Some(zeus_core::PrometheusConfig::default());
        cfg
    };
    let mut mission_tool_executor: Option<Arc<dyn zeus_prometheus::ToolExecutor>> = None;
    let prometheus = {
        match zeus_prometheus::Prometheus::new(prom_config).await {
            Ok(mut prom) => {
                // Build a ToolRegistry matching the agent's config
                let mut registry = if config.talos.is_some() {
                    zeus_agent::ToolRegistry::with_talos(zeus_talos::TalosRegistry::with_defaults())
                } else {
                    zeus_agent::ToolRegistry::with_defaults()
                };
                // Wire TriggerHandle from Prometheus scheduler so create_trigger
                // talks to the live CronScheduler (persisted to ~/.zeus/scheduler.db)
                if let Some(scheduler_arc) = prom.scheduler() {
                    registry.set_trigger(
                        Arc::new(zeus_prometheus::TriggerHandle::new(scheduler_arc)) as Arc<dyn TriggerExecutor>
                    );
                }
                let executor = {
                    let agent_guard = agent.read().await;
                    // #181: wire the agent's Mnemosyne into the cook-path registry so
                    // memory_store works inside cooks (mirror of #167's AppState fix).
                    if let Some(mn) = agent_guard.mnemosyne() {
                        registry.set_memory(mn.clone(), agent_guard.session().id.clone());
                    }
                    let mut ex = AgentToolExecutor::new(registry, None);
                    if let Some(cm) = agent_guard.channel_manager() {
                        ex = ex.with_channels(cm);
                    }
                    ex = ex.with_llm(Arc::new(agent_guard.llm().clone()));
                    ex
                };
                prom.set_tool_executor(Arc::new(executor));

                // Build a second executor for Pantheon mission dispatch (AppState.tool_executor)
                let mut mission_registry = if config.talos.is_some() {
                    zeus_agent::ToolRegistry::with_talos(zeus_talos::TalosRegistry::with_defaults())
                } else {
                    zeus_agent::ToolRegistry::with_defaults()
                };
                if let Some(scheduler_arc) = prom.scheduler() {
                    mission_registry.set_trigger(
                        Arc::new(zeus_prometheus::TriggerHandle::new(scheduler_arc)) as Arc<dyn TriggerExecutor>
                    );
                }
                mission_tool_executor = Some({
                    let agent_guard = agent.read().await;
                    // #181: same Mnemosyne wiring for the mission executor registry.
                    if let Some(mn) = agent_guard.mnemosyne() {
                        mission_registry.set_memory(mn.clone(), agent_guard.session().id.clone());
                    }
                    let mut ex = AgentToolExecutor::new(mission_registry, None);
                    if let Some(cm) = agent_guard.channel_manager() {
                        ex = ex.with_channels(cm);
                    }
                    ex = ex.with_llm(Arc::new(agent_guard.llm().clone()));
                    Arc::new(ex) as Arc<dyn zeus_prometheus::ToolExecutor>
                });
                info!("Prometheus initialized with AgentToolExecutor");

                // Share Mnemosyne with Prometheus for memory-aware orchestration
                {
                    let agent_guard = agent.read().await;
                    if let Some(mn) = agent_guard.mnemosyne() {
                        prom.set_mnemosyne(mn.clone());
                        info!("Prometheus connected to Mnemosyne");
                    }
                }

                Some(Arc::new(RwLock::new(prom)))
            }
            Err(e) => {
                warn!("Failed to initialize Prometheus: {}", e);
                None
            }
        }
    };

    // Start Prometheus subsystems (heartbeat, scheduler, consolidation)
    if let Some(ref prometheus) = prometheus {
        let mut prom_guard = prometheus.write().await;
        // Get tool schemas from agent for heartbeat
        let tool_schemas = {
            let agent_guard = agent.read().await;
            agent_guard.tool_schemas()
        };
        // Wire Nous cognitive engine into Prometheus for cooking loop intelligence
        {
            let agent_guard = agent.read().await;
            if let Some(nous) = agent_guard.nous() {
                prom_guard.set_nous(nous.clone());
                info!("Nous wired into Prometheus cooking loop");
            }
        }
        // S69: Wire heartbeat result delivery to Discord
        let (hb_result_tx, mut hb_result_rx) = tokio::sync::mpsc::channel::<String>(16);
        prom_guard.set_heartbeat_result_tx(hb_result_tx);
        // Spawn task to deliver heartbeat results to Discord
        {
            let agent_for_hb = agent.clone();
            let fleet_ch = fleet_ch_global.clone();
            tokio::spawn(async move {
                while let Some(result) = hb_result_rx.recv().await {
                    // S79: Filter silent/low-value heartbeat results before posting to Discord.
                    // OpenClaw approach: HEARTBEAT_OK and NO_REPLY are never forwarded to channels.
                    let result_lower = result.trim().to_lowercase();
                    if result_lower.contains("heartbeat_ok")
                        || result_lower == "no_reply"
                        || result_lower.is_empty()
                    {
                        tracing::debug!("Heartbeat result suppressed (silent/low-value): {}", result.chars().take(80).collect::<String>());
                        continue;
                    }
                    let agent_guard = agent_for_hb.read().await;
                    if let Some(cm) = agent_guard.channel_manager() {
                        let target = zeus_channels::ChannelSource::with_chat(
                            "discord", "zeus", &fleet_ch
                        );
                        let msg = if result.len() > 1800 {
                            format!("{}…", result.chars().take(1800).collect::<String>())
                        } else {
                            result
                        };
                        if let Err(e) = cm.send(&target, &msg).await {
                            tracing::warn!("Failed to deliver heartbeat result to Discord: {}", e);
                        }
                    }
                }
            });
        }

        // NOTE: channel_message_active wiring moved below (after variable is defined at ~line 938)

        // Start heartbeat (replaces manual heartbeat).
        // Gated on gateway.enable_heartbeat — which now merges config-file
        // ([gateway]/[prometheus] enable_heartbeat) with the CLI --no-heartbeat
        // flag (either source can disable). Previously this call was
        // unconditional, so config-file disable was silently ignored.
        if gateway.enable_heartbeat {
            if let Err(e) = prom_guard.start_heartbeat(tool_schemas).await {
                warn!("Failed to start Prometheus heartbeat: {}", e);
            }
        } else {
            warn!(
                "Heartbeat is DISABLED (config or --no-heartbeat) — proactive HEARTBEAT.md tasks will NOT fire. \
                 To enable: set enable_heartbeat = true under [gateway] in ~/.zeus/config.toml and restart the gateway."
            );
        }
        // Wire trigger result channel: scheduler → gateway → agent context
        // Must be set BEFORE start_scheduler() so the loop has the sender.
        let (trigger_result_tx, mut trigger_result_rx) =
            tokio::sync::mpsc::unbounded_channel::<String>();
        prom_guard.set_trigger_result_tx(trigger_result_tx);
        // Start cron scheduler — gated on gateway.enable_cron (config + CLI).
        if gateway.enable_cron {
            if let Err(e) = prom_guard.start_scheduler().await {
                warn!("Failed to start Prometheus scheduler: {}", e);
            }
        } else {
            warn!(
                "Cron scheduler is DISABLED (config or --no-cron) — scheduled tasks and triggers will NOT fire. \
                 To enable: set enable_cron = true under [gateway] in ~/.zeus/config.toml and restart the gateway."
            );
        }
        // Start consolidation engine
        prom_guard.start_consolidation();
        info!("Prometheus subsystems started (heartbeat, scheduler, consolidation)");

        // Spawn trigger result listener: when a cron trigger fires, inject its
        // output into the agent's context by triggering a cook with the output
        // as a system message.
        {
            let trigger_agent = agent.clone();
            let trigger_prom = prometheus.clone();
            tokio::spawn(async move {
                while let Some(output) = trigger_result_rx.recv().await {
                    info!("Trigger result received, injecting into agent context");
                    // Add the trigger output as a system message to the agent's session
                    {
                        let mut guard = trigger_agent.write().await;
                        let _ = guard.session_mut().add(
                            zeus_core::Message::system(&output)
                        ).await;
                    }
                    // Wake the heartbeat so the agent processes the trigger output.
                    // NOTE: this is a bounded mpsc Sender — `send()` returns a
                    // Future. Without `.await` the future is dropped and the
                    // wake silently never fires (the bug that left trigger
                    // output sitting in the session until the next safety-net
                    // tick, up to 1h later).
                    if let Some(ref wake_tx) = trigger_prom.read().await.heartbeat_wake_sender() {
                        if let Err(e) = wake_tx
                            .send(zeus_prometheus::heartbeat::WakeRequest {
                                reason: "trigger_fired".to_string(),
                                agent_id: None,
                            })
                            .await
                        {
                            warn!("Failed to send trigger_fired wake request: {e}");
                        }
                    }
                }
                info!("Trigger result listener shut down");
            });
        }

        // S66-P2: Session-start hook — inject active goals into agent context
        let active_goals = prom_guard.active_goals_summary();
        if !active_goals.is_empty() {
            info!("Session start: {} active goal(s) pending", active_goals.len());
            let goal_context = active_goals.join("\n");
            let mut agent_guard = agent.write().await;
            agent_guard.set_goals_context(Some(format!(
                "You have {} pending goal(s):\n{}",
                active_goals.len(),
                goal_context
            )));
            info!("Goals context injected into agent system prompt");
        }
    }

    // S67-C2: Grab heartbeat wake sender for event-driven triggers (outer scope)
    let heartbeat_wake_tx = if let Some(ref prom) = prometheus {
        let guard = prom.read().await;
        guard.heartbeat_wake_sender()
    } else {
        None
    };

    // #89.2: Grab peer-status context handle for fleet presence injection
    let peer_status_handle: Option<Arc<tokio::sync::RwLock<Option<String>>>> =
        if let Some(ref prom) = prometheus {
            let guard = prom.read().await;
            guard.peer_status_handle()
        } else {
            None
        };

    // #89.3: Grab unread context handle for message history injection
    let unread_handle: Option<Arc<tokio::sync::RwLock<Option<String>>>> =
        if let Some(ref prom) = prometheus {
            let guard = prom.read().await;
            guard.unread_handle()
        } else {
            None
        };

    // Collect tasks to run concurrently
    let mut tasks: Vec<tokio::task::JoinHandle<Result<()>>> = Vec::new();
    // Shared shutdown token — all background tasks check this for graceful exit
    let shutdown_token = tokio_util::sync::CancellationToken::new();

    // Create shared API state (used by API server and agent registry routing)
    let api_state = Arc::new(RwLock::new(
        zeus_api::AppState::new(config.clone())
            .map_err(|e| anyhow::anyhow!("AppState init failed: {}", e))?,
    ));

    // #315 item-0: share the agent loop's LIVE ChannelManager into AppState.
    // AppState::new wires an empty ChannelManager::new(256) (zeus-api lib.rs),
    // so every HTTP-path consumer — /v1/tools `message` execution, channel
    // handlers, webhooks, the Pantheon announcement hook — saw zero adapters
    // even when the agent's channels were healthy. Overwrite BOTH the state
    // field and the ToolRegistry's handle with the agent's real manager.
    // Must happen BEFORE AppState::boot: the announcement hook clones
    // state.channel_manager during boot and would otherwise keep the empty one.
    {
        let live_channels = { agent.read().await.channel_manager() };
        if let Some(cm) = live_channels {
            let mut s = api_state.write().await;
            s.channel_manager = cm.clone();
            s.tools.set_channels(cm);
            info!("AppState wired with agent's live ChannelManager (#315 item-0)");
        }
    }

    zeus_api::AppState::boot(&api_state).await;

    // Wire default agent into AppState so /v1/chat shares the agent's session
    api_state.write().await.default_agent = Some(agent.clone());

    // Wire tool executor for Pantheon mission dispatch (real tool execution, not simulated)
    if let Some(exec) = mission_tool_executor {
        api_state.write().await.tool_executor = Some(exec);
        info!("Pantheon mission tool executor wired into AppState");
    }

    // Wire agent's Mnemosyne into AppState so API routes can access it
    {
        let agent_guard = agent.read().await;
        if let Some(mn) = agent_guard.mnemosyne() {
            {
                let mut state = api_state.write().await;
                state.mnemosyne = Some(mn.clone());
                // Wire the same agent Mnemosyne into AppState's tool registry so the
                // API-invoked `memory_store` tool writes to the agent's store (not a
                // second DB handle). session id matches the agent for memory grouping.
                state.tools.set_memory(mn.clone(), agent_guard.session().id.clone());
            }
            info!("Mnemosyne wired into AppState (+ tools.memory_store)");

            // Sync workspace files → Mnemosyne on boot (MEMORY.md, AGENTS.md, etc.)
            let workspace_path = config.workspace.clone();
            let mn_boot = mn.clone();
            tokio::spawn(async move {
                match mn_boot.sync_workspace(&workspace_path).await {
                    Ok(stats) => info!(
                        "Workspace→Mnemosyne boot sync: {} files scanned, {} changed, {} chunks embedded",
                        stats.files_scanned, stats.files_changed, stats.chunks_embedded
                    ),
                    Err(e) => warn!("Workspace→Mnemosyne boot sync failed: {}", e),
                }
            });
        }
    }

    // S36 Track B: Pre-populate agent registry from config.agents.
    // Each AgentConfig entry becomes an isolated Agent instance with its own
    // workspace, sessions, and model. Messages tagged with `account_id` are
    // routed to the matching agent via `route_by_account()`.
    if !config.agents.is_empty() {
        let mut state = api_state.write().await;
        for agent_cfg in &config.agents {
            match state.agent_registry.spawn_from_config(agent_cfg).await {
                Ok(()) => info!(
                    "Agent registry: registered '{}' (account={:?})",
                    agent_cfg.id, agent_cfg.discord_account
                ),
                Err(e) => warn!("Agent registry: failed to register '{}': {}", agent_cfg.id, e),
            }
        }
        info!("Agent registry populated: {} agent(s)", config.agents.len());

        // Share the default agent's ChannelManager with all registry agents
        // so their `message` tool can send through platform channels.
        let default_channels = {
            let default_agent = agent.read().await;
            default_agent.channel_manager()
        };
        if let Some(channels) = default_channels {
            state.agent_registry.share_channels(channels).await;
        }
    }

    // Wire real ToolExecutor into AppState for Pantheon mission execution
    {
        let mut registry = if config.talos.is_some() {
            zeus_agent::ToolRegistry::with_talos(zeus_talos::TalosRegistry::with_defaults())
        } else {
            zeus_agent::ToolRegistry::with_defaults()
        };
        // Wire TriggerHandle from Prometheus scheduler
        if let Some(ref prometheus) = prometheus {
            let prom_guard = prometheus.read().await;
            if let Some(scheduler_arc) = prom_guard.scheduler() {
                registry.set_trigger(
                    Arc::new(zeus_prometheus::TriggerHandle::new(scheduler_arc)) as Arc<dyn TriggerExecutor>
                );
            }
        }
        let executor: Arc<dyn zeus_prometheus::ToolExecutor> = {
            let agent_guard = agent.read().await;
            // #181: wire Mnemosyne so Pantheon-mission tool calls can memory_store.
            if let Some(mn) = agent_guard.mnemosyne() {
                registry.set_memory(mn.clone(), agent_guard.session().id.clone());
            }
            let mut ex = AgentToolExecutor::new(registry, None);
            if let Some(cm) = agent_guard.channel_manager() {
                ex = ex.with_channels(cm);
            }
            ex = ex.with_llm(Arc::new(agent_guard.llm().clone()));
            Arc::new(ex)
        };
        api_state.write().await.tool_executor = Some(executor);
        info!("Pantheon missions wired with real ToolExecutor");
    }

    // Periodic mission timeout check (every 60s, configurable via [gateway].timeout_secs)
    let _mission_timeout_handle = {
        let s = api_state.read().await;
        let timeout = s
            .config
            .gateway
            .as_ref()
            .map(|g| g.timeout_secs)
            .unwrap_or(1800);
        let store = s.pantheon.clone();
        drop(s);
        zeus_api::PantheonStore::start_timeout_check_task(
            store,
            std::time::Duration::from_secs(timeout),
        )
    };
    info!("Mission timeout checker started (interval=60s, default_timeout=30min)");

    // #89.2: Fleet peer-status watcher — periodically formats channel_presence
    // into the heartbeat's peer_status_context so wake prompts include fleet status.
    if let Some(handle) = peer_status_handle {
        let watcher_state = api_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let state = watcher_state.read().await;
                let presence = state.channel_presence.read().await;
                let formatted = if presence.is_empty() {
                    None
                } else {
                    let items: Vec<String> = presence.values().map(|p| {
                        format!("- {} ({}) — last seen: {}, last msg: {}", p.name, p.agent_type, p.last_seen, p.last_message)
                    }).collect();
                    Some(format!("\n\n## Fleet Peer Status\n{}", items.join("\n")))
                };
                *handle.write().await = formatted;
            }
        });
        info!("Fleet peer-status watcher started (interval=30s)");
    }

    // #89.3: Unread message watcher — periodically queries discord_history
    // and formats recent messages into the heartbeat's unread_context.
    if let Some(handle) = unread_handle {
        if !fleet_ch_global.is_empty() {
        let watcher_state = api_state.clone();
        let channel_id = fleet_ch_global.clone();
        let channel_id_log = channel_id.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let state = watcher_state.read().await;
                let messages = state.discord_history.get_history(&channel_id, 10).await;
                let formatted = if messages.is_empty() {
                    None
                } else {
                    let items: Vec<String> = messages.iter().map(|m| {
                        let role = if m.is_bot { "bot" } else { "user" };
                        format!("- [{}] {}: {}", role, m.author_name, m.content.chars().take(120).collect::<String>())
                    }).collect();
                    Some(format!("\n\n## Recent Channel Messages\n{}", items.join("\n")))
                };
                *handle.write().await = formatted;
            }
        });
        info!("Unread message watcher started (interval=60s, channel={})", channel_id_log);
        }
    }

    // S98: Ensure a default war room exists so Pantheon TUI isn't empty on first launch.
    // S106 (regression fix): Also ensure every configured agent has a Pantheon identity
    // and is auto-joined to the default fleet-ops room. Without this, the TUI shows
    // 0 members / 0 messages because the `identities` and `room_members` tables never
    // get populated on boot.
    {
        let s = api_state.read().await;

        // 1. Find or create the default fleet-ops room.
        let rooms = s.pantheon.list_rooms().await;
        let fleet_room_id = if let Some(existing) = rooms.iter().find(|r| r.name == "fleet-ops") {
            existing.id.clone()
        } else {
            let now = chrono::Utc::now();
            let room = zeus_api::Room {
                id: format!("r-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                name: "fleet-ops".to_string(),
                description: Some("Default fleet coordination room".to_string()),
                room_type: zeus_api::RoomType::Public,
                mission_id: None,
                created_by: "system".to_string(),
                created_at: now,
                updated_at: now,
            };
            let room_id = room.id.clone();
            s.pantheon.insert_room(&room).await;
            info!("Created default war room 'fleet-ops' ({})", room_id);
            room_id
        };

        // 2. Auto-register every configured agent as a Pantheon identity and
        //    join them to fleet-ops. Idempotent — safe to run every boot.
        let now = chrono::Utc::now();
        let mut joined = 0usize;
        for agent_cfg in &config.agents {
            let display_name = agent_cfg
                .name
                .clone()
                .unwrap_or_else(|| agent_cfg.id.clone());

            // Upsert identity (display_name).
            s.pantheon
                .set_identity(&agent_cfg.id, &display_name, None)
                .await;

            // Join fleet-ops (INSERT OR IGNORE — idempotent).
            let member = zeus_api::RoomMember {
                agent_id: agent_cfg.id.clone(),
                agent_name: display_name,
                role: "member".to_string(),
                joined_at: now,
            };
            s.pantheon.join_room(&fleet_room_id, &member).await;
            joined += 1;
        }
        if joined > 0 {
            info!(
                "Pantheon: auto-registered {} agent identit(ies) and joined fleet-ops ({})",
                joined, fleet_room_id
            );
        }
    }

    // Phase 4: Pantheon IRC client auto-connect
    // If [pantheon] config exists, connect to the standalone Pantheon server on boot.
    let _pantheon_client_tx = if let Some(ref pantheon_cfg) = config.pantheon {
        let client_config = zeus_pantheon_server::client::ClientConfig {
            server_url: pantheon_cfg.server.clone(),
            user_id: pantheon_cfg.nick.clone(),
            display_name: pantheon_cfg.nick.clone(),
            channel_key: pantheon_cfg.channel_key.clone(),
            is_agent: pantheon_cfg.is_agent,
            auto_join: pantheon_cfg.auto_join.clone(),
        };
        let (inbound_tx, mut inbound_rx) = tokio::sync::mpsc::channel(256);
        let outbound_tx = zeus_pantheon_server::client::spawn_auto_connect(client_config, inbound_tx);
        info!(
            "Pantheon IRC: connecting to {} as {} (auto-join: {:?})",
            pantheon_cfg.server, pantheon_cfg.nick, pantheon_cfg.auto_join
        );

        // Spawn a task to log inbound Pantheon messages (bridge can forward to Discord later)
        let bridge_channel = pantheon_cfg.bridge_channel.clone();
        tokio::spawn(async move {
            while let Some(msg) = inbound_rx.recv().await {
                match &msg {
                    zeus_pantheon_server::protocol::ServerMessage::AuthOk { user_id, channels, .. } => {
                        info!("Pantheon IRC: authenticated as {} (channels: {:?})", user_id, channels);
                    }
                    zeus_pantheon_server::protocol::ServerMessage::AuthErr { reason } => {
                        warn!("Pantheon IRC: auth failed: {}", reason);
                    }
                    zeus_pantheon_server::protocol::ServerMessage::Msg { channel, from, content, .. } => {
                        info!("Pantheon [{}] <{}> {}", channel, from.display_name, content);
                    }
                    _ => {}
                }
            }
        });

        Some(outbound_tx)
    } else {
        None
    };

    // Session pruning background task
    let _pruning_handle = if config.pruning.as_ref().map(|p| p.enabled).unwrap_or(false) {
        let pruning_config = config.pruning.clone().expect("pruning checked above");
        let sessions_dir = config.sessions.clone();
        let handle = zeus_session::start_pruning_task(pruning_config.clone(), sessions_dir);
        info!(
            "Session pruning started (interval={}s, max_age={}d, max_sessions={}, max_size={}MB{})",
            pruning_config.check_interval_secs,
            pruning_config.max_age_days,
            pruning_config.max_sessions,
            pruning_config.max_total_size_mb,
            if pruning_config.dry_run {
                ", dry_run"
            } else {
                ""
            }
        );
        Some(handle)
    } else {
        None
    };

    // Start config file watcher for hot-reload (must live as long as the gateway)
    let _config_watcher = if gateway.enable_api {
        match zeus_api::start_config_watcher(api_state.clone()) {
            Ok((watcher, handle)) => {
                info!("Config hot-reload watcher started");
                Some((watcher, handle))
            }
            Err(e) => {
                warn!("Failed to start config watcher: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Web Frontend API router — built from the same AppState + auth config as
    // the :8080 API server, so the WebUI port can serve SPA + API same-origin.
    // Populated inside the `enable_api` block below (where the auth params are
    // in scope) and handed to spawn_web_server as the router base.
    let mut web_api_router: Option<axum::Router> = None;

    // 1. API Server
    if gateway.enable_api {
        let auth_token = gateway.api_token.clone();
        let allowed_origins: Vec<String> = gateway.cors_origins.as_deref()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect();
        let rl_cfg = gateway
            .rate_limit
            .enabled
            .then_some(zeus_api::RateLimitConfig {
                global_rpm: gateway.rate_limit.global_rpm,
                llm_rpm: gateway.rate_limit.llm_rpm,
                burst_size: gateway.rate_limit.burst_size,
                cleanup_interval_secs: 300,
            });
        let router = zeus_api::create_router_with_auth(
            api_state.clone(),
            true,
            auth_token.clone(),
            &allowed_origins,
            rl_cfg.clone(),
        );

        // Build a second, independent router instance with the same routes,
        // state, and auth for the WebUI port (:8081). axum Routers aren't
        // Clone, so we construct a fresh one from the same inputs rather than
        // sharing the :8080 instance.
        web_api_router = Some(zeus_api::create_router_with_auth(
            api_state.clone(),
            true,
            auth_token,
            &allowed_origins,
            rl_cfg,
        ));

        let addr = format!("{}:{}", gateway.host, gateway.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        info!("API server listening on http://{}", addr);

        let api_shutdown = shutdown_token.clone();
        tasks.push(tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                    api_shutdown.cancelled().await;
                })
                .await
                .map_err(|e| anyhow::anyhow!("API server error: {}", e))
        }));
    }

    // 1a. Web Frontend Server (separate port, serves SPA + API same-origin)
    if let Some(web_router) = web_api_router {
        if let Some(task) = crate::gateway_web::spawn_web_server(
            &gateway.host,
            gateway.web_port,
            gateway.web_dist.as_deref(),
            web_router,
            shutdown_token.clone(),
        ).await {
            tasks.push(task);
        }
    }

    // 1b. MCP Server
    #[cfg(feature = "mcp")]
    if gateway.enable_mcp {
        let mcp_config = zeus_mcp::McpConfig {
            host: gateway.host.clone(),
            port: gateway.mcp_port,
            cors: true,
            workspace: Some(config.workspace.display().to_string()),
            auth_token: None,
        };
        // Surface D(a): pass the agent's shared ChannelManager into the MCP
        // server so MCP-served `message` tool calls can dispatch to platform
        // adapters (matches Cut D-real / Surface E on the agent-loop side).
        let mcp_channels = {
            let agent_guard = agent.read().await;
            agent_guard.channel_manager()
        };
        let mcp_server =
            zeus_mcp::McpServer::with_full_config(mcp_config, &config, mcp_channels);
        let mcp_addr = mcp_server.address();
        info!("MCP server listening on http://{}", mcp_addr);

        tasks.push(tokio::spawn(async move {
            mcp_server
                .run()
                .await
                .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))
        }));
    }

    // 1c-1d. Bootstrap workspace (HEARTBEAT.md, CAPABILITIES.md, goal files)
    crate::gateway_bootstrap::bootstrap_workspace(&agent, &config).await;
    if let Some(ref prom) = prometheus {
        crate::gateway_bootstrap::load_goal_files(prom, &config.workspace).await;
    }

    // 1e. Check for interrupted cooking sessions and auto-resume (S66-P1A)
    // Skip auto-resume on fresh start — agent should have zero prior context.
    // Initialize the in-memory fresh-start flag exactly once here, before any
    // other code path reads it (channel history injection, etc.). This avoids
    // the race where each callsite raced to delete the marker file independently.
    let is_fresh_start = crate::gateway_bootstrap::init_fresh_start_flag();
    if is_fresh_start {
        info!("Fresh start detected — skipping auto-resume of interrupted sessions");
    }

    // Gate auto-resume on heartbeat being enabled. Auto-resume resurrects
    // interrupted cooking sessions — when heartbeat is disabled (via config or
    // --no-heartbeat) the agent is meant to be quiet, so a resumed session
    // re-posting is exactly the "there he is again" loop. Honor the disable.
    if !gateway.enable_heartbeat {
        info!("Heartbeat disabled — skipping auto-resume of interrupted cooking sessions");
    }
    if !is_fresh_start && gateway.enable_heartbeat {
        if let Some(ref prom) = prometheus {
            let prom_guard = prom.read().await;
            let interrupted = prom_guard.find_interrupted_sessions().await;
            drop(prom_guard);
            if let Some(prom_clone) = prometheus.clone() {
                crate::gateway_bootstrap::spawn_session_resume(prom_clone, agent.clone(), interrupted);
            }
        }
    }

    // S-PRIORITY: Flag to signal a channel message is actively being processed.
    // Heartbeat checks this before acquiring the agent lock — defers if true.
    // Declared here (outside channel block) so heartbeat spawn can access it.
    let channel_cook_state: zeus_core::CookState = zeus_core::CookState::new();

    // Bug B Part 2: cancel token for in-flight channel cook.
    // When a new channel message arrives, the previous cook is cancelled gracefully
    // (finishes current iteration, doesn't start new ones), then replaced.
    let current_cook_cancel: Arc<tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Wire channel-active flag into Prometheus heartbeat so it defers during real message processing
    if let Some(ref prom) = prometheus {
        prom.write().await.set_channel_active(channel_cook_state.clone());
    }

    // S98: Unified agent inbox — all input sources push here, one consumer processes sequentially.
    // This prevents concurrent agent.run() calls and session corruption.
    let (agent_inbox, inbox_rx, inbox_queue_depth) = zeus_core::inbox::create_inbox();

    // Wire inbox-queue-depth into Heartbeat for busy-aware fire-decision (`busy: inbound`).
    if let Some(ref prom) = prometheus {
        prom.write().await.set_inbox_queue_depth(std::sync::Arc::clone(&inbox_queue_depth));
    }

    // Wire inbox into AppState so websocket/chat_handlers can send messages through it.
    api_state.write().await.agent_inbox = Some(agent_inbox.clone());
    let inbox_agent = agent.clone();
    let inbox_prometheus = prometheus.clone();
    let _inbox_config = config.clone();
    let inbox_cook_state = channel_cook_state.clone();
    let inbox_sessions_dir = config.sessions.clone();
    let inbox_default_agent_id = default_agent_id.clone();
    let inbox_api_state = api_state.clone();
    let inbox_consumer_depth = std::sync::Arc::clone(&inbox_queue_depth);
    // #192b P2b: per-session cook lanes. Different sessions cook concurrently;
    // same-session cooks serialize FIFO via the lane's OwnedMutexGuard, which
    // the dispatcher acquires (lock_owned) BEFORE spawn and moves into the task.
    let inbox_lane_mgr = std::sync::Arc::new(zeus_core::SessionLaneManager::new());
    tasks.push(tokio::spawn(async move {
        let mut rx = inbox_rx;
        while let Some(msg) = rx.recv().await {
            // Counter-invariant: decrement BEFORE handler dispatch (panic-drift mitigation).
            // Cook-flight "busy: cook" via CookState is the orthogonal in-flight signal.
            inbox_consumer_depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            // Session isolation (#192b P1): inbox messages (from TUI/API) use their
            // own session. The COOK path now runs against a per-cook local `Session`
            // resolved from the shared store — never the shared agent's `session`
            // field — so a cook takes NO `agent.write()` (the inner-lane removal).
            // The legacy `use_cooking=false` `run()` path still requires `&mut Agent`
            // (it reads `self.session`), so that branch alone does a SCOPED swap +
            // restore below, preserving its session isolation (zero behavior change).
            let inbox_session_id = msg
                .session_id
                .clone()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| "agent:main:main".to_string());
            let prev_session_id = {
                let guard = inbox_agent.read().await;
                guard.session().id.clone()
            };
            // Per-cook session carrier — resolved from the #192 shared session store
            // (resume_or_create reads the same on-disk session the swap used to load),
            // so history + persistence are byte-identical to the pre-P1 behavior.
            let mut cook_ctx = zeus_prometheus::CookContext::new(
                zeus_session::Session::resume_or_create(
                    &inbox_sessions_dir, &inbox_session_id,
                ).await,
            );
            // Tracks whether the legacy branch swapped the shared agent's session,
            // so the restore at the bottom only fires for that branch.
            let mut legacy_swapped = false;

            // FIX D: Inject Discord fleet-channel history so TUI agents see recent
            // Discord context (mirrors channel-handler path at gateway.rs:1776).
            // Uses the primary fleet channel; gated by inject_channel_history()
            // internals (fresh-start skip, session-count skip, discord-type filter).
            let msg_content = {
                let state = inbox_api_state.read().await;
                let own_bot_id = crate::gateway_consumer::decode_bot_snowflake();
                // #192b P1: read the per-cook session, not the shared agent field.
                let session_msg_count = cook_ctx.session().messages.len();
                crate::gateway_consumer::inject_channel_history(
                    msg.content.clone(),
                    "discord",
                    "1488620262676238426",
                    None,
                    session_msg_count,
                    &own_bot_id,
                    &state.discord_history,
                ).await
            };

            // #192b P2b: per-session cook lane (replaces the global CAS serializer).
            // The inbox consumer is single-session ("agent:main:main"), so every inbox
            // cook acquires the SAME lane → inbox stays internally serial (FIFO), exactly
            // as before. But the lane is per-session, so an inbox cook NO LONGER mutually
            // excludes a CHANNEL cook on a different session — that is the cross-loop
            // concurrency win. The global single-slot CAS (`try_acquire`) used to force
            // fleet-wide serialization; the lane scopes exclusion to the session id.
            // RAII: the OwnedMutexGuard releases at end of this iteration (panic-safe).
            let _inbox_lane = inbox_lane_mgr.lane(&inbox_session_id).lock_owned().await;
            // In-flight counter for concurrency-aware heartbeat deferral (Bug-A preserved):
            // increments here, decrements when `_inbox_flight` drops at iteration end.
            // Unlike the single-slot CAS, this counts correctly under concurrent cooks.
            let _inbox_flight = inbox_cook_state.begin_cook(zeus_core::ActiveCookType::Channel);

            // If this is a streaming request, set the stream callback on the agent
            // so LLM tokens are forwarded in real-time (TUI "thinking..." → live tokens)
            let stream_tx_clone = if let zeus_core::inbox::ResponseChannel::Stream(ref tx) = msg.response_tx {
                let tx_clone = tx.clone();
                let mut guard = inbox_agent.write().await;
                guard.set_stream_tx(tx_clone.clone());
                drop(guard);
                Some(tx_clone)
            } else {
                None
            };

            // Forward cooking events (tool starts/ends/iterations) to stream channel
            // so TUI shows live progress instead of static "thinking..."
            let event_forwarder = if let Some(ref stream_tx) = stream_tx_clone {
                if let Some(ref prom) = inbox_prometheus {
                    let mut event_rx = prom.read().await.subscribe_events();
                    let tx = stream_tx.clone();
                    Some(tokio::spawn(async move {
                        while let Ok(event) = event_rx.recv().await {
                            let chunk = match event {
                                zeus_prometheus::CookingEvent::ToolExecutionStart { name, input, .. } => {
                                    let input_summary = input
                                        .map(|v| v.to_string().chars().take(60).collect::<String>())
                                        .unwrap_or_default();
                                    Some(zeus_core::inbox::StreamChunk::ToolStart { name, input: input_summary })
                                }
                                zeus_prometheus::CookingEvent::ToolExecutionComplete { name, result, .. } => {
                                    let output = result.chars().take(100).collect();
                                    Some(zeus_core::inbox::StreamChunk::ToolEnd { name, output })
                                }
                                zeus_prometheus::CookingEvent::TextDelta { text, .. } => {
                                    Some(zeus_core::inbox::StreamChunk::Token(text))
                                }
                                zeus_prometheus::CookingEvent::ThinkingDelta { text, .. } => {
                                    // Wrap in <think> tags so TUI parser routes to Thinking event
                                    Some(zeus_core::inbox::StreamChunk::Thinking(text))
                                }
                                _ => None,
                            };
                            if let Some(c) = chunk {
                                if tx.send(c).await.is_err() {
                                    break; // receiver dropped
                                }
                            }
                        }
                    }))
                } else { None }
            } else { None };

            let timeout = std::time::Duration::from_secs(msg.timeout_secs);
            let result = tokio::time::timeout(timeout, async {
                if msg.use_cooking {
                    if let Some(ref prom) = inbox_prometheus {
                        let prom_guard = prom.read().await;
                        let (tool_schemas, session_history, channel_manager) = {
                            let agent_guard = inbox_agent.read().await;
                            let schemas = agent_guard.tool_schemas();
                            // #192b P1: build history from the per-cook session, not the
                            // shared agent field. #52: #51 engine seam (recent_history_for_llm).
                            let recent = cook_ctx.session().recent_history_for_llm();
                            (schemas, recent, agent_guard.channel_manager())
                        };
                        let user_content = msg_content.clone();
                        // Tier 1: keep channels showing "typing..." during cooking.
                        // Guard cancels the heartbeat on drop (any return path).
                        let _typing_guard = spawn_typing_heartbeat(channel_manager, msg.source.clone());
                        let alias = {
                            let sessions_guard = prom_guard.sessions().read().await;
                            let (human_id, channel_kind) = match &msg.source {
                                Some(src) => (src.sender_id.as_deref(), src.channel_type.parse().unwrap_or(zeus_prometheus::ChannelKind::Other("internal".to_string()))),
                                None => (None, zeus_prometheus::ChannelKind::Other("internal".to_string())),
                            };
                            prom_guard.session_resolver(&sessions_guard, &inbox_default_agent_id, human_id, channel_kind, chrono::Utc::now()).await
                        };
                        let prior_dispatches = prom_guard.track_dispatch(&alias).await;
                        let human_display = msg.source.as_ref().and_then(|s| s.sender_id.as_deref()).unwrap_or("none");
                        let channel_kind_display = msg.source.as_ref().map(|s| s.channel_type.parse().unwrap_or(zeus_prometheus::ChannelKind::Other("internal".to_string()))).unwrap_or(zeus_prometheus::ChannelKind::Other("internal".to_string()));
                        let cook_span = info_span!("cook",
                            fleet_session_alias = %alias,
                            agent = %inbox_default_agent_id,
                            human = %human_display,
                            channel_kind = %channel_kind_display,
                            surface = "gateway",
                            callsite = "gateway:1123"
                        );
                        let _cook_guard = cook_span.enter();
                        tracing::info!(
                            gate = "fleet_session_correlation",
                            fleet_session_alias = %alias,
                            resolved_via = alias.resolved_via(),
                            prior_dispatches_for_alias = prior_dispatches,
                            agent = %inbox_default_agent_id,
                            human = %human_display,
                            channel_kind = %channel_kind_display,
                            "resolver decision"
                        );
                        tracing::info!(
                            gate = "cook_dispatched",
                            history_len = session_history.len(),
                            message_len = msg_content.len(),
                            "cook entry"
                        );
                        drop(_cook_guard);
                        match prom_guard.cook_with_history(&msg_content, &tool_schemas, &session_history).instrument(cook_span).await {
                            Ok(result) => {
                                // Persist user + assistant messages to session so subsequent
                                // messages have full conversation context. Without this,
                                // each cook sees stale history and the agent "forgets" prior turns.
                                // #192b P1: persist to the per-cook session (shared
                                // store), not the shared agent field → no agent.write().
                                let _ = cook_ctx.session_mut().add(zeus_core::Message::user(&user_content)).await;
                                let _ = cook_ctx.session_mut().add(zeus_core::Message::assistant(&result.response)).await;
                                Ok(result.response)
                            }
                            Err(e) => Err(format!("Cooking error: {}", e)),
                        }
                    } else {
                        // Legacy run() path: reads self.session, so it needs the
                        // shared agent on the inbox session. Scoped swap (this branch
                        // only) preserves isolation; restored at loop bottom.
                        let mut guard = inbox_agent.write().await;
                        if prev_session_id != inbox_session_id {
                            guard.set_session(zeus_session::Session::resume_or_create(&inbox_sessions_dir, &inbox_session_id).await);
                            legacy_swapped = true;
                        }
                        guard.run(&msg_content).await
                            .map_err(|e| format!("Agent error: {}", e))
                    }
                } else {
                    // Always use full agent path with tools — model decides
                    // whether to call tools or respond with text.
                    // (run_fast was skipping tools entirely, breaking tool access
                    // for any message that didn't match a keyword list)
                    let (provider, _) = _inbox_config.parse_model();
                    // Legacy run() path: reads self.session → scoped swap (restored
                    // at loop bottom) preserves inbox isolation. (#192b P1)
                    let mut guard = inbox_agent.write().await;
                    if prev_session_id != inbox_session_id {
                        guard.set_session(zeus_session::Session::resume_or_create(&inbox_sessions_dir, &inbox_session_id).await);
                        legacy_swapped = true;
                    }
                    if msg.attachments.is_empty() {
                        guard.run(&msg_content).await
                            .map_err(|e| format!("Agent error: {}", e))
                    } else {
                        guard.run_with_attachments(&msg_content, msg.attachments).await
                            .map_err(|e| format!("Agent error: {}", e))
                    }
                }
            }).await;

            // RAII: CookGuard drops at end of scope, releasing cook state automatically

            // Stop event forwarder now that cooking is done
            if let Some(handle) = event_forwarder {
                handle.abort();
            }

            // Clear stream callback after run completes
            if stream_tx_clone.is_some() {
                let mut guard = inbox_agent.write().await;
                guard.clear_stream_tx();
            }

            let response = match result {
                Ok(Ok(text)) => Ok(text),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(format!("Processing timed out ({}s)", msg.timeout_secs)),
            };

            match msg.response_tx {
                zeus_core::inbox::ResponseChannel::OneShot(tx) => {
                    if tx.send(response).is_err() {
                        warn!("Response channel dropped — client may have disconnected");
                    }
                }
                zeus_core::inbox::ResponseChannel::Stream(tx) => {
                    if tx.send(zeus_core::inbox::StreamChunk::Done(response)).await.is_err() {
                        warn!("Stream response channel dropped — client may have disconnected");
                    }
                }
            }

            // #192b P1: only the legacy run() branch swaps the shared agent's
            // session, so restore only when it did. The cook path never swapped
            // (it ran against cook_ctx), so it needs no restore — no agent.write()
            // on the cook path at all.
            if legacy_swapped {
                let restored = zeus_session::Session::resume_or_create(
                    &inbox_sessions_dir, &prev_session_id,
                ).await;
                let mut guard = inbox_agent.write().await;
                guard.set_session(restored);
                tracing::debug!("Inbox: restored session → {}", prev_session_id);
            }
        }
        Ok::<(), anyhow::Error>(())
    }));

    // #13-B: snapshot prometheus config before channel spawns consume `config` via move.
    let prometheus_config_snapshot = config.prometheus.clone();

    // 2. Channel adapters - consume inbound messages and route to agent
    //    Uses agent registry for binding-based routing with default agent fallback
    if gateway.enable_channels
        && (config.channels.is_some() || zeus_core::ChannelsConfig::from_env().is_some())
    {
        let channel_rx = {
            let mut agent_guard = agent.write().await;
            agent_guard.take_channel_receiver()
        };

        if let Some(mut rx) = channel_rx {
            let agent_for_rx = agent.clone();
            let cook_state_for_rx = channel_cook_state.clone();
            // #192b P2c: per-session lane manager for the channels consumer.
            // Different channel sessions cook concurrently; same-session cooks
            // serialize FIFO via `lane(key).lock_owned()`. Replaces the global
            // single-slot CAS as the cross-session exclusion primitive (the CAS
            // is retained only as the S-PRIORITY heartbeat-deferral advisory).
            let channel_lane_mgr = std::sync::Arc::new(zeus_core::SessionLaneManager::new());
            let cook_cancel_for_rx = current_cook_cancel.clone();
            let api_state_for_rx = api_state.clone();
            let prometheus_for_rx = prometheus.clone();
            let config_for_threads = config.clone();
            let enable_agent_processing = gateway.enable_agent_processing;
            // Per-channel session wiring — moved into consumer so we can
            // swap the dispatch_agent's session based on (channel_type, chat_id).
            let channel_session_router_for_rx = channel_session_router.clone();
            let sessions_dir_for_rx = config.sessions.clone();

            // Thread-bound subagents: each Discord thread gets its own agent context
            let thread_agents: Arc<
                tokio::sync::RwLock<
                    std::collections::HashMap<String, Arc<tokio::sync::RwLock<zeus_agent::Agent>>>,
                >,
            > = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

            // S40 Track A: Message debouncer — batches rapid messages from the
            // same author in the same channel within a 1.5s window. Prevents
            // fleet chatter from triggering N separate agent loops.
            let debouncer_config = zeus_channels::debouncer::DebouncerConfig::default();
            let (debouncer, mut debounced_rx) =
                zeus_channels::debouncer::MessageDebouncer::new(debouncer_config);

            // Feeder task: raw channel messages → debouncer
            tasks.push(tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    // P2 observability: stable greppable inbound-message line.
                    info!(
                        target: "msg",
                        dir = "in",
                        channel = %msg.source.channel_type,
                        peer = %msg.source.user_id,
                        len = msg.content.len(),
                        "msg in"
                    );
                    debouncer.push(msg).await;
                }
                Ok(())
            }));

            let inbox_for_channel = agent_inbox.clone();
            let channel_default_agent_id = default_agent_id.clone();
            // #13-B: clone prometheus config before spawn so it remains available
            // for the heartbeat fallback check at line ~2726.
            // #13-B: resolve cooking-loop timeout before spawn (avoids partial move of config.prometheus).
            let resolved_cooking_timeout_secs: u64 = config.prometheus.as_ref()
                .map(|p| zeus_core::resolve_cooking_loop_timeout(p, gateway.timeout_secs).as_secs())
                .unwrap_or_else(|| if gateway.timeout_secs > 0 { gateway.timeout_secs } else { 1800 });
            // Election liveness: track which peer agents we've recently seen messages from.
            // Used by check_mention_full_with_presence to skip offline/wedged peers in the
            // alphabetical winner selection for role/broadcast mentions.
            let presence = crate::presence_tracker::PresenceTracker::new();
            tasks.push(tokio::spawn(async move {
                info!("Channel message consumer started");
                if !enable_agent_processing {
                    info!("Agent processing DISABLED — relay-only mode (set [gateway] enable_agent_processing = true to re-enable)");
                }

                // T3b: Response prefix for agent identity in fleet channels.
                // Resolves {agent_name} template from network config.
                let response_prefix: Option<String> = config_for_threads
                    .gateway
                    .as_ref()
                    .and_then(|g| g.response_prefix.as_ref())
                    .map(|prefix| {
                        let agent_name = config_for_threads.agent.as_ref().and_then(|a| a.name.as_deref())
                            .or_else(|| config_for_threads.name.as_deref())
                            .or_else(|| config_for_threads.network.as_ref().and_then(|n| n.agent_name.as_deref()))
                            .unwrap_or("Zeus");
                        prefix.replace("{agent_name}", agent_name)
                    });

                // Track consecutive errors per agent for auto-session-reset.
                // Key: agent_id (or "default"), Value: consecutive error count.
                let mut error_streaks: std::collections::HashMap<String, u32> =
                    std::collections::HashMap::new();
                const MAX_CONSECUTIVE_ERRORS: u32 = 3;

                // Message queue depth tracking — logs when messages queue up
                // behind a slow cooking loop, informing future optimization.
                let mut pending_since: Option<std::time::Instant> = None;

                // Mid-loop interrupt: when a new message interrupts an in-progress cook,
                // the interrupted batch is stored here and processed first on the next loop
                // iteration instead of being lost.
                let mut pending_interrupt_batch: Option<zeus_channels::debouncer::MessageBatch> = None;

                loop {
                    let mut batch = if let Some(p) = pending_interrupt_batch.take() {
                        p
                    } else {
                        match debounced_rx.recv().await {
                            Some(b) => b,
                            None => break,
                        }
                    };
                    if let Some(since) = pending_since.take() {
                        let wait_ms = since.elapsed().as_millis();
                        if wait_ms > 1000 {
                            info!("Message waited {}ms in queue (cooking was busy)", wait_ms);
                        }
                        // Staleness guard: if message queued >5min, inject context so agent
                        // knows it's responding to an old message, not a live one.
                        if wait_ms > 300_000 {
                            let stale_mins = wait_ms as f64 / 60_000.0;
                            for m in batch.messages.iter_mut() {
                                m.content = format!(
                                    "⚠️ Note: this message was sent {:.0}m ago (was queued behind other work).\n\n{}",
                                    stale_mins, m.content
                                );
                            }
                            info!("Injected staleness context ({:.0}m delay) into batch", stale_mins);
                        }
                    }
                    // S53-T2: Debouncer now emits MessageBatch preserving all messages.
                    // Convert to a single ChannelMessage for the gateway consumer,
                    // using combined content so the LLM sees ALL batched messages.
                    let batch_len = batch.len();
                    let msg = if batch_len == 1 {
                        match batch.messages.into_iter().next() {
                            Some(m) => m,
                            None => continue,
                        }
                    } else {
                        let combined_content = batch.combined_content();
                        let mut base = match batch.messages.into_iter().last() {
                            Some(b) => b,
                            None => continue,
                        };
                        base.content = combined_content;
                        info!("Debouncer: processing batch of {} messages", batch_len);
                        base
                    };

                    let preview = if msg.content.len() > 50 {
                        let mut end = 50;
                        while !msg.content.is_char_boundary(end) && end < msg.content.len() {
                            end += 1;
                        }
                        &msg.content[..end]
                    } else {
                        &msg.content
                    };
                    info!(
                        "Received message from {}/{} [sender_type={}]: {}",
                        msg.source.channel_type(),
                        msg.source.user_id,
                        msg.source.sender_type,
                        preview
                    );

                    // S63: Push to office broadcast for live visualization
                    {
                        let state = api_state_for_rx.read().await;
                        state.office_broadcast.send(zeus_api::OfficeMessage {
                            sender_id: msg.source.user_id.clone(),
                            sender_name: msg.source.user_id.clone(), // TODO: resolve display name
                            channel_type: msg.source.channel_type().to_string(),
                            content: msg.content.chars().take(100).collect(),
                            timestamp: msg.timestamp.to_rfc3339(),
                        });
                    }

                    // Cache channel messages to SQLite for context across restarts (S52-T2, #317)
                    {
                        let channel_type = msg.source.channel_type();
                        let chat_id = msg.source.chat_id.clone().unwrap_or_default();
                        // Prefix key with channel_type to avoid cross-channel collisions
                        let prefixed_channel_id = format!("{}:{}", channel_type, chat_id);
                        let cached = zeus_api::CachedMessage {
                            id: msg.platform_message_id.clone().unwrap_or_else(|| msg.id.clone()),
                            channel_id: prefixed_channel_id,
                            author_id: msg.source.user_id.clone(),
                            author_name: String::new(), // extracted from content prefix "[Name]: ..."
                            content: msg.content.clone(),
                            timestamp: msg.timestamp.timestamp(),
                            is_bot: msg.source.sender_type.is_bot(),
                        };
                        let state = api_state_for_rx.read().await;
                        state.discord_history.insert(&cached).await;

                        // S93→S94: Track ALL sender presence for Office agent discovery
                        // Humans get tracked too with agent_type: "human"
                        {
                            let sender_id = msg.source.user_id.clone();
                            let is_bot = msg.source.sender_type.is_bot();
                            let sender_name = if is_bot {
                                // Bots: extract name from "[BotName]: ..." prefix
                                msg.content.split(']').next()
                                    .and_then(|s| s.strip_prefix('['))
                                    .unwrap_or(&sender_id)
                                    .to_string()
                            } else {
                                // Humans: use user_id (Discord username)
                                sender_id.clone()
                            };
                            let task_hint = msg.content.chars().take(80).collect::<String>();
                            let agent_type = if is_bot { "bot" } else { "human" };
                            // Election liveness: stamp this peer as recently-seen so the
                            // alphabetical election filters them in. Only bots count —
                            // humans aren't election candidates.
                            if is_bot {
                                presence.record_seen(&sender_name);
                            }
                            let mut presence_state = state.channel_presence.write().await;
                            presence_state.insert(sender_id.clone(), zeus_api::ChannelAgent {
                                id: sender_id,
                                name: sender_name,
                                last_seen: chrono::Utc::now().timestamp(),
                                last_message: task_hint,
                                status: "active".to_string(),
                                agent_type: agent_type.to_string(),
                            });
                        }
                    }

                    // S65: In multi-agent setups, skip messages not addressed to this agent.
                    // Single-agent setups process all messages (backward compatible).
                    // Bot filtering is handled by the channel adapter's allow_bots config.
                    // No additional gateway-level filtering — fleet agents must communicate freely.
                    // Self-echo prevention is already in Layer 1 (discord.rs).

                    // Account-based routing (S35): one gateway → N bot identities → N agents.
                    // Track B tags inbound messages with `account_id` from
                    // `[channels.discord.accounts.*]` config keys. Convention: agent_id == account_id.
                    let (account_routed_agent, account_routed_id) = if let Some(ref acct_id) = msg.source.account_id {
                        let mut state = api_state_for_rx.write().await;
                        if let Some(instance) = state.agent_registry.route_by_account(acct_id) {
                            let agent_arc = instance.agent.clone();
                            let agent_id = instance.agent_id.clone();
                            state.agent_registry.update_activity(&agent_id);
                            info!(
                                "Routed to agent '{}' via account_id '{}'",
                                agent_id, acct_id
                            );
                            (Some(agent_arc), Some(agent_id))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                    // Try binding-based routing via agent registry
                    let (routed_agent, routed_id) = {
                        let mut state = api_state_for_rx.write().await;
                        let matched = state.agent_registry.route(
                            msg.source.channel_type(),
                            &msg.source.user_id,
                            msg.source.chat_id.as_deref().unwrap_or(""),
                        );
                        if let Some(instance) = matched {
                            let agent_arc = instance.agent.clone();
                            let agent_id = instance.agent_id.clone();
                            state.agent_registry.update_activity(&agent_id);
                            info!("Routed to agent '{}' via binding", agent_id);
                            (Some(agent_arc), Some(agent_id))
                        } else {
                            (None, None)
                        }
                    };

                    // Inject active goals from Prometheus before running
                    if let Some(ref prom) = prometheus_for_rx {
                        let prom_guard = prom.read().await;
                        let goals = prom_guard.active_goals_summary();
                        if !goals.is_empty() {
                            let mut agent_guard = agent_for_rx.write().await;
                            agent_guard.set_goals_context(Some(goals.join("\n")));
                        }
                    }

                    // Inject pending tasks from TaskStore (task-driven autonomy).
                    // #168 Phase 4: also capture the formatted string into an outer
                    // binding so it survives to the `cook_with_history_interruptible`
                    // call below and threads onto the prom cook path (which bypasses
                    // the agent's `tasks_context`, where this string would otherwise die).
                    let pending_tasks_ctx: Option<String> = {
                        let state_guard = api_state_for_rx.read().await;
                        let active_tasks = state_guard.task_store.get_active_tasks().await;
                        if !active_tasks.is_empty() {
                            let task_lines: Vec<String> = active_tasks.iter().map(|t| {
                                format!("- [{}] {} (iter {}/{}, branch: {})",
                                    t.status, t.description,
                                    t.iterations_used, t.iterations_budget,
                                    if t.branch.is_empty() { "none" } else { &t.branch })
                            }).collect();
                            let tasks_str = format!(
                                "You have {} active task(s):\n{}",
                                active_tasks.len(),
                                task_lines.join("\n")
                            );
                            let mut agent_guard = agent_for_rx.write().await;
                            agent_guard.set_tasks_context(Some(tasks_str.clone()));
                            Some(tasks_str)
                        } else {
                            None
                        }
                    };

                    // Skip agent processing if disabled (relay-only mode)
                    if !enable_agent_processing {
                        info!("Agent processing disabled — message relayed only (relay-only mode)");
                        continue;
                    }

                    // S84: Activation filter — mentions-only mode.
                    // When gateway.mentions_only = true, ALL messages (bot AND human)
                    // require @mention to trigger cooking. When false (default), only
                    // bot messages are filtered (human messages always trigger).
                    // Non-mentioned messages are saved to session for context.
                    let mentions_only = config_for_threads.gateway.as_ref()
                        .map(|g| g.mentions_only)
                        .unwrap_or(false);
                    // #69-B: Channel-adapter pre-classification short-circuit.
                    //
                    // When the channel adapter (e.g. discord.rs) has already classified
                    // a message as addressed via `ChannelMessage::with_addressed(true)` —
                    // for example, a peer-bot message under `allow_bots = "on"` per the
                    // #69 fix in compute_is_addressed — respect that classification and
                    // skip the content-based mention re-derivation below. Otherwise the
                    // mention filter would drop peer-bot messages that don't @-mention
                    // us, defeating the allow_bots=on semantics.
                    let pre_addressed = msg.is_addressed == Some(true);
                    if pre_addressed {
                        info!(
                            "Message pre-classified addressed by channel adapter (from {}, sender_type={}) — bypassing content-based mention filter",
                            msg.source.user_id, msg.source.sender_type
                        );
                    }
                    // ── Mention filter: only process messages addressed to this agent ──
                    if !pre_addressed {
                        // #296: non-matching sentinel, never the match-all "zeus".
                        let agent_name = config_for_threads.agent.as_ref().and_then(|a| a.name.as_deref())
                            .or_else(|| config_for_threads.name.as_deref())
                            .or_else(|| config_for_threads.network.as_ref().and_then(|n| n.agent_name.as_deref()))
                            .unwrap_or("<unnamed agent>");
                        let bot_snowflake = crate::gateway_consumer::decode_bot_snowflake();
                        let role_ids = config_for_threads.gateway.as_ref()
                            .map(|g| g.discord_role_ids.as_slice())
                            .unwrap_or(&[]);
                        let peer_agent_names = config_for_threads.gateway.as_ref()
                            .map(|g| g.peer_agent_names.as_slice())
                            .unwrap_or(&[]);

                        // DMs have no chat_id (e.g. IRC PRIVMSG to our nick, iMessage 1:1,
                        // Signal 1:1). They are always implicitly addressed to the recipient.
                        let is_dm = msg.source.chat_id.is_none();
                        // Election liveness: filter peers down to those we've seen recently.
                        // Self is always live (PresenceTracker contract). Empty peer list
                        // skips the filter (preserved by check_mention_full_with_presence).
                        let peer_strings: Vec<String> = peer_agent_names.iter().cloned().collect();
                        let live = presence.live_peers(
                            &peer_strings,
                            agent_name,
                            std::time::Duration::from_secs(
                                crate::presence_tracker::DEFAULT_STALENESS_SECS,
                            ),
                        );
                        match crate::gateway_consumer::check_mention_full_with_presence(
                            &msg.content, agent_name, &bot_snowflake, role_ids,
                            peer_agent_names, Some(&live), is_dm,
                        ) {
                            crate::gateway_consumer::MentionCheck::ContextOnly => {
                                debug!(
                                    "Message not addressed to {} — context only, skipping session (from {})",
                                    agent_name, msg.source.user_id
                                );
                                // Do NOT add to session — this creates unpaired user messages
                                // (user, user, user...) that crash strict APIs like Kimi K2.5.
                                // Channel history is already cached to SQLite (line ~1000) and
                                // gets re-injected via inject_channel_history() when addressed.
                                continue;
                            }
                            crate::gateway_consumer::MentionCheck::Addressed {
                                is_mentioned, is_role_mentioned, is_broadcast,
                            } => {
                                info!("Message addressed to {} — processing (mentioned={}, role={}, broadcast={})",
                                    agent_name, is_mentioned, is_role_mentioned, is_broadcast);
                            }
                        }
                    }

                    // S79: Filter HEARTBEAT_OK messages
                    if crate::gateway_consumer::is_heartbeat_ok(&msg.content) {
                        debug!("Filtering HEARTBEAT_OK message — not processing");
                        continue;
                    }
                    // S65: /stop command — immediately terminate any active cooking
                    if crate::gateway_consumer::is_stop_command(&msg.content) {
                        info!("Stop command received — acknowledging");
                        let stop_msg = if let Some(ref prefix) = response_prefix {
                            format!("{} Stopped.", prefix)
                        } else {
                            "Stopped.".to_string()
                        };
                        let agent_guard = agent_for_rx.read().await;
                        agent_guard.send_to_channel(&msg.source, &stop_msg).await;
                        // #3: make stand-down DURABLE — write the ~/.zeus/PAUSE sentinel so
                        // the heartbeat stops firing report-cooks until this seat is re-tasked.
                        // (Was transient before: the next heartbeat tick fired as if nothing
                        // happened.) Explicit-operator action only; cleared on the next
                        // addressed task below, so it can never pause the seat forever.
                        let pause_path = dirs::home_dir().unwrap_or_default().join(".zeus").join("PAUSE");
                        match std::fs::write(&pause_path, "stood down via stop command\n") {
                            Ok(()) => info!("Stand-down: wrote PAUSE sentinel {}", pause_path.display()),
                            Err(e) => tracing::warn!("Failed to write PAUSE sentinel {}: {}", pause_path.display(), e),
                        }
                        continue;
                    }

                    // #3: a real ADDRESSED message (ContextOnly was already filtered above)
                    // resumes the seat — clear any PAUSE sentinel so the heartbeat and
                    // work-drive come back online. This is the code-enforced terminal gate:
                    // stand-down cannot persist past the next task addressed to this seat.
                    {
                        let pause_path = dirs::home_dir().unwrap_or_default().join(".zeus").join("PAUSE");
                        if pause_path.exists() {
                            let _ = std::fs::remove_file(&pause_path);
                            info!("Resume: cleared PAUSE sentinel on addressed inbound message");
                        }
                    }

                    // The default agent owns all channel adapters (Discord/Telegram/etc).
                    // Registry agents have empty channels — always use the default agent
                    // for sends so messages go through the correct adapter.
                    let channel_agent = agent_for_rx.clone();

                    // Thread-bound subagent routing: if this message is in a thread,
                    // route to a dedicated agent for that thread (isolated context)
                    let thread_agent = if let Some(ref tid) = msg.source.thread_id {
                        let thread_key = format!("{}:{}", msg.source.channel_type(), tid);
                        let agents = thread_agents.read().await;
                        if let Some(ta) = agents.get(&thread_key) {
                            Some(ta.clone())
                        } else {
                            drop(agents); // Release read lock before write
                            // Spawn new thread-bound agent
                            let tc = config_for_threads.clone();
                            match zeus_llm::LlmClient::from_config(&tc) {
                                Ok(llm) => {
                                    let ws = zeus_memory::Workspace::from_config(&tc);
                                    let dm_scope = tc.gateway.as_ref()
                                        .map(|g| g.dm_scope.as_str())
                                        .unwrap_or("main");
                                    let stable_id = if dm_scope == "main" {
                                        "primary".to_string()
                                    } else {
                                        format!("thread-{}", tid)
                                    };
                                    let sess = if dm_scope == "main" {
                                        zeus_session::Session::get_or_create_labeled(&tc.sessions, "primary").await
                                    } else {
                                        zeus_session::Session::resume_or_create(&tc.sessions, &stable_id).await
                                    };
                                    // Wire the gateway's main-agent ChannelManager into the
                                    // thread-bound agent so its `message` tool can dispatch to
                                    // platform adapters (Discord/Telegram/Slack/X/etc.). Without
                                    // this, thread-routed inbound messages would hit
                                    // `Channel::Unknown` fallthrough at tools.rs:626 for any
                                    // platform channel — the load-bearing fix for the
                                    // cross-channel-amnesia + message-tool-broken regression.
                                    let thread_channels = agent_for_rx.read().await.channel_manager();
                                    let mut ta = zeus_agent::Agent::new(tc, llm, ws, sess, thread_channels);
                                    ta.set_goals_context(Some(format!(
                                        "You are a thread-bound agent for thread {}. \
                                         Maintain context within this thread only.",
                                        tid
                                    )));
                                    let ta = Arc::new(tokio::sync::RwLock::new(ta));
                                    let mut agents_w = thread_agents.write().await;
                                    agents_w.insert(thread_key.clone(), ta.clone());
                                    // Cap thread agents at 100 to prevent unbounded growth.
                                    // Evict oldest entries when limit exceeded.
                                    if agents_w.len() > 100 {
                                        let to_remove: Vec<String> = agents_w
                                            .keys()
                                            .take(agents_w.len() - 100)
                                            .cloned()
                                            .collect();
                                        for key in &to_remove {
                                            agents_w.remove(key);
                                        }
                                        info!("Evicted {} stale thread agents", to_remove.len());
                                    }
                                    drop(agents_w);
                                    info!("Spawned thread-bound agent for {}", thread_key);
                                    Some(ta)
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to create thread agent for {}: {}",
                                        thread_key, e
                                    );
                                    None
                                }
                            }
                        }
                    } else {
                        None
                    };

                    // Resolve final dispatch agent: account-routed > thread > binding > default.
                    // This Arc is reused for all subsequent reactions and send_to_channel so
                    // that replies always originate from the correct bot identity. (S36 Track C)
                    let dispatch_agent = account_routed_agent
                        .as_ref()
                        .or(thread_agent.as_ref())
                        .or(routed_agent.as_ref())
                        .cloned()
                        .unwrap_or_else(|| agent_for_rx.clone());
                    let dispatch_agent_id = account_routed_id
                        .as_deref()
                        .or(routed_id.as_deref())
                        .unwrap_or("default")
                        .to_string();

                    // Unified Titan session swap — all channel surfaces for this Titan
                    // use one Session file, while channel provenance stays on `msg.source`
                    // for replies/audit. Skipped when no chat_id is available (e.g.
                    // internal API calls), in which case dispatch_agent keeps its existing
                    // fallback session.
                    // #192b P1 (2b): resolve the per-cook session WITHOUT swapping the
                    // shared agent's field. The Titan-scoped session id is resolved from
                    // the same router the swap used; when no chat_id is available we fall
                    // back to the agent's current session id. Either way we load from the
                    // #192 shared store (resume_or_create reads the same on-disk session),
                    // so history + persistence are byte-identical to the pre-P1 swap —
                    // but the cook never takes dispatch_agent.write() (the inner lane gone).
                    let cook_session_id = if msg.source.chat_id.is_some() {
                        let scope = ContextScope::titan(channel_default_agent_id.as_str());
                        let session_id = channel_session_router_for_rx.resolve_context(&scope).await;
                        debug!(
                            channel_type = %msg.source.channel_type(),
                            chat_id = ?msg.source.chat_id.as_deref(),
                            session_id = %session_id,
                            "Titan-scoped channel session resolved (cook_ctx)"
                        );
                        session_id
                    } else {
                        let guard = dispatch_agent.read().await;
                        guard.session().id.clone()
                    };
                    let mut cook_ctx = zeus_prometheus::CookContext::new(
                        Session::resume_or_create(
                            &sessions_dir_for_rx,
                            &cook_session_id,
                        ).await,
                    );

                    // Channel prompt from config.toml (not hardcoded)
                    let effective_content = crate::gateway_consumer::build_final_content(
                        &msg.content,
                        msg.source.chat_id.as_deref(),
                        msg.source.channel_type(),
                        gateway.channel_prompt.as_deref(),
                    );

                    // Process attachments: images/PDFs → LLM content blocks,
                    // audio → Whisper STT transcription, text → inline extraction.
                    let whisper_url = config_for_threads.deployment.as_ref()
                        .and_then(|d| d.whisper_stt_url.clone())
                        .unwrap_or_default();
                    let (core_attachments, extra_context) = crate::gateway_consumer::process_attachments(
                        &msg.attachments, &whisper_url,
                    ).await;

                    // Save original message for intent classification (before history injection)
                    let original_message = effective_content.clone();

                    // Prepend extracted text/transcriptions to the message
                    let final_content = if extra_context.is_empty() {
                        effective_content
                    } else {
                        format!("{}{}", effective_content, extra_context)
                    };

                    // S53: Inject recent channel history as context
                    // #192b P1 (2b): read the per-cook session, not the shared agent field.
                    let session_msg_count = cook_ctx.session().messages.len();
                    let final_content = {
                        let state = api_state_for_rx.read().await;
                        let own_bot_id = crate::gateway_consumer::decode_bot_snowflake();
                        crate::gateway_consumer::inject_channel_history(
                            final_content,
                            msg.source.channel_type(),
                            msg.source.chat_id.as_deref().unwrap_or(""),
                            msg.platform_message_id.as_deref(),
                            session_msg_count,
                            &own_bot_id,
                            &state.discord_history,
                        ).await
                    };

                    // Classify intent on the ORIGINAL message only — not the
                    // history-enriched final_content. History context contains old
                    // tool instructions that would misclassify as ToolUse/Complex.
                    let intent_input = original_message;

                    // Intent-based prompt injection: tell the LLM whether this is
                    // a question (answer directly) or a task (execute with tools).
                    // Prevents "task complete" framing on Q&A like "what is illumos?"
                    let msg_trimmed = intent_input.trim();
                    let msg_lower = msg_trimmed.to_lowercase();
                    let is_question = msg_trimmed.ends_with('?')
                            || msg_lower.starts_with("what ")
                            || msg_lower.starts_with("how ")
                            || msg_lower.starts_with("why ")
                            || msg_lower.starts_with("where ")
                            || msg_lower.starts_with("when ")
                            || msg_lower.starts_with("who ")
                            || msg_lower.starts_with("which ")
                            || msg_lower.starts_with("is ")
                            || msg_lower.starts_with("are ")
                            || msg_lower.starts_with("does ")
                            || msg_lower.starts_with("do ")
                            || msg_lower.starts_with("can ")
                            || msg_lower.starts_with("could ")
                            || msg_lower.starts_with("would ")
                            || msg_lower.starts_with("should ")
                            || msg_lower.starts_with("tell me ")
                            || msg_lower.starts_with("explain ");
                    let final_content = {
                        if is_question {
                            debug!("Intent heuristic: QUESTION — injecting direct-answer prompt");
                            format!(
                                "[INTENT: QUESTION — answer this question directly and concisely. \
                                 Do NOT frame your response as a task status update or 'task complete'. \
                                 Just answer the question.]\n\n{}",
                                final_content
                            )
                        } else {
                            final_content
                        }
                    };
                    // Ollama bypass: skip intent classification for Ollama provider.
                    // Ollama is slow — avoid the extra LLM call. Simple messages go
                    // straight through agent.run(), not the cooking loop.
                    let (llm_provider, _) = config_for_threads.parse_model();
                    // Use gateway config timeout (default 1800s). Ollama gets at least 600s.
                    let channel_timeout_secs: u64 = if llm_provider == zeus_core::Provider::Ollama {
                        std::cmp::max(gateway.timeout_secs, 600)
                    } else {
                        gateway.timeout_secs
                    };
                    // Ollama now routes through OpenAI-compat — cooking loop enabled for all providers.
                    let (use_cooking, use_plan_mode) = if let Some(ref prom) = prometheus_for_rx {
                        let prom_guard = prom.read().await;
                        let tool_schemas = {
                            dispatch_agent.read().await.tool_schemas()
                        };
                        let analysis = prom_guard.classify_intent(
                            &intent_input, &tool_schemas,
                        );
                        let dominated_by_complexity = matches!(
                            analysis.complexity,
                            zeus_prometheus::TaskComplexity::Complex
                                | zeus_prometheus::TaskComplexity::Moderate
                        );
                        let is_complex_intent = matches!(
                            analysis.intent,
                            zeus_prometheus::Intent::ComplexTask
                                | zeus_prometheus::Intent::ToolUse
                        );
                        let should_cook = dominated_by_complexity || is_complex_intent;

                        // Wire AutonomyEngine — let it refine the decision
                        let autonomy_decision = {
                            let tool_names: Vec<String> = tool_schemas.iter().map(|t| t.name.clone()).collect();
                            let tool_count = tool_names.len();
                            let decision_ctx = zeus_prometheus::autonomy::DecisionContext {
                                intent: analysis.clone(),
                                has_memory_context: true,
                                session_message_count: 0,
                                recent_error_count: 0,
                                available_tools: tool_names,
                                autonomous_tool_count: tool_count,
                            };
                            let engine = zeus_prometheus::autonomy::AutonomyEngine::default();
                            engine.decide(&decision_ctx)
                        };

                        // AutonomyEngine can override: Reflect → cook, RespondDirectly → skip cook
                        let should_cook = match &autonomy_decision {
                            zeus_prometheus::autonomy::Decision::Reflect => {
                                info!("Autonomy: reflecting (error recovery)");
                                true // cook with reflection prompt
                            }
                            zeus_prometheus::autonomy::Decision::RespondDirectly(_) => {
                                false // simple response, no cooking needed
                            }
                            _ => should_cook, // use original decision
                        };

                        // Plan Mode: Complex tasks from HUMANS get a written plan before execution.
                        // Moderate tasks cook directly (no plan overhead).
                        // Bot/agent messages NEVER trigger plan mode — they're status updates,
                        // acknowledgements, and coordination chatter, not task assignments.
                        // This prevents the infinite plan resume loop where conversational
                        // agent messages create short-lived plans that heartbeat resumes.
                        let sender_is_bot_for_plan = msg.source.sender_type.is_bot();
                        // #157 creation-side gate: do not persist a resumable plan for
                        // ephemeral, question-shaped conversational fragments. Both new
                        // clauses only SUBTRACT from the already-narrow Complex+ComplexTask
                        // set, so they cannot over-block a legit goal-plan (76ca3c42 guard):
                        //  - confidence >= 0.65 reuses the classifier's OWN single-clause
                        //    tell (0.6 for single-clause, 0.75 for multi-clause goals).
                        //  - !is_interrogative filters question-led / ?-only fragments that
                        //    lack an imperative goal. A real "build/implement X and Y" goal is
                        //    imperative + multi-clause → unaffected. Filtered messages still
                        //    cook; they just don't persist a resumable plan dir.
                        let needs_plan = !sender_is_bot_for_plan
                            && analysis.confidence >= 0.65
                            && !zeus_prometheus::intent::is_interrogative(&intent_input)
                            && matches!(
                                analysis.complexity,
                                zeus_prometheus::TaskComplexity::Complex
                            ) && matches!(
                                analysis.intent,
                                zeus_prometheus::Intent::ComplexTask
                            );

                        if should_cook {
                            let sender_is_bot = msg.source.sender_type.is_bot();
                            info!(
                                "Cooking loop engaged: intent={}, complexity={:?}, plan_mode={}, cap=default (bot_sender={})",
                                analysis.intent, analysis.complexity, needs_plan, sender_is_bot,
                            );
                        }
                        (should_cook, needs_plan)
                    } else {
                        (false, false)
                    };

                    // Attachments are now supported in cooking — image content blocks are
                    // injected into the first user message via Message::user_with_attachments.
                    let use_plan_mode = use_plan_mode && use_cooking;

                    // Skip re-cooking if task already delivered and message is chatter
                    let skip_already_delivered = crate::gateway_consumer::check_task_completed(
                        cook_ctx.session().messages.as_slice(), &intent_input,
                    );

                    if skip_already_delivered {
                        info!("Skipping re-cook — task already delivered, incoming message is chatter");
                        // RAII: no guard held yet in this branch, nothing to clear.
                        continue;
                    }

                    // Clear the TASK_COMPLETED marker — new task is starting
                    // (marker will be re-added after this response is delivered)

                    // S-PRIORITY: try to acquire Channel cook slot.
                    // If heartbeat is cooking, `try_acquire` returns None → the existing
                    // queue path (see L~1591) drains the message after the current cook.
                    // Stop commands bypass the queue (unchanged behavior).
                    let channel_cook_guard = match cook_state_for_rx.try_acquire(zeus_core::ActiveCookType::Channel) {
                        Some(g) => g,
                        None => {
                            debug!("Channel cook deferred — another cook in flight; message queued");
                            // The debounced consumer loop will re-pick this up after drain.
                            continue;
                        }
                    };
                    // #192b P2: mark the channel in-flight counter alongside the CAS
                    // guard (same lifetime — drops with `channel_cook_guard` at cook
                    // end). This is what makes the migrated heartbeat/autonomy deferral
                    // reads (`is_channel_cooking()` / `any_cook_inflight()`) honest: the
                    // CAS alone is invisible to the counter. Scoped to exactly the
                    // cook's lifetime so `channel_inflight` decrements when this cook
                    // finishes. (The per-message spawn for true cross-session
                    // concurrency is the final sub-cook; this keeps the counter correct
                    // in the meantime with zero behavior change to the channel path.)
                    let _channel_flight = cook_state_for_rx.begin_cook(zeus_core::ActiveCookType::Channel);

                    // #192b P2c seam: acquire this session's lane. Same-session cooks
                    // serialize FIFO here; different sessions take different lanes.
                    // Held for the cook's duration (dropped at end of this iteration
                    // alongside `channel_cook_guard`). The per-message `tokio::spawn`
                    // that moves this guard into the task — enabling true cross-session
                    // concurrency — is the next sub-cook; this seam wires the primitive
                    // and preserves byte-identical serial behavior until then.
                    let _channel_lane = channel_lane_mgr.lane(&cook_session_id).lock_owned().await;

                    let (provider, _) = config_for_threads.parse_model();
                    // Always use full path with tools — no run_fast gating.
                    // Model decides whether to call tools or respond with text.
                    let response: Result<String, zeus_core::Error> = if use_cooking {
                        // Complex task → Prometheus cooking loop (multi-iteration)

                        // Issue a fresh cancel token for this cook (cancels any prior in-flight cook).
                        let cook_token = {
                            let mut guard = cook_cancel_for_rx.lock().await;
                            if let Some(old) = guard.take() {
                                old.cancel();
                                debug!("Cancelled in-flight cook for new channel message");
                            }
                            let token = tokio_util::sync::CancellationToken::new();
                            *guard = Some(token.clone());
                            token
                        };

                        // Mid-loop interrupt lane: gateway routes by resolved session key
                        // through SessionLaneManager (mpsc, not broadcast). The guard
                        // clears the lane on every cook exit/drop/timeout path.
                        let (interrupt_rx, _interrupt_lane_guard) =
                            channel_lane_mgr.open_interrupt_lane(cook_session_id.clone());

                        // Clone the cancel token so both the cooking_future (which captures it
                        // by move) and the select! branch below can call .cancel() on it.
                        let cook_token_for_interrupt = cook_token.clone();

                        // Tier 1: keep channels showing "typing..." during dispatch cooking.
                        // Guard cancels the heartbeat on any return path from the select! below.
                        let _typing_guard = {
                            let agent_guard = dispatch_agent.read().await;
                            let channels = agent_guard.channel_manager();
                            drop(agent_guard);
                            spawn_typing_heartbeat(channels, Some(channels_source_to_core(&msg.source)))
                        };

                        // #284B: last-activity signal. The cooking loop stores unix-secs
                        // for model text responses and completed tool calls; the timeout arm
                        // kills only after a full idle window with no observed activity.
                        let initial_activity_at = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let progress_signal = std::sync::Arc::new(
                            std::sync::atomic::AtomicU64::new(initial_activity_at),
                        );
                        let progress_signal_for_cook = progress_signal.clone();

                        // #192b P1 (2b): snapshot the per-cook session id + history BEFORE the
                        // cooking_future so the future captures owned values, not a borrow of
                        // cook_ctx (which must stay free for session_mut() persist after the cook).
                        let cook_session_id_snapshot = cook_ctx.session().id.clone();
                        let cook_recent_history = cook_ctx.session().recent_history_for_llm();

                        let cooking_future = async {
                            let prom_ref = prometheus_for_rx.as_ref().ok_or_else(|| zeus_core::Error::Config("Prometheus not initialized — cannot process channel messages".into()))?;
                            let prom_guard = prom_ref.read().await;
                            let (tool_schemas, session_history) = {
                                let agent_guard = dispatch_agent.read().await;
                                // Context-aware tool loading: includes core tools + configured
                                // integrations only. No hardcoded provider checks.
                                let schemas = agent_guard.context_schemas();
                                // S79: Compact session history before injecting into cooking loop.
                                // Two-phase: strip tool outputs, then summarize if over threshold.
                                // #192b P1 (2b): use the pre-cook snapshot (owned), so the
                                // future never borrows cook_ctx — leaving it free for persist.
                                let session_id = cook_session_id_snapshot.clone();
                                // #52: migrate to #51 engine seam — replaces legacy len()>50 slicing.
                                let recent = cook_recent_history.clone();
                                // C2: Write context journal before compaction (captures structured task state)
                                let journal_dir = config_for_threads.sessions.join("journals");
                                let journal = zeus_session::ContextJournal::new(journal_dir, 10);
                                let _journal_path = journal.write_journal(
                                    &session_id, &recent, 180_000,
                                ).unwrap_or_else(|e| {
                                    debug!("Journal write skipped (non-fatal): {}", e);
                                    std::path::PathBuf::new()
                                });
                                // Try compaction — falls back to raw history on error.
                                // Skip LLM-based compaction for Ollama to avoid extra round-trip latency.
                                let (provider, _) = config_for_threads.parse_model();
                                let mut history = if provider == zeus_core::Provider::Ollama {
                                    recent // Ollama: use raw capped history, skip LLM compaction
                                } else if let Ok(llm) = zeus_llm::LlmClient::from_config(&config_for_threads) {
                                    let compaction_config = zeus_prometheus::compaction::CompactionConfig::default();
                                    match zeus_prometheus::compaction::compact_session_history(
                                        &recent, &compaction_config, &llm,
                                    ).await {
                                        Ok(result) => {
                                            if result.compacted {
                                                info!(
                                                    "Session compacted: {} → {} messages, saved {} tokens",
                                                    result.messages_before, result.messages_after,
                                                    result.tokens_before.saturating_sub(result.tokens_after)
                                                );
                                            }
                                            result.messages
                                        }
                                        Err(e) => {
                                            warn!("Session compaction failed (non-fatal): {}", e);
                                            recent
                                        }
                                    }
                                } else {
                                    recent
                                };
                                // C2: Inject journal state after compaction so agent retains structured task context
                                if let Ok(Some(journal_content)) = journal.read_latest_journal(&session_id) {
                                    if !journal_content.is_empty() {
                                        debug!("Injecting context journal after compaction ({} chars)", journal_content.len());
                                        history.insert(0, zeus_core::Message::system(&format!(
                                            "[Context Journal — task state preserved across compaction]\n{}",
                                            journal_content
                                        )));
                                    }
                                }
                                (schemas, history)
                            };
                            // Q&A context cleaning: strip tool_use/tool_result pairs
                            // from session history when answering a question. Prevents
                            // previous dev work (file reads, shell commands, code edits)
                            // from polluting the LLM context for a simple Q&A answer.
                            let session_history = if is_question {
                                let before = session_history.len();
                                let cleaned: Vec<zeus_core::Message> = session_history.into_iter()
                                    .filter(|m| {
                                        // Keep user + assistant text messages
                                        // Drop tool_use (assistant with tool_calls) and tool_result (role=tool)
                                        m.tool_calls.is_empty() && m.role != zeus_core::Role::Tool
                                    })
                                    .collect();
                                let after = cleaned.len();
                                if before != after {
                                    info!("Q&A context cleaning: stripped {} tool messages from history ({} → {})", before - after, before, after);
                                }
                                cleaned
                            } else {
                                session_history
                            };
                            // #91 Cut 1 — Cross-channel session-tail injection (Option B).
                            // Mirrors #86 Mnemosyne injection pattern: same gateway-side hook,
                            // episodic session tail instead of semantic memory.
                            // #192: default-on at n=5 (same-human continuity across
                            // surfaces; resolver keyed on (agent, human) so no
                            // cross-human bleed). Set cross_channel_session_tail_n=0
                            // under [prometheus] to opt out.
                            let session_history = {
                                let tail_n = config_for_threads.prometheus.as_ref()
                                    .map(|p| p.cross_channel_session_tail_n)
                                    .unwrap_or_else(|| {
                                        zeus_core::PrometheusConfig::default()
                                            .cross_channel_session_tail_n
                                    });
                                if tail_n > 0 {
                                    let current_session_id = {
                                        let guard = dispatch_agent.read().await;
                                        guard.session().id.clone()
                                    };
                                    let tail_store = zeus_session::SessionStore::new(&sessions_dir_for_rx);
                                    let tail_msgs = tail_store
                                        .load_cross_channel_tail(&current_session_id, tail_n)
                                        .await;
                                    if !tail_msgs.is_empty() {
                                        let mut tail_lines = vec!["## Cross-channel session tail".to_string()];
                                        for m in &tail_msgs {
                                            let role_str = match m.role {
                                                zeus_core::Role::User => "user",
                                                zeus_core::Role::Assistant => "assistant",
                                                _ => "system",
                                            };
                                            tail_lines.push(format!("[{}]: {}", role_str, m.content));
                                        }
                                        let tail_block = tail_lines.join("\n");
                                        let tail_msg = zeus_core::Message::system(&tail_block);
                                        let mut injected = vec![tail_msg];
                                        injected.extend(session_history);
                                        debug!(tail_n = tail_msgs.len(), "#91: cross-channel session tail injected");
                                        injected
                                    } else {
                                        session_history
                                    }
                                } else {
                                    session_history
                                }
                            };
                            let alias = {
                                let sessions_guard = prom_guard.sessions().read().await;
                                prom_guard.session_resolver(&sessions_guard, &channel_default_agent_id, Some(&msg.source.user_id), msg.source.channel_type().parse().unwrap_or(zeus_prometheus::ChannelKind::Other("internal".to_string())), chrono::Utc::now()).await
                            };
                            let prior_dispatches = prom_guard.track_dispatch(&alias).await;
                            let channel_kind_display = msg.source.channel_type().parse().unwrap_or(zeus_prometheus::ChannelKind::Other("internal".to_string()));
                            let cook_span = info_span!("cook",
                                fleet_session_alias = %alias,
                                agent = %channel_default_agent_id,
                                human = %msg.source.user_id,
                                channel_kind = %channel_kind_display,
                                surface = "gateway",
                                callsite = "gateway:2048"
                            );
                            let _cook_guard = cook_span.enter();
                            tracing::info!(
                                gate = "fleet_session_correlation",
                                fleet_session_alias = %alias,
                                resolved_via = alias.resolved_via(),
                                prior_dispatches_for_alias = prior_dispatches,
                                agent = %channel_default_agent_id,
                                human = %msg.source.user_id,
                                channel_kind = %channel_kind_display,
                                "resolver decision"
                            );
                            tracing::info!(
                                gate = "cook_dispatched",
                                history_len = session_history.len(),
                                message_len = final_content.len(),
                                "cook entry"
                            );
                            drop(_cook_guard);
                            // Sprint-C (#86): set channel kind so inject_cross_channel
                            // knows what to exclude when querying Mnemosyne.
                            prom_guard.set_current_channel_kind(&msg.source.channel_type());
                            // #192: set human (sender) id so the fleet session resolver
                            // can correlate same-human sessions across surfaces.
                            prom_guard.set_current_human_id(Some(msg.source.user_id.as_str()));
                            let cook_result = if use_plan_mode {
                                info!("Plan mode engaged — generating plan before execution");
                                prom_guard.cook_with_plan(
                                    &final_content, &tool_schemas, &session_history,
                                    Some(cook_token),
                                ).instrument(cook_span.clone()).await?
                            } else {
                                prom_guard.cook_with_history_interruptible(
                                    &final_content, &tool_schemas, &session_history,
                                    Some(cook_token), Some(interrupt_rx),
                                    core_attachments.clone(),
                                    pending_tasks_ctx.clone(), // #168 Phase 4: thread gateway tasks onto cook path
                                    Some(progress_signal_for_cook.clone()), // #284B: activity signal
                                ).instrument(cook_span).await?
                            };
                            info!(
                                "Cooking complete: {} iterations, {} tool calls{}",
                                cook_result.iterations,
                                cook_result.tool_calls.len(),
                                if cook_result.interrupted_by.is_some() { " (interrupted)" } else { "" },
                            );
                            Ok::<String, zeus_core::Error>(cook_result.response)
                        };

                        // Cooking timeout: PrometheusConfig NL/secs > gateway default (1800s).
                        // #13-B: resolved before spawn to avoid partial move of config.prometheus.
                        // #176 H1 / #284B: task-derived idle window — if the dispatch text
                        // states an explicit "for Xh / for the next Xh" intent, use it instead
                        // of the config default, failsafe-capped by cooking_loop_max so a
                        // misparse cannot become an unbounded idle wait. No explicit intent → config default.
                        let cooking_timeout = {
                            let ceiling = config_for_threads
                                .prometheus
                                .as_ref()
                                .map(zeus_core::resolve_cooking_loop_max)
                                .unwrap_or_else(|| std::time::Duration::from_secs(4 * 60 * 60));
                            match zeus_core::extract_task_timeout(&final_content, ceiling) {
                                Some(task_budget) => {
                                    let secs = task_budget.as_secs();
                                    info!(
                                        gate = "cooking_timeout_task_derived",
                                        task_budget_secs = secs,
                                        config_default_secs = resolved_cooking_timeout_secs,
                                        "#176 H1: task-derived cooking budget applied"
                                    );
                                    secs
                                }
                                None => resolved_cooking_timeout_secs,
                            }
                        };
                        tokio::pin!(cooking_future);
                        // #284B: treat the configured cooking timeout as an idle window,
                        // not an absolute cook lifetime. Active cooks re-arm below based on
                        // `last_activity_at`; idle/wedged cooks are killed after this window.
                        let idle_window = std::time::Duration::from_secs(cooking_timeout);
                        let mut timeout_at = tokio::time::Instant::now() + idle_window;
                        let mut interrupt_sent = false;
                        let cook_response = loop {
                            tokio::select! {
                                result = &mut cooking_future => {
                                    break result;
                                }
                                maybe_new = debounced_rx.recv(), if !interrupt_sent => {
                                    if let Some(new_batch) = maybe_new {
                                        let content = new_batch.combined_content();
                                        // Only interrupt on explicit stop/pause/redirect commands
                                        // addressed to this agent. All other messages are queued
                                        // until cooking completes.
                                        if let Some(interrupt_kind) = classify_mid_loop_interrupt(&content) {
                                            match interrupt_kind {
                                                MidLoopInterruptKind::Stop => {
                                                    info!(
                                                        "Mid-loop interrupt: STOP command detected ('{}...'), signalling cook to stop",
                                                        content.chars().take(40).collect::<String>(),
                                                    );
                                                }
                                                MidLoopInterruptKind::Redirect => {
                                                    info!(
                                                        "Mid-loop interrupt: REDIRECT command detected ('{}...'), swapping to queued input",
                                                        content.chars().take(40).collect::<String>(),
                                                    );
                                                }
                                            }
                                            let _ = channel_lane_mgr.send_interrupt(&cook_session_id, content);
                                            cook_token_for_interrupt.cancel();
                                            interrupt_sent = true;
                                        } else {
                                            debug!(
                                                "Message queued during cooking ('{}...')",
                                                content.chars().take(40).collect::<String>(),
                                            );
                                        }
                                        pending_interrupt_batch = Some(new_batch);
                                        // Keep looping — cooking_future will exit at next iteration (if interrupted)
                                        // or queue will be processed after cooking completes
                                    }
                                    // else: channel closed, cooking_future will finish naturally
                                }
                                _ = tokio::time::sleep_until(timeout_at) => {
                                    let now_unix = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0);
                                    let last_activity_at =
                                        progress_signal.load(std::sync::atomic::Ordering::Relaxed);
                                    if zeus_core::should_abort_for_idle(
                                        last_activity_at,
                                        now_unix,
                                        idle_window,
                                    ) {
                                        let idle_for_secs = now_unix.saturating_sub(last_activity_at);
                                        warn!(
                                            "Agent '{}' cooking timed out after {}s idle (idle_window={}s, last_activity_at={})",
                                            dispatch_agent_id,
                                            idle_for_secs,
                                            cooking_timeout,
                                            last_activity_at,
                                        );
                                        break Err(zeus_core::Error::Internal(format!(
                                            "Cooking loop timed out after {}s idle",
                                            cooking_timeout,
                                        )));
                                    }

                                    let idle_for_secs = now_unix.saturating_sub(last_activity_at);
                                    let remaining = idle_window
                                        .saturating_sub(std::time::Duration::from_secs(idle_for_secs));
                                    let rearm_after = if remaining.is_zero() {
                                        std::time::Duration::from_secs(1)
                                    } else {
                                        remaining
                                    };
                                    timeout_at = tokio::time::Instant::now() + rearm_after;
                                    debug!(
                                        agent = %dispatch_agent_id,
                                        idle_for_secs,
                                        rearm_after_secs = rearm_after.as_secs(),
                                        idle_window_secs = cooking_timeout,
                                        "#284B: cook activity observed — re-arming idle watchdog"
                                    );
                                }
                            }
                        };

                        // Persist cooking messages to agent session so context
                        // survives gateway restarts (fixes memory persistence bug).
                        // Also store in Mnemosyne for semantic search (parity with
                        // agent.run() path which stores via store_with_embedding).
                        {
                            // #192b P1 (2b): session persistence runs against the per-cook
                            // cook_ctx (no dispatch_agent.write() — inner lane gone). The
                            // read guard is only for agent-level resources (mnemosyne).
                            let agent_ro = dispatch_agent.read().await;

                            // FIX: If cooking timed out with a pending tool_use, inject
                            // synthetic tool_results so the session stays valid for Anthropic API.
                            // Without this, the next message breaks tool_use→tool_result pairing.
                            if cook_response.is_err() {
                                // Collect orphaned tool_call IDs before mutating session
                                let orphaned_ids: Vec<String> = cook_ctx.session().messages.last()
                                    .map(|m| m.tool_calls.iter().map(|tc| tc.id.clone()).collect())
                                    .unwrap_or_default();
                                if !orphaned_ids.is_empty() {
                                    let tool_results: Vec<zeus_core::ToolResult> = orphaned_ids.iter().map(|id| {
                                        zeus_core::ToolResult {
                                            call_id: id.clone(),
                                            success: false,
                                            output: "[Tool execution interrupted — cooking loop timed out]".to_string(),
                                        }
                                    }).collect();
                                    let mut repair_msg = zeus_core::Message::tool(
                                        &tool_results[0].call_id,
                                        false,
                                        "[Tool execution interrupted — cooking loop timed out]",
                                    );
                                    if tool_results.len() > 1 {
                                        repair_msg.tool_results = tool_results;
                                    }
                                    let _ = cook_ctx.session_mut().add(repair_msg).await;
                                    warn!("Injected synthetic tool_results for {} orphaned tool_use(s) after cooking timeout",
                                        orphaned_ids.len());
                                }
                            }

                            // Tag with channel source AND prefix content so LLM sees the origin
                            let channel_type = msg.source.channel_type().to_string();
                            let sender = msg.source.user_id.clone();
                            // On failure: persist a CLEAN user message (strip attachment noise)
                            // so the session doesn't accumulate toxic context from failed attempts.
                            let persist_content = if cook_response.is_err() {
                                // Strip [Image attachment: ...] and [Audio attachment: ...] lines
                                // from the message — they bloat context on retry and confuse the model.
                                let clean = final_content.lines()
                                    .filter(|line| {
                                        let trimmed = line.trim();
                                        !trimmed.starts_with("[Image attachment:") &&
                                        !trimmed.starts_with("[Audio attachment:") &&
                                        !trimmed.starts_with("[File:")
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                if clean.trim().is_empty() {
                                    // Message was ONLY attachments — don't persist empty noise
                                    String::new()
                                } else {
                                    clean
                                }
                            } else {
                                final_content.clone()
                            };

                            // Only persist to session if there's meaningful content
                            // (skip pure-attachment messages that failed — they poison context)
                            if !persist_content.trim().is_empty() {
                                let tagged_content = format!("[{} | {}] {}", channel_type, sender, persist_content);
                                let user_msg = zeus_core::Message::user(&tagged_content)
                                    .with_channel_source(zeus_core::ChannelSource {
                                        channel_type: channel_type.clone(),
                                        channel_id: msg.source.chat_id.clone(),
                                        channel_name: None,
                                        sender_name: Some(sender.clone()),
                                        sender_id: Some(msg.source.user_id.clone()),
                                    });
                                if let Err(e) = cook_ctx.session_mut().add(user_msg.clone()).await {
                                    warn!("Failed to persist cooking user msg to session: {}", e);
                                }
                                // On success: persist assistant response normally.
                                // On failure: do NOT persist failure markers — they accumulate
                                // and degrade context quality. The agent recovers on the next
                                // valid message with a clean session.
                                if let Ok(ref response_text) = cook_response {
                                    let assistant_msg = zeus_core::Message::assistant(response_text);
                                    if let Err(e) = cook_ctx.session_mut().add(assistant_msg.clone()).await {
                                        warn!("Failed to persist cooking response to session: {}", e);
                                    }
                                    // Store in Mnemosyne for cross-session semantic search (success only)
                                    if let Some(mnemosyne) = agent_ro.mnemosyne() {
                                        let session_id = cook_ctx.session().id.clone();
                                        // Sprint-B (#86): tag with channel provenance for cross-channel context
                                        let ck = Some(channel_type.as_str());
                                        let cid = msg.source.chat_id.as_deref();
                                        let _ = mnemosyne.store_with_embedding_tagged(&session_id, &user_msg, ck, cid).await;
                                        let _ = mnemosyne.store_with_embedding_tagged(&session_id, &assistant_msg, ck, cid).await;
                                    }
                                } else {
                                    warn!("Cooking failed — skipping session persistence to prevent context pollution");
                                }
                            } else {
                                info!("Skipping session persistence for failed attachment-only message (no text content)");
                            }
                        }

                        // Continuation: if the cooking loop completed but the response
                        // indicates more work is needed, re-enter the cooking loop.
                        // This enables multi-task autonomous execution — agents keep
                        // working through task lists without stopping.
                        let mut final_cook_response = cook_response;
                        let max_continuations = 2; // S79: reduced from 5 — less re-firing, less defensive language
                        let mut continuation_fired = false; // self-loop guard: only allow one continuation trigger
                        for continuation in 0..max_continuations {
                            if let Ok(ref response_text) = final_cook_response {
                                // Self-loop guard: if we already fired a continuation, don't re-check
                                // the response for more signals. Continuation responses often contain
                                // the same phrases ("next task", etc.) causing infinite re-triggering.
                                if continuation_fired {
                                    break;
                                }
                                // Check if response indicates work is still in progress.
                                // Continuation fires when the agent signals incomplete work.
                                // Does NOT fire when the agent signals completion.
                                let lower = response_text.to_lowercase();

                                // Completion signals — agent is done, do NOT continue
                                let is_done = lower.contains("standing by")
                                    || lower.contains("ready for")
                                    || lower.contains("delivered")
                                    || lower.contains("shipped")
                                    || lower.contains("pushed to main")
                                    || lower.contains("merged")
                                    || lower.contains("heartbeat_ok")
                                    || lower.ends_with("done.")
                                    || lower.ends_with("done!")
                                    || lower.ends_with("complete.")
                                    || lower.ends_with("⚡");

                                // In-progress signals — agent is still working, continue.
                                // ONLY trigger on unambiguous self-referential signals.
                                // Generic words like "writing", "creating", "researching"
                                // appear in Q&A answers ("Sun was writing Solaris") and
                                // must NOT trigger continuation. Require first-person
                                // phrasing that clearly indicates the agent's own intent.
                                let is_in_progress = lower.contains("i'm working on")
                                    || lower.contains("i am working on")
                                    || lower.contains("i'll continue")
                                    || lower.contains("i will continue")
                                    || lower.contains("next task")
                                    || lower.contains("moving to task")
                                    || lower.contains("now let me implement")
                                    || lower.contains("now let me build")
                                    || lower.contains("let me fix that now");

                                if is_done || !is_in_progress {
                                    break;
                                }
                                
                                continuation_fired = true; // self-loop guard: mark that we fired once
                                info!(
                                    "Continuation {} — agent indicates more work, re-entering cooking loop",
                                    continuation + 1,
                                );
                                
                                // Re-cook with continuation prompt
                                // Tier 1: keep channels showing "typing..." during continuation cook.
                                let _typing_guard = {
                                    let agent_guard = dispatch_agent.read().await;
                                    let channels = agent_guard.channel_manager();
                                    drop(agent_guard);
                                    spawn_typing_heartbeat(channels, Some(channels_source_to_core(&msg.source)))
                                };
                                // #192b P1 (2b): snapshot continuation history before the future
                                // so it captures an owned value, leaving cook_ctx free for persist.
                                let cont_recent_history = cook_ctx.session().recent_history_for_llm();
                                let cont_future = async {
                                    let prom_ref = prometheus_for_rx.as_ref().ok_or_else(|| zeus_core::Error::Config("Prometheus not initialized for continuation".into()))?;
                                    let prom_guard = prom_ref.read().await;
                                    let (tool_schemas, session_history) = {
                                        let agent_guard = dispatch_agent.read().await;
                                        let schemas = agent_guard.tool_schemas();
                                        // #52: migrate to #51 engine seam — replaces legacy len()>50 slicing.
                                        // #192b P1 (2b): continuation history from the per-cook snapshot.
                                        let history = cont_recent_history.clone();
                                        (schemas, history)
                                    };
                                    let alias = {
                                        let sessions_guard = prom_guard.sessions().read().await;
                                        prom_guard.session_resolver(&sessions_guard, &channel_default_agent_id, None, zeus_prometheus::ChannelKind::Other("internal".to_string()), chrono::Utc::now()).await
                                    };
                                    let prior_dispatches = prom_guard.track_dispatch(&alias).await;
                                    let cook_span = info_span!("cook",
                                        fleet_session_alias = %alias,
                                        agent = %channel_default_agent_id,
                                        human = "none",
                                        channel_kind = %zeus_prometheus::ChannelKind::Other("internal".to_string()),
                                        surface = "gateway",
                                        callsite = "gateway:2340"
                                    );
                                    let _cook_guard = cook_span.enter();
                                    tracing::info!(
                                        gate = "fleet_session_correlation",
                                        fleet_session_alias = %alias,
                                        resolved_via = alias.resolved_via(),
                                        prior_dispatches_for_alias = prior_dispatches,
                                        agent = %channel_default_agent_id,
                                        human = "none",
                                        channel_kind = %zeus_prometheus::ChannelKind::Other("internal".to_string()),
                                        "resolver decision"
                                    );
                                    tracing::info!(
                                        gate = "cook_dispatched",
                                        history_len = session_history.len(),
                                        message_len = 0usize,
                                        "cook entry"
                                    );
                                    drop(_cook_guard);
                                    let result = prom_guard.cook_with_history(
                                        "Keep going — you indicated there's more work. Pick up where you left off.",
                                        &tool_schemas, &session_history,
                                    ).instrument(cook_span).await?;
                                    info!(
                                        "Continuation {} complete: {} iterations, {} tool calls",
                                        continuation + 1, result.iterations, result.tool_calls.len(),
                                    );
                                    Ok::<String, zeus_core::Error>(result.response)
                                };
                                
                                final_cook_response = match tokio::time::timeout(
                                    std::time::Duration::from_secs(cooking_timeout),
                                    cont_future,
                                ).await {
                                    Ok(result) => result,
                                    Err(_) => {
                                        warn!("Continuation {} timed out", continuation + 1);
                                        break;
                                    }
                                };
                                
                                // Persist continuation response
                                // #192b P1 (2b): persist to cook_ctx, no dispatch_agent.write().
                                {
                                    if let Ok(ref text) = final_cook_response {
                                        let msg = zeus_core::Message::assistant(text);
                                        let _ = cook_ctx.session_mut().add(msg).await;
                                    }
                                }
                            } else {
                                break; // Error — don't continue
                            }
                        }
                        
                        final_cook_response
                    } else {
                        // Simple message → route through unified inbox (sequential, no concurrent writes)
                        inbox_for_channel.send_and_wait(
                            final_content.clone(),
                            None,
                            core_attachments.iter().cloned().collect(),
                            channel_timeout_secs,
                            false,
                            msg.is_addressed, // #66 Cut 3: propagate ChannelMessage→InboxMessage classification
                        ).await.map_err(zeus_core::Error::Internal)
                    };

                    match response {
                        Ok(response) => {
                            // Reset error streak on success
                            error_streaks.remove(&dispatch_agent_id);
                            info!("Agent response: {} chars", response.len());

                            // Always send the response — agents must communicate.
                            // Strip NO_REPLY token if present but still send remaining content.
                            let final_response = zeus_session::strip_silent_token(&response)
                                .unwrap_or_else(|| response.clone());
                            if !final_response.trim().is_empty() {
                                let prefixed = if let Some(ref prefix) = response_prefix {
                                    format!("{} {}", prefix, final_response)
                                } else {
                                    final_response
                                };
                                let agent_read = channel_agent.read().await;
                                agent_read.send_to_channel(&msg.source, &prefixed).await;
                            }

                            // Mark task as delivered — prevents re-cooking the same task
                            // when subsequent channel messages mention the agent in chatter.
                            // #192b P1 (2b): marker persists to cook_ctx, no dispatch_agent.write().
                            {
                                let marker = zeus_core::Message::system("[TASK_COMPLETED: response delivered to channel]");
                                let _ = cook_ctx.session_mut().add(marker).await;
                            }

                            // Auto-detect task assignment (Layer B fallback):
                            // If the inbound message looks like a task assignment AND
                            // the agent's HEARTBEAT.md CURRENT TASK is empty, auto-persist.
                            // SKIP if the cook already answered the message — don't re-queue
                            // questions that were just responded to. Only persist tasks that
                            // arrived while the agent was busy cooking something else.
                            // REGRESSION FIX (2026-06-25): this gate was hardcoded
                            // `cook_already_answered = true` in ac32db62 (May 30), which
                            // dead-coded the whole auto-task-detection block → channel
                            // tasks never persisted to CURRENT TASK/TaskStore → seats
                            // "looked but didn't cook". Re-enabled: detect_task_with_llm
                            // classifies task-vs-question (returns None for chat/praise/
                            // acks) and task_store.persist_detected is idempotent on
                            // source_channel, so always running this is safe.
                            let run_task_detect = true;
                            if run_task_detect {
                                // #296: non-matching sentinel, never the match-all "zeus".
                                let agent_name = config_for_threads.agent.as_ref()
                                    .and_then(|a| a.name.as_deref())
                                    .or_else(|| config_for_threads.name.as_deref())
                                    .unwrap_or("<unnamed agent>");
                                // LLM-powered task detection: understand context instead of keyword matching.
                                // Uses agent's LLM for a lightweight classification call.
                                // Falls back to keywords if LLM fails.
                                let task_desc_opt = {
                                    let guard = dispatch_agent.read().await;
                                    detect_task_with_llm(guard.llm(), &msg.content, agent_name).await
                                };
                                if let Some(task_desc) = task_desc_opt {
                                    let agent_guard = dispatch_agent.read().await;
                                    let ws = agent_guard.workspace();
                                    match ws.get_current_task().await {
                                        Ok(None) => {
                                            // CURRENT TASK is empty — persist the detected task
                                            let task_summary = if task_desc.len() > 200 {
                                                format!("{}...", &task_desc[..200])
                                            } else {
                                                task_desc.clone()
                                            };
                                            info!("Auto-detected task assignment: {}", &task_summary[..task_summary.len().min(80)]);
                                            // Write to HEARTBEAT.md — use set_current_task to preserve
                                            // all other sections (## tasks, ## Daily, ## Weekly, etc.)
                                            if let Err(e) = ws.set_current_task(&task_summary).await {
                                                warn!("Failed to auto-persist task to HEARTBEAT.md: {}", e);
                                            } else {
                                                info!("Task auto-persisted to HEARTBEAT.md CURRENT TASK");
                                            }

                                            // Also persist to TaskStore (SQLite) for heartbeat pickup
                                            // via the task-driven autonomy prompt injection.
                                            // Idempotent on source_channel — replay-safe.
                                            let source_channel = format!(
                                                "{}:{}:{}",
                                                msg.source.channel_type,
                                                msg.source.chat_id.as_deref().unwrap_or(&msg.source.user_id),
                                                msg.source.user_id
                                            );
                                            let assigned_by = msg.source.user_id.clone();
                                            let state_guard = api_state_for_rx.read().await;
                                            match state_guard.task_store.persist_detected(
                                                &dispatch_agent_id,
                                                &task_summary,
                                                &source_channel,
                                                &assigned_by,
                                            ).await {
                                                Ok((task_id, true)) => {
                                                    info!("Task auto-persisted to TaskStore: {} (source: {})", task_id, source_channel);
                                                }
                                                Ok((task_id, false)) => {
                                                    debug!("Task already in TaskStore (idempotent): {}", task_id);
                                                }
                                                Err(e) => {
                                                    warn!("Failed to persist detected task to TaskStore: {}", e);
                                                }
                                            }
                                        }
                                        Ok(Some(_existing)) => {
                                            // CURRENT TASK occupied — append to TASK QUEUE
                                            let task_summary = if task_desc.len() > 200 {
                                                format!("{}...", &task_desc[..200])
                                            } else {
                                                task_desc.clone()
                                            };
                                            info!("CURRENT TASK occupied — queuing detected task: {}", &task_summary[..task_summary.len().min(80)]);
                                            if let Err(e) = ws.append_to_task_queue(&task_summary).await {
                                                warn!("Failed to append task to TASK QUEUE: {}", e);
                                            } else {
                                                info!("Task appended to HEARTBEAT.md TASK QUEUE");
                                            }

                                            // Also persist to TaskStore
                                            let source_channel = format!(
                                                "{}:{}:{}",
                                                msg.source.channel_type,
                                                msg.source.chat_id.as_deref().unwrap_or(&msg.source.user_id),
                                                msg.source.user_id
                                            );
                                            let assigned_by = msg.source.user_id.clone();
                                            let state_guard = api_state_for_rx.read().await;
                                            if let Err(e) = state_guard.task_store.persist_detected(
                                                &dispatch_agent_id,
                                                &task_summary,
                                                &source_channel,
                                                &assigned_by,
                                            ).await {
                                                warn!("Failed to persist queued task to TaskStore: {}", e);
                                            }
                                        }
                                        Err(e) => {
                                            debug!("Could not check CURRENT TASK: {}", e);
                                        }
                                    }
                                }
                            }

                            // Feed interaction to Prometheus for learning and metrics
                            if let Some(ref prom) = prometheus_for_rx {
                                let prom_guard = prom.read().await;
                                let tool_schemas = { dispatch_agent.read().await.tool_schemas() };
                                let analysis =
                                    prom_guard.classify_intent(&msg.content, &tool_schemas);
                                if let Some(engine) = prom_guard.learning_engine() {
                                    let record = zeus_prometheus::InteractionRecord {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        timestamp: chrono::Utc::now(),
                                        query_type: analysis.intent.to_string(),
                                        tools_used: vec![],
                                        success: true,
                                        duration_ms: 0,
                                        error_message: None,
                                        complexity: format!("{:?}", analysis.complexity),
                                    };
                                    let _ = engine.record(record);
                                }
                                prom_guard.monitor().record_llm_call(0, true);
                                debug!(
                                    "Prometheus: recorded interaction (intent={}, complexity={:?})",
                                    analysis.intent, analysis.complexity
                                );
                            }
                        }
                        Err(e) => {
                            let err_str = e.to_string();
                            error!("Agent error processing channel message: {}", err_str);

                            // Surface error to channel so user/agents see what happened
                            // (instead of silent failure)
                            let warning = if err_str.contains("429") || err_str.contains("rate_limit") || err_str.contains("rate limit") {
                                format!("[WARNING] API rate limited. Waiting before retry. Error: {}", err_str)
                            } else if err_str.contains("token") && (err_str.contains("exceed") || err_str.contains("quota") || err_str.contains("limit")) {
                                format!("[WARNING] API token quota exceeded. Check your plan limits. Error: {}", err_str)
                            } else if err_str.contains("timed out") || err_str.contains("timeout") {
                                format!("[WARNING] Request timed out. The model may be overloaded. Error: {}", err_str)
                            } else if err_str.contains("401") || err_str.contains("auth") || err_str.contains("invalid.*key") {
                                format!("[WARNING] Authentication failed. Check API key. Error: {}", err_str)
                            } else {
                                format!("[WARNING] Agent error: {}", err_str)
                            };
                            {
                                let agent_read = channel_agent.read().await;
                                agent_read.send_to_channel(&msg.source, &warning).await;
                            }

                            // Track consecutive errors — auto-reset session after threshold
                            let streak = error_streaks
                                .entry(dispatch_agent_id.clone())
                                .or_insert(0);
                            *streak += 1;
                            if *streak >= MAX_CONSECUTIVE_ERRORS {
                                warn!(
                                    "Agent '{}' hit {} consecutive errors — resetting session",
                                    dispatch_agent_id, streak
                                );
                                let mut guard = dispatch_agent.write().await;
                                guard.reset_session();
                                *streak = 0;
                            }

                            // Record failed interaction in Prometheus
                            if let Some(ref prom) = prometheus_for_rx {
                                let prom_guard = prom.read().await;
                                prom_guard.monitor().record_llm_call(0, false);
                            }
                        }
                    }
                    // S-PRIORITY: Channel cook complete — drop RAII guard to clear state.
                    drop(channel_cook_guard);

                    // Mark time for queue depth tracking
                    pending_since = Some(std::time::Instant::now());
                }
                Ok(())
            }));
        } else {
            warn!("No channel receiver available");
        }
    }

    // 2b-2i. Channel relays (Telegram, Slack, Matrix, Signal, Email, MQTT, WhatsApp, Mattermost)
    crate::gateway_relays::start_telegram_relay(&config, &agent_inbox, gateway.enable_agent_processing).await;
    crate::gateway_relays::start_slack_relay(&config).await;
    crate::gateway_relays::start_matrix_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_signal_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_email_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_mqtt_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_whatsapp_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_mattermost_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;
    crate::gateway_relays::start_x_relay(&config, &agent_inbox, &prometheus, &mut tasks).await;

    // 3. Heartbeat (fallback: only when Prometheus is not handling it)
    if gateway.enable_heartbeat
        && prometheus.is_none()
        && let Some(ref prom_config) = prometheus_config_snapshot
        && prom_config.enable_heartbeat
    {
        let interval = prom_config.heartbeat_interval_secs;
        let agent_hb = agent.clone();
        let channel_cook_state = channel_cook_state.clone();
        let fleet_ch_hb = fleet_ch_global.clone();
        tasks.push(tokio::spawn(async move {
            info!(
                "Heartbeat starting with interval {}s (fallback mode)",
                interval
            );
            let mut interval_timer =
                tokio::time::interval(std::time::Duration::from_secs(interval));
            loop {
                interval_timer.tick().await;
                let (heartbeat_content, goals) = {
                    let agent_guard = agent_hb.read().await;
                    let hb = agent_guard.workspace().get_heartbeat().await.unwrap_or_default();
                    let goals = agent_guard.workspace().get_goals().await.unwrap_or_default();
                    (hb, goals)
                };
                if !heartbeat_content.is_empty() {
                    // S-PRIORITY: Defer heartbeat if a channel/user cook is in
                    // flight. #192b P2: lanes replaced the global CAS, so cooks no
                    // longer flip `is_active()` — read the in-flight counter instead,
                    // which stays honest under concurrent per-session cooks.
                    if channel_cook_state.is_channel_cooking() {
                        info!("Heartbeat deferred — another cook in progress");
                        continue;
                    }
                    let mut agent_guard = agent_hb.write().await;
                    let goals_section = if goals.is_empty() {
                        String::new()
                    } else {
                        format!("\n\n[Active Goals — prioritize these]\n{}\n",
                            goals.iter().map(|g| format!("- {}", g)).collect::<Vec<_>>().join("\n"))
                    };
                    // Capture the heartbeat result and deliver to Discord
                    match agent_guard
                        .run(&format!(
                            "[Heartbeat] Check and process pending tasks listed below.\n\
                            IMPORTANT: Only act on tasks explicitly listed here. Do NOT infer, \
                            repeat, or status-update on tasks from prior sessions or chat history. \
                            If nothing below requires action, reply HEARTBEAT_OK.\n\n{}{}",
                            heartbeat_content, goals_section
                        ))
                        .await
                    {
                        Ok(result) => {
                            let result_lower = result.trim().to_lowercase();
                            // Only deliver non-trivial results to Discord
                            if !result_lower.contains("heartbeat_ok")
                                && result_lower != "no_reply"
                                && !result_lower.is_empty()
                            {
                                if let Some(cm) = agent_guard.channel_manager() {
                                    let target = zeus_channels::ChannelSource::with_chat(
                                        "discord", "zeus", &fleet_ch_hb
                                    );
                                    let msg = if result.len() > 1800 {
                                        format!("{}…", result.chars().take(1800).collect::<String>())
                                    } else {
                                        result
                                    };
                                    if let Err(e) = cm.send(&target, &msg).await {
                                        warn!("Failed to deliver heartbeat result to Discord: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Heartbeat cook failed: {}", e);
                        }
                    }
                }
            }
            #[allow(unreachable_code)]
            Ok(())
        }));
    }

    // Mnemosyne → MEMORY.md periodic sync (every 30 minutes)
    // Exports high-importance memories from SQLite to the workspace flat file
    // so cold starts and MEMORY.md readers get accumulated knowledge.
    {
        let agent_sync = agent.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1800));
            // Run immediately on boot, then every 30 min
            loop {
                interval.tick().await;
                let agent_guard = agent_sync.read().await;
                if let Some(mn) = agent_guard.mnemosyne() {
                    let store = mn.store.lock().await;
                    match store.export_memory_summary(50) {
                        Ok(summary) if !summary.is_empty() => {
                            let workspace = agent_guard.workspace();
                            // Read existing MEMORY.md
                            let existing = workspace.get_memory().await.unwrap_or_default();

                            // Replace or append the Mnemosyne sync section
                            let marker = "## Mnemosyne Memory Sync";
                            let updated = if let Some(pos) = existing.find(marker) {
                                // Replace existing sync section
                                format!("{}{}", &existing[..pos], summary)
                            } else {
                                // Append sync section
                                format!("{}\n\n{}", existing.trim_end(), summary)
                            };

                            if let Err(e) = workspace.write("memory/MEMORY.md", &updated).await {
                                error!("Mnemosyne→MEMORY.md sync failed: {}", e);
                            } else {
                                info!("Mnemosyne→MEMORY.md sync complete ({} chars)", summary.len());
                            }
                        }
                        Ok(_) => {} // empty summary, nothing to sync
                        Err(e) => {
                            warn!("Mnemosyne export failed: {}", e);
                        }
                    }
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }));
    }

    // S66-P1B: Background autonomous orchestration loop
    // Runs every 60s: checks GoalStack for pending goals, processes autonomously.
    // This is the key missing piece — gateway can now work WITHOUT incoming messages.
    if gateway.enable_heartbeat {
        if let Some(ref prom_arc) = prometheus {
            let prom_auto = prom_arc.clone();
            let agent_auto = agent.clone();
            let wake_tx_auto = heartbeat_wake_tx.clone();
            let _config_for_auto = config.clone();
            let fleet_ch_auto = fleet_ch_global.clone();
            let channel_cook_state_auto = channel_cook_state.clone();
            let auto_default_agent_id = default_agent_id.clone();
            tasks.push(tokio::spawn(async move {
                // Wait 30s for gateway to stabilize before first check
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                // #156 defense-in-depth: per-goal consecutive-no-op counter. The
                // substantive-response gate already prevents a one-shot answer from
                // wedging the stack, but a goal that genuinely produces only no-ops
                // would otherwise re-cook forever (top_goal has no retry cap). After
                // GOAL_MAX_NOOPS consecutive no-op cooks we Abandon it so the stack
                // can advance. Map lives in the long-lived loop task; a goal that
                // does real work clears its entry on the Completed path.
                const GOAL_MAX_NOOPS: u32 = 5;
                let mut goal_noop_counts: std::collections::HashMap<String, u32> =
                    std::collections::HashMap::new();
                loop {
                    interval.tick().await;

                    // S-PRIORITY: skip if any cook is in flight (heartbeat OR channel).
                    // #192b P2: this site coordinates the autonomous-goal loop against
                    // BOTH channel cooks AND the Prometheus heartbeat (singleton-ness),
                    // so it reads the combined counter. The heartbeat now `begin_cook`s
                    // alongside its CAS so this count sees it (else this defer goes blind).
                    if channel_cook_state_auto.any_cook_inflight() {
                        debug!("Autonomous goal tick deferred — another cook in progress");
                        continue;
                    }

                    let prom_guard = prom_auto.read().await;

                    // 1. Check goal stack for pending goals
                    if let Some(goal_stack) = prom_guard.goal_stack() {
                        match goal_stack.top_goal() {
                            Ok(Some(goal)) => {
                                info!("Autonomous: processing top goal [{}]: {}", goal.priority, goal.description);

                                // #157 layer 2 (load-bearing): the DURABLE, code-enforced
                                // bounded-retry gate. A promoted goal is a SQLite row that
                                // `top_goal` re-selects every tick — with no goal-file left to
                                // re-stamp, this is the only place the cap can hold. We bump the
                                // `attempt` column in code on EVERY re-cook (not just no-ops, and
                                // surviving restart) and enforce in code; the cap is NEVER derived
                                // from the LLM-echoed `[loop-control]` body directive, which is at
                                // most a best-effort pre-promotion hint. At the cap the goal is
                                // marked Abandoned (drops out of `top_goal`) and we NOTIFY.
                                match goal_stack.bump_attempt_and_enforce(&goal.id) {
                                    Ok(Some(att)) => {
                                        warn!(
                                            "Goal {} hit bounded-retry cap at attempt {} (max {}); abandoned",
                                            goal.id, att, goal.max_attempts,
                                        );
                                        let _ = goal_stack.unblock(&goal.id);
                                        goal_noop_counts.remove(&goal.id);
                                        let agent_guard = agent_auto.read().await;
                                        if let Some(cm) = agent_guard.channel_manager() {
                                            let target = zeus_channels::ChannelSource::with_chat(
                                                "discord", "zeus", &fleet_ch_auto,
                                            );
                                            let notify = format!(
                                                "[Goal ABANDONED — bounded-retry cap: {}]\nGave up after {} of {} attempts; the re-cook condition was never met. (#157 durable retry cap)",
                                                goal.description.chars().take(80).collect::<String>(),
                                                att, goal.max_attempts,
                                            );
                                            let _ = cm.send(&target, &notify).await;
                                        }
                                        continue;
                                    }
                                    Ok(None) => { /* within bounds or unbounded — proceed */ }
                                    Err(e) => {
                                        warn!("Failed to enforce bounded-retry on goal {}: {}", goal.id, e);
                                    }
                                }

                                let agent_guard = agent_auto.read().await;
                                let tools = agent_guard.tool_schemas();
                                let _typing_guard = spawn_typing_heartbeat(agent_guard.channel_manager(), None);
                                drop(agent_guard);
                                let alias = {
                                    let sessions_guard = prom_guard.sessions().read().await;
                                    prom_guard.session_resolver(&sessions_guard, &auto_default_agent_id, None, zeus_prometheus::ChannelKind::Other("internal".to_string()), chrono::Utc::now()).await
                                };
                                let prior_dispatches = prom_guard.track_dispatch(&alias).await;
                                let cook_span = info_span!("cook",
                                    fleet_session_alias = %alias,
                                    agent = %auto_default_agent_id,
                                    human = "none",
                                    channel_kind = %zeus_prometheus::ChannelKind::Other("internal".to_string()),
                                    surface = "autonomous_loop",
                                    callsite = "gateway:2808"
                                );
                                let _cook_guard = cook_span.enter();
                                tracing::info!(
                                    gate = "fleet_session_correlation",
                                    fleet_session_alias = %alias,
                                    resolved_via = alias.resolved_via(),
                                    prior_dispatches_for_alias = prior_dispatches,
                                    agent = %auto_default_agent_id,
                                    human = "none",
                                    channel_kind = %zeus_prometheus::ChannelKind::Other("internal".to_string()),
                                    "resolver decision"
                                );
                                tracing::info!(
                                    gate = "cook_dispatched",
                                    history_len = 0usize,
                                    message_len = goal.description.len(),
                                    "cook entry"
                                );
                                drop(_cook_guard);
                                match prom_guard.cook_with_history(
                                    &goal.description, &tools, &[]
                                ).instrument(cook_span).await {
                                    Ok(result) => {
                                        // #156: a goal counts as real work — eligible to be
                                        // marked Completed and posted — when the final response is
                                        // a substantive answer, regardless of iteration count or
                                        // whether tools ran. A goal the LLM resolves in a single
                                        // no-tool text completion (iterations==1, tool_calls empty;
                                        // the legit terminal path at tool_executor.rs:627) IS work
                                        // and must advance the stack. Only a true no-op — empty or a
                                        // HEARTBEAT_OK / "Completed after N iterations" filler ack —
                                        // is left pending. Gating on iterations>1 && tools ran (the
                                        // prior shape) misclassified the one-shot answer as a no-op,
                                        // leaving it pending forever: top_goal() re-returns the same
                                        // highest-priority pending goal every 60s, wedging the whole
                                        // stack behind it and burning tokens. Parent 25cb189f marked
                                        // Completed unconditionally so the stack always advanced;
                                        // the substantive-response gate restores that advance while
                                        // still silencing genuine no-ops.
                                        let did_work = !zeus_prometheus::heartbeat::is_noop_heartbeat(&result.response)
                                            && !result.response.trim().is_empty();
                                        if !did_work {
                                            // #156 defense-in-depth: count consecutive no-ops; abandon
                                            // at the cap so a pathologically-repeating no-op can't wedge
                                            // the stack forever (top_goal has no retry cap of its own).
                                            let noops = goal_noop_counts
                                                .entry(goal.id.clone())
                                                .and_modify(|n| *n += 1)
                                                .or_insert(1);
                                            if *noops >= GOAL_MAX_NOOPS {
                                                // Snapshot the count so the borrow of goal_noop_counts
                                                // is released before the .remove() / notify below.
                                                let noop_count = *noops;
                                                warn!(
                                                    "Goal {} produced {} consecutive no-ops (cap {}); abandoning so the stack can advance",
                                                    goal.id, noop_count, GOAL_MAX_NOOPS,
                                                );
                                                if let Err(e) = goal_stack.update_status(
                                                    &goal.id,
                                                    zeus_prometheus::GoalStatus::Abandoned {
                                                        reason: format!(
                                                            "Abandoned after {} consecutive no-op cooks (#156 retry cap)",
                                                            noop_count
                                                        ),
                                                    },
                                                ) {
                                                    warn!("Failed to abandon stalled goal {}: {}", goal.id, e);
                                                } else {
                                                    let _ = goal_stack.unblock(&goal.id);
                                                    goal_noop_counts.remove(&goal.id);
                                                    // #157 item A: mirror the SQLite-cap notify so the
                                                    // no-op abandon is never silent (was warn!()-only).
                                                    let agent_guard = agent_auto.read().await;
                                                    if let Some(cm) = agent_guard.channel_manager() {
                                                        let target = zeus_channels::ChannelSource::with_chat(
                                                            "discord", "zeus", &fleet_ch_auto,
                                                        );
                                                        let notify = format!(
                                                            "[Goal ABANDONED — no-op cap: {}]\nGave up after {} consecutive no-op cooks; the goal produced no work. (#156 retry cap)",
                                                            goal.description.chars().take(80).collect::<String>(),
                                                            noop_count,
                                                        );
                                                        let _ = cm.send(&target, &notify).await;
                                                    }
                                                }
                                                continue;
                                            }
                                            info!(
                                                "Goal {} produced no work (iters={}, tools={}, noop_response={}, consecutive_noops={}/{}); leaving pending for retry",
                                                goal.id,
                                                result.iterations,
                                                result.tool_calls.len(),
                                                zeus_prometheus::heartbeat::is_noop_heartbeat(&result.response),
                                                *noops,
                                                GOAL_MAX_NOOPS,
                                            );
                                            continue;
                                        }
                                        // Real work happened — clear any prior no-op streak for this goal.
                                        goal_noop_counts.remove(&goal.id);
                                        let outcome = format!("Completed in {} iterations", result.iterations);
                                        if let Err(e) = goal_stack.update_status(
                                            &goal.id,
                                            zeus_prometheus::GoalStatus::Completed { outcome },
                                        ) {
                                            warn!("Failed to update goal status: {}", e);
                                        }
                                        let _ = goal_stack.unblock(&goal.id);
                                        info!("Goal completed: {} ({} iterations)", goal.id, result.iterations);

                                        // S67-C2: Wake heartbeat immediately after goal completion
                                        if let Some(ref wake_tx) = wake_tx_auto {
                                            let _ = wake_tx.try_send(zeus_prometheus::heartbeat::WakeRequest {
                                                reason: "goal_complete".to_string(),
                                                agent_id: None,
                                            });
                                        }

                                        // S67-A2: Persist to session
                                        {
                                            let mut agent_guard = agent_auto.write().await;
                                            let _ = agent_guard.session_mut().add(
                                                zeus_core::Message::user(&goal.description)
                                            ).await;
                                            let _ = agent_guard.session_mut().add(
                                                zeus_core::Message::assistant(&result.response)
                                            ).await;
                                        }

                                        // S67-A1/A3: Deliver goal completion to Discord
                                        if !result.response.is_empty() {
                                            let agent_guard = agent_auto.read().await;
                                            if let Some(cm) = agent_guard.channel_manager() {
                                                let target = zeus_channels::ChannelSource::with_chat(
                                                    "discord", "zeus", &fleet_ch_auto
                                                );
                                                let notify = format!(
                                                    "[Goal completed: {}]\n{}",
                                                    goal.description.chars().take(80).collect::<String>(),
                                                    result.response.chars().take(1800).collect::<String>(),
                                                );
                                                if let Err(e) = cm.send(&target, &notify).await {
                                                    warn!("Failed to deliver goal result to Discord: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Goal execution failed: {}: {}", goal.id, e);
                                        let _ = goal_stack.update_status(
                                            &goal.id,
                                            zeus_prometheus::GoalStatus::Failed { reason: e.to_string() },
                                        );

                                        // S67-A3: Notify Discord of goal failure
                                        {
                                            let agent_guard = agent_auto.read().await;
                                            if let Some(cm) = agent_guard.channel_manager() {
                                                let target = zeus_channels::ChannelSource::with_chat(
                                                    "discord", "zeus", &fleet_ch_auto
                                                );
                                                let notify = format!(
                                                    "[Goal FAILED: {}]\nError: {}",
                                                    goal.description.chars().take(80).collect::<String>(),
                                                    e.to_string().chars().take(500).collect::<String>(),
                                                );
                                                let _ = cm.send(&target, &notify).await;
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                debug!("Autonomous: no pending goals");
                            }
                            Err(e) => {
                                warn!("Autonomous: failed to check goals: {}", e);
                            }
                        }
                    }

                    // 2. Reload workspace goal files (hot-load new .md files)
                    if let Some(goal_stack) = prom_guard.goal_stack() {
                        let goals_dir = dirs::home_dir()
                            .unwrap_or_default()
                            .join(".zeus/workspace/goals");
                        if goals_dir.exists() {
                            if let Ok(entries) = std::fs::read_dir(&goals_dir) {
                                let now_ts = chrono::Utc::now().timestamp();
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.extension().map_or(false, |e| e == "md") {
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            // S67-F2: Honor `not_before` front-matter so
                                            // delayed loop_tool calls don't fire early.
                                            let (not_before, body) =
                                                zeus_agent::tools::parse_goal_front_matter(&content);
                                            // #157 layer-2: sweep the un-fired straggler.
                                            // A future-dated loop file that has already
                                            // reached its retry cap is the "clear-survival"
                                            // wake — loop_tool stopped re-arming, but this
                                            // one pending file would still fire once more
                                            // (the N+1). The goal file *is* the scheduled
                                            // wake in this model, so cancelling the wake
                                            // means removing the file here rather than
                                            // skipping it.
                                            let (attempt, max_attempts) =
                                                zeus_agent::tools::parse_goal_retry_counters(
                                                    &content,
                                                );
                                            if let (Some(a), Some(m)) = (attempt, max_attempts) {
                                                if a >= m {
                                                    info!(
                                                        "Hot-loader: sweeping capped loop straggler {} (attempt={} >= max_attempts={})",
                                                        path.display(),
                                                        a,
                                                        m
                                                    );
                                                    let _ = std::fs::remove_file(&path);
                                                    continue;
                                                }
                                            }
                                            if let Some(nb) = not_before {
                                                if now_ts < nb {
                                                    debug!(
                                                        "Hot-loader: skipping {} (not_before={}, now={}, {}s remaining)",
                                                        path.display(),
                                                        nb,
                                                        now_ts,
                                                        nb - now_ts
                                                    );
                                                    continue;
                                                }
                                            }
                                            // #157 layer-1 carry-forward: the goal file
                                            // *is* the wake, and the agent re-arms by calling
                                            // `loop` again — but it has no memory of the prior
                                            // attempt count. So at fire time, if this is a
                                            // bounded loop, stamp a machine directive into the
                                            // body telling the agent's next `loop` call to
                                            // reuse the same `loop_id` and pass `attempt+1`.
                                            // This threads the cap across the schedule → fire
                                            // → re-arm cycle without trusting the LLM to
                                            // remember anything — it only has to echo the
                                            // values the directive hands it.
                                            let (cur_attempt, cur_max) =
                                                zeus_agent::tools::parse_goal_retry_counters(&content);
                                            let cur_loop_id =
                                                zeus_agent::tools::parse_goal_loop_id(&content);
                                            let body_with_carry = match (cur_loop_id, cur_attempt, cur_max) {
                                                (Some(lid), Some(att), Some(max)) => {
                                                    format!(
                                                        "{}\n\n[loop-control: if you re-arm this with the `loop` tool, you MUST pass loop_id=\"{}\", attempt={}, and max_attempts={} so the bounded-retry cap is honored. This is attempt {} of {}; at the cap the loop is abandoned automatically.]",
                                                        body.trim(), lid, att + 1, max, att, max
                                                    )
                                                }
                                                _ => body.trim().to_string(),
                                            };
                                            let mut goal = zeus_prometheus::Goal::new(
                                                body_with_carry.as_str(),
                                                zeus_prometheus::Priority::Normal,
                                                zeus_prometheus::GoalSource::System,
                                            );
                                            // #157 layer 2: seed the DURABLE bounded-retry
                                            // counter from the goal-file front-matter. The
                                            // file is deleted right after promotion, so this
                                            // is the one chance to carry the cap onto the
                                            // SQLite row — past this point the counter is
                                            // owned and enforced in code at the re-cook site,
                                            // never re-derived from the (LLM-echoed) body.
                                            if let Some(max) = cur_max {
                                                goal.max_attempts = max as u32;
                                                goal.attempt = cur_attempt.unwrap_or(0) as u32;
                                            }
                                            if goal_stack.add(&goal).is_ok() {
                                                info!("Hot-loaded goal from {}", path.display());
                                                // Remove file after loading to prevent re-processing
                                                let _ = std::fs::remove_file(&path);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                #[allow(unreachable_code)]
                Ok::<(), anyhow::Error>(())
            }));
            info!("Autonomous orchestration loop registered (60s interval, 30s startup delay)");
        }
    }

    // Sprint #84: Autonomous backlog-pull loop.
    // Reads [prometheus.backlog_sync] from config; opt-in (enabled=false default).
    // When enabled, periodically pulls items from a backlog source (local
    // markdown file / GitHub issues stub) and stages them as goal files in
    // ~/.zeus/workspace/goals/ for the autonomous_loop above to pick up.
    if let Some(ref prom_cfg) = prometheus_config_snapshot
        && let Some(ref bs_cfg) = prom_cfg.backlog_sync
        && bs_cfg.enabled
    {
        use zeus_prometheus::backlog_sync::{
            sync_loop, BacklogSource, BacklogSyncConfig,
        };
        let source = match bs_cfg.source.as_str() {
            "github_issues" => BacklogSource::GithubIssues {
                repo: bs_cfg.github_repo.clone().unwrap_or_default(),
                labels: bs_cfg.github_labels.clone(),
                token: bs_cfg
                    .github_token_env
                    .as_ref()
                    .and_then(|k| std::env::var(k).ok())
                    .unwrap_or_else(|| std::env::var("GITHUB_TOKEN").unwrap_or_default()),
            },
            "hybrid" => {
                let local_path = bs_cfg.local_path.clone().unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(".zeus/workspace/BACKLOG.md")
                });
                BacklogSource::Hybrid {
                    github: Box::new(BacklogSource::GithubIssues {
                        repo: bs_cfg.github_repo.clone().unwrap_or_default(),
                        labels: bs_cfg.github_labels.clone(),
                        token: bs_cfg
                            .github_token_env
                            .as_ref()
                            .and_then(|k| std::env::var(k).ok())
                            .unwrap_or_else(|| std::env::var("GITHUB_TOKEN").unwrap_or_default()),
                    }),
                    local: Box::new(BacklogSource::LocalFile { path: local_path }),
                }
            }
            _ => {
                // default + "local_file"
                let path = bs_cfg.local_path.clone().unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(".zeus/workspace/BACKLOG.md")
                });
                BacklogSource::LocalFile { path }
            }
        };
        let engine_cfg = BacklogSyncConfig {
            source,
            poll_interval_secs: bs_cfg.poll_interval_secs,
            max_pending: bs_cfg.max_pending,
            titan_role: bs_cfg.titan_role.clone(),
        };
        let goals_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus/workspace/goals");
        let poll_secs = engine_cfg.poll_interval_secs;
        let titan_role = engine_cfg.titan_role.clone();
        tasks.push(tokio::spawn(async move {
            // 30s startup delay to let gateway stabilize.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if let Err(e) = sync_loop(engine_cfg, goals_dir).await {
                tracing::warn!(error = %e, "backlog_sync loop exited with error");
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }));
        info!(
            poll_interval_secs = poll_secs,
            titan_role = %titan_role,
            "Backlog sync loop registered (sprint #84)"
        );
    }

    // Use info! instead of println! so output goes to log file
    // (println! would corrupt MCP stdio JSON-RPC when gateway runs as background task)
    info!("Zeus Gateway running");
    info!(
        "  API:       {}",
        if gateway.enable_api {
            format!("http://{}:{}", gateway.host, gateway.port)
        } else {
            "disabled".to_string()
        }
    );
    info!(
        "  MCP:       {}",
        if gateway.enable_mcp {
            format!("http://{}:{}", gateway.host, gateway.mcp_port)
        } else {
            "disabled".to_string()
        }
    );
    info!(
        "  Channels:  {}",
        if gateway.enable_channels {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  TG Relay:  {}",
        if config.telegram_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  SK Relay:  {}",
        if config.slack_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  MX Relay:  {}",
        if config.matrix_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  SG Relay:  {}",
        if config.signal_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  EM Relay:  {}",
        if config.email_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  MQ Relay:  {}",
        if config.mqtt_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  WA Relay:  {}",
        if config.whatsapp_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  MM Relay:  {}",
        if config.mattermost_relay.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  Heartbeat: {}",
        if gateway.enable_heartbeat {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  Cron:      {}",
        if gateway.enable_cron {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  Pruning:   {}",
        if config.pruning.as_ref().map(|p| p.enabled).unwrap_or(false) {
            let p = config.pruning.as_ref().expect("pruning checked above");
            format!(
                "enabled (every {}s, max_age={}d, max_sessions={})",
                p.check_interval_secs, p.max_age_days, p.max_sessions
            )
        } else {
            "disabled".to_string()
        }
    );

    // Wait for shutdown signal
    shutdown_signal().await;
    info!("Shutting down gateway...");

    // Stop Prometheus subsystems gracefully
    if let Some(ref prometheus) = prometheus {
        let mut prom_guard = prometheus.write().await;
        prom_guard.stop_heartbeat();
        prom_guard.stop_scheduler();
        prom_guard.stop_consolidation();
        info!("Prometheus subsystems stopped");
    }

    // Stop channels gracefully
    {
        let agent_guard = agent.read().await;
        agent_guard.stop_channels().await;
    }

    // Abort session pruning task
    if let Some(handle) = _pruning_handle {
        handle.abort();
    }

    // Signal all background tasks to shut down gracefully
    shutdown_token.cancel();
    info!("Shutdown signal sent — waiting for tasks to complete...");

    // Give tasks 5 seconds to finish gracefully, then abort
    let grace = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            for task in tasks {
                let _ = task.await;
            }
        }
    ).await;

    if grace.is_err() {
        warn!("Graceful shutdown timed out after 5s — some tasks may have been interrupted");
    }

    info!("Gateway shut down");
    Ok(())
}

/// Wait for a shutdown signal (Ctrl+C or SIGTERM)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, shutting down gateway...");
        }
        _ = terminate => {
            info!("Received SIGTERM, shutting down gateway...");
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MidLoopInterruptKind {
    Stop,
    Redirect,
}

fn classify_mid_loop_interrupt(content: &str) -> Option<MidLoopInterruptKind> {
    let content_lower = content.to_lowercase();
    if crate::gateway_consumer::is_stop_command(content)
        || content_lower.contains("stop")
        || content_lower.contains("pause")
        || content_lower.contains("halt")
        || content_lower.contains("wait")
        || content_lower.contains("cancel")
    {
        return Some(MidLoopInterruptKind::Stop);
    }

    if content_lower.contains("redirect") {
        return Some(MidLoopInterruptKind::Redirect);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mid_loop_interrupt_command_preserves_stop_pause_cancel() {
        assert_eq!(
            classify_mid_loop_interrupt("/stop"),
            Some(MidLoopInterruptKind::Stop)
        );
        assert_eq!(
            classify_mid_loop_interrupt("pause for now"),
            Some(MidLoopInterruptKind::Stop)
        );
        assert_eq!(
            classify_mid_loop_interrupt("cancel this cook"),
            Some(MidLoopInterruptKind::Stop)
        );
        assert_eq!(classify_mid_loop_interrupt("keep going"), None);
    }

    #[test]
    fn test_mid_loop_interrupt_command_classifies_redirect() {
        assert_eq!(
            classify_mid_loop_interrupt("redirect to the newest request"),
            Some(MidLoopInterruptKind::Redirect)
        );
        assert_eq!(
            classify_mid_loop_interrupt("please REDIRECT this cook to the new batch"),
            Some(MidLoopInterruptKind::Redirect)
        );
    }

    #[test]
    fn test_gateway_config_defaults() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert!(cfg.enable_channels);
        assert!(cfg.enable_cron);
        assert!(cfg.enable_heartbeat);
        assert!(cfg.enable_api);
        assert!(cfg.enable_mcp);
        assert_eq!(cfg.mcp_port, 3002);
        assert!(cfg.enable_agent_processing);
    }

    #[test]
    fn test_gateway_config_with_overrides() {
        let cfg = GatewayConfig {
            host: "0.0.0.0".to_string(),
            port: 9090,
            public_url: "https://example.zeuslab.ai".to_string(),
            enable_channels: false,
            enable_cron: false,
            enable_heartbeat: false,
            enable_api: true,
            enable_mcp: false,
            mcp_port: 4000,
            web_dist: None,
            web_port: 8081,
            ..GatewayConfig::default()
        };
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 9090);
        assert!(!cfg.enable_channels);
        assert!(!cfg.enable_mcp);
    }

    #[tokio::test]
    async fn test_shutdown_signal_select() {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                // Timer fires first, which is expected in tests
            }
            _ = shutdown_signal() => {
                // Would only reach here on actual signal
            }
        }
    }
}
