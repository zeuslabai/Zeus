//! API Routes

use crate::middleware as key_middleware;
use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, post, put},
};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tower_http::services::ServeDir;

use crate::SharedState;
use crate::api_key::ApiKeyValidator;
use crate::docs;
use crate::handlers;
use crate::node_ws;
use crate::rate_limit::{HttpRateLimiter, RateLimitConfig, RateLimitLayer};
use crate::security_headers::SecurityHeadersLayer;
use crate::upload_handlers;
use crate::websocket;

/// Create the API router with all routes
pub fn create_router(state: SharedState, cors: bool) -> Router {
    create_router_with_auth(state, cors, None, &[], None)
}

/// Create a test router without auth middleware (for unit tests)
pub fn create_test_router(state: SharedState) -> Router {
    build_base_router(state, None)
}

/// Create the API router with authentication and CORS configuration
pub fn create_router_with_auth(
    state: SharedState,
    cors: bool,
    auth_token: Option<String>,
    allowed_origins: &[String],
    rate_limit_config: Option<RateLimitConfig>,
) -> Router {
    let mut router = build_base_router(state.clone(), rate_limit_config);

    // Load API keys from environment
    let api_keys = ApiKeyValidator::from_env();
    if api_keys.has_keys() {
        tracing::info!(
            "API key auth enabled ({} key(s) loaded from ZEUS_API_KEYS/ZEUS_API_KEY)",
            api_keys.key_count()
        );
    }

    // ALWAYS apply auth middleware — when no token is configured, only
    // health/auth/onboarding endpoints are accessible (forces setup).
    // 2FA middleware layer — runs BEFORE auth (layer order is reversed in axum).
    // Must be added first so it runs after auth middleware in the request pipeline.
    {
        let state_for_2fa = state.clone();
        router = router.layer(middleware::from_fn(move |req, next| {
            let st = state_for_2fa.clone();
            totp_2fa_middleware(req, next, st)
        }));
    }

    match auth_token {
        Some(token) => {
            router = router.layer(middleware::from_fn(move |req, next| {
                let token = token.clone();
                let keys = api_keys.clone();
                auth_middleware(req, next, token, keys)
            }));
        }
        None => {
            if api_keys.has_keys() {
                // No Bearer token, but API keys are configured — allow API key auth only
                router = router.layer(middleware::from_fn(move |req, next| {
                    let keys = api_keys.clone();
                    api_key_only_middleware(req, next, keys)
                }));
            } else {
                tracing::warn!(
                    "No auth token configured — all API routes except health/auth/onboarding are blocked.                  Set ZEUS_API_TOKEN or configure auth in config.toml."
                );
                let state_for_no_auth = state.clone();
                router = router.layer(middleware::from_fn(move |req, next| {
                    let s = state_for_no_auth.clone();
                    no_token_middleware(req, next, s)
                }));
            }
        }
    }

    if cors {
        let cors_layer = build_cors_layer(allowed_origins);
        router = router.layer(cors_layer);
    }

    // Add security headers to all responses
    router = router.layer(SecurityHeadersLayer::default());

    router
}

/// Build the base router with all routes (no auth middleware)
fn build_base_router(state: SharedState, rate_limit_config: Option<RateLimitConfig>) -> Router {
    let mut router = Router::new()
        // Health (no auth required)
        .route("/", get(handlers::health))
        .route("/health", get(handlers::health))
        .route("/health/detailed", get(handlers::health_detailed))
        // Chat
        .route("/v1/chat", post(handlers::chat))
        // LLM Council — multi-model deliberation
        .route("/v1/council/query", post(handlers::council_handlers::council_query))
        // Studio (ZeusWeb chat with custom system prompt)
        .route("/v1/studio", post(handlers::studio_chat))
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(handlers::openai_chat_completions),
        )
        .route("/v1/models", get(handlers::openai_list_models))
        // OpenAI Responses API
        .route("/v1/responses", post(handlers::responses::create_response))
        .route("/v1/responses/:id", get(handlers::responses::get_response))
        .route(
            "/v1/responses/:id",
            delete(handlers::responses::delete_response),
        )
        // Vector stores
        .route(
            "/v1/vector_stores",
            post(handlers::vector_store::create_vector_store),
        )
        .route(
            "/v1/vector_stores",
            get(handlers::vector_store::list_vector_stores),
        )
        .route(
            "/v1/vector_stores/:id",
            get(handlers::vector_store::get_vector_store),
        )
        .route(
            "/v1/vector_stores/:id",
            delete(handlers::vector_store::delete_vector_store),
        )
        .route(
            "/v1/vector_stores/:id/search",
            post(handlers::vector_store::search_vector_store),
        )
        .route(
            "/v1/vector_stores/:id/files",
            post(handlers::vector_store::add_file_to_store),
        )
        .route(
            "/v1/vector_stores/:id/files",
            get(handlers::vector_store::list_store_files),
        )
        // Batch API
        .route("/v1/batches", post(handlers::batch::create_batch))
        .route("/v1/batches/:id", get(handlers::batch::get_batch))
        .route(
            "/v1/batches/:id/results",
            get(handlers::batch::get_batch_results),
        )
        // Embeddings API
        .route("/v1/embeddings", post(handlers::batch::create_embeddings))
        // Sessions
        .route("/v1/sessions", get(handlers::list_sessions))
        .route("/v1/sessions", post(handlers::create_session))
        .route("/v1/sessions/:id", get(handlers::get_session))
        .route("/v1/sessions/:id", delete(handlers::delete_session))
        .route("/v1/sessions/:id/clear", post(handlers::clear_session))
        .route("/v1/sessions/:id/compact", post(handlers::compact_session))
        .route("/v1/sessions/search", post(handlers::search_sessions))
        // Tools
        .route("/v1/tools", get(handlers::list_tools))
        .route("/v1/tools/:name", post(handlers::execute_tool))
        // Token counting
        .route("/v1/tokens/count", post(handlers::count_tokens))
        // Memory
        .route("/v1/memory", get(handlers::get_memory))
        .route("/v1/memory/remember", post(handlers::remember))
        .route("/v1/memory/note", post(handlers::add_note))
        // Blog CMS
        .route("/v1/blog/posts", get(handlers::blog::list_posts))
        .route("/v1/blog/posts", post(handlers::blog::create_post))
        .route("/v1/blog/posts/:slug", get(handlers::blog::get_post))
        .route("/v1/blog/posts/:slug", put(handlers::blog::update_post))
        .route("/v1/blog/posts/:slug", delete(handlers::blog::delete_post))
        .route("/v1/blog/tags", get(handlers::blog::list_tags))
        // Blog media (images)
        .route("/v1/blog/images", post(handlers::blog::upload_media))
        .route("/v1/blog/images", get(handlers::blog::list_media))
        .route(
            "/v1/blog/images/:filename",
            get(handlers::blog::serve_media),
        )
        .route(
            "/v1/blog/images/:filename",
            delete(handlers::blog::delete_media),
        )
        // Blog admin 2FA
        .route("/v1/auth/totp/setup", post(handlers::blog_auth::totp_setup))
        .route(
            "/v1/auth/totp/enable",
            post(handlers::blog_auth::totp_enable),
        )
        .route(
            "/v1/auth/totp/verify",
            post(handlers::blog_auth::totp_verify),
        )
        .route(
            "/v1/auth/totp/status",
            get(handlers::blog_auth::totp_status),
        )
        .route("/v1/auth/totp", delete(handlers::blog_auth::totp_disable))
        // Webhooks (inbound)
        .route("/v1/webhooks", get(handlers::webhook_health))
        .route("/v1/webhooks", post(handlers::receive_webhook))
        .route(
            "/v1/webhooks/:source",
            post(handlers::receive_webhook_source),
        )
        // Twilio WhatsApp webhook
        .route(
            "/v1/webhooks/whatsapp",
            get(handlers::whatsapp_webhook_health),
        )
        .route(
            "/v1/webhooks/whatsapp",
            post(handlers::receive_whatsapp_webhook),
        )
        // Twilio Voice inbound webhook
        .route("/v1/voice/inbound", get(handlers::voice_inbound_health))
        .route("/v1/voice/inbound", post(handlers::receive_voice_inbound))
        .route(
            "/v1/voice/recording-status",
            post(handlers::receive_recording_status),
        )
        // Config
        .route("/v1/config", get(handlers::get_config))
        .route("/v1/config", put(handlers::update_config))
        .route("/v1/config/test", post(handlers::test_provider))
        .route("/v1/config/models", post(handlers::fetch_provider_models))
        // Activity
        .route("/v1/activity", get(handlers::get_activity))
        // Stats
        .route("/v1/stats", get(handlers::get_stats))
        // Session stats
        .route("/v1/sessions/:id/stats", get(handlers::get_session_stats))
        // Doctor
        .route("/v1/doctor", get(handlers::doctor))
        // Status
        .route("/v1/status", get(handlers::status))
        // Skills
        .route("/v1/credentials", get(handlers::list_credentials))
        .route("/v1/credentials", post(handlers::store_credential))
        .route("/v1/credentials/:name", delete(handlers::delete_credential))
        .route("/v1/skills", get(handlers::list_skills))
        .route("/v1/skills", post(handlers::install_skill))
        .route("/v1/skills/search", get(handlers::search_skills))
        .route(
            "/v1/skills/categories",
            get(handlers::list_skill_categories),
        )
        .route(
            "/v1/skills/clawhub/install",
            post(handlers::install_clawhub_skill),
        )
        .route("/v1/skills/:id", get(handlers::get_skill))
        .route("/v1/skills/:id", put(handlers::update_skill))
        .route("/v1/skills/:id", delete(handlers::delete_skill))
        .route("/v1/skills/:id/schema", get(handlers::get_skill_schema))
        // Outcome Templates
        .route("/v1/templates", get(handlers::templates::list_templates))
        .route("/v1/templates", post(handlers::templates::create_template))
        .route(
            "/v1/templates/categories",
            get(handlers::templates::list_categories),
        )
        .route(
            "/v1/templates/search",
            get(handlers::templates::search_templates),
        )
        .route("/v1/templates/:id", get(handlers::templates::get_template))
        .route(
            "/v1/templates/:id",
            put(handlers::templates::update_template),
        )
        .route(
            "/v1/templates/:id",
            delete(handlers::templates::delete_template),
        )
        .route(
            "/v1/templates/:id/apply",
            post(handlers::templates::apply_template),
        )
        // MCP Servers
        .route("/v1/mcp/servers", get(handlers::list_mcp_servers))
        .route("/v1/mcp/servers", post(handlers::add_mcp_server))
        .route("/v1/mcp/servers/:id", delete(handlers::delete_mcp_server))
        .route(
            "/v1/mcp/servers/:id/tools",
            get(handlers::list_mcp_server_tools),
        )
        .route("/v1/mcp/tools/:tool/test", post(handlers::test_mcp_tool))
        // Memory files
        .route("/v1/memory/files", get(handlers::list_memory_files))
        .route("/v1/memory/files", post(handlers::create_memory_file))
        .route("/v1/memory/files/*path", get(handlers::read_memory_file))
        .route("/v1/memory/files/*path", put(handlers::write_memory_file))
        .route(
            "/v1/memory/files/*path",
            delete(handlers::delete_memory_file),
        )
        .route("/v1/memory/search", post(handlers::search_memory))
        // Graph memory (Sprint 9)
        .route("/v1/memory/graph/search", post(handlers::graph_search))
        .route(
            "/v1/memory/graph/:entity_id",
            get(handlers::get_entity_graph),
        )
        .route("/v1/memory/communities", get(handlers::list_communities))
        // Knowledge graph visualization (Phase 7)
        .route(
            "/v1/memory/graph/nodes",
            get(handlers::intelligence::graph_nodes),
        )
        .route(
            "/v1/memory/graph/edges",
            get(handlers::intelligence::graph_edges),
        )
        .route(
            "/v1/memory/graph/stats",
            get(handlers::intelligence::graph_stats),
        )
        .route(
            "/v1/memory/patterns",
            get(handlers::intelligence::memory_patterns),
        )
        .route(
            "/v1/memory/stats",
            get(handlers::intelligence::memory_stats),
        )
        .route(
            "/v1/memory/entities/:id/messages",
            get(handlers::intelligence::entity_messages),
        )
        // Nous cognitive engine (Phase 7)
        .route(
            "/v1/nous/reflect",
            get(handlers::intelligence::nous_reflect),
        )
        .route(
            "/v1/nous/capabilities",
            get(handlers::intelligence::nous_capabilities),
        )
        .route(
            "/v1/nous/learning/stats",
            get(handlers::intelligence::nous_learning_stats),
        )
        .route(
            "/v1/nous/learning/lessons",
            get(handlers::intelligence::nous_learning_lessons),
        )
        .route(
            "/v1/nous/understand",
            post(handlers::intelligence::nous_understand),
        )
        .route("/v1/nous/reason", post(handlers::intelligence::nous_reason))
        .route(
            "/v1/nous/learning/outcome",
            post(handlers::intelligence::nous_learn_outcome),
        )
        .route(
            "/v1/nous/observe",
            post(handlers::intelligence::nous_observe),
        )
        // Predictive spawning (Phase 7)
        .route(
            "/v1/spawner/status",
            get(handlers::intelligence::spawner_status),
        )
        .route(
            "/v1/spawner/active",
            get(handlers::intelligence::spawner_active),
        )
        .route(
            "/v1/spawner/history",
            get(handlers::intelligence::spawner_history),
        )
        .route(
            "/v1/spawner/analyze",
            post(handlers::intelligence::spawner_analyze),
        )
        // Phase 6 — Intelligence Deep
        .route(
            "/v1/learning/feedback",
            post(handlers::intelligence::learning_feedback),
        )
        .route(
            "/v1/learning/insights",
            get(handlers::intelligence::learning_insights),
        )
        .route(
            "/v1/learning/strategies",
            get(handlers::intelligence::learning_strategies),
        )
        .route(
            "/v1/memory/marketplace/list",
            post(handlers::intelligence::memory_marketplace_list),
        )
        .route(
            "/v1/memory/marketplace/browse",
            get(handlers::intelligence::memory_marketplace_browse),
        )
        .route(
            "/v1/memory/marketplace/acquire",
            post(handlers::intelligence::memory_marketplace_acquire),
        )
        .route(
            "/v1/spawner/predict",
            post(handlers::intelligence::spawner_predict),
        )
        .route(
            "/v1/spawner/daemon/status",
            get(handlers::intelligence::spawner_daemon_status),
        )
        // Context journals
        .route("/v1/context/journals", get(handlers::list_context_journals))
        .route("/v1/memory/sync", post(handlers::sync_memory))
        .route("/v1/memory/timeline", get(handlers::memory_timeline))
        // Channels
        .route("/v1/channels", get(handlers::list_channels))
        .route("/v1/channels/health", get(handlers::channel_health))
        .route("/v1/channels/signal/link-uri", post(handlers::signal_link_uri))
        .route("/v1/channels", post(handlers::create_channel))
        .route("/v1/channels/:id", get(handlers::get_channel))
        .route("/v1/channels/:id", put(handlers::update_channel))
        .route("/v1/channels/:id", delete(handlers::delete_channel))
        .route("/v1/channels/:id/test", post(handlers::test_channel))
        .route("/v1/channels/:id/status", get(handlers::channel_status))
        .route("/v1/channels/:id/connect", post(handlers::connect_channel))
        .route(
            "/v1/channels/:id/disconnect",
            post(handlers::disconnect_channel),
        )
        // Channel broadcast
        .route("/v1/channels/broadcast", post(handlers::channel_broadcast))
        // Telegram polls
        .route("/v1/channels/:id/poll", post(handlers::send_poll))
        .route(
            "/v1/channels/:id/poll/:message_id",
            delete(handlers::stop_poll),
        )
        // Session replay
        .route("/v1/sessions/:id/replay", get(handlers::session_replay))
        .route(
            "/v1/sessions/:id/replay/:turn",
            get(handlers::session_replay_turn),
        )
        // Session branching
        .route("/v1/sessions/:id/branch", post(handlers::create_branch))
        .route("/v1/sessions/:id/branches", get(handlers::list_branches))
        // Phase 3: Session detail
        .route("/v1/sessions/:id/raw", get(handlers::get_session_raw))
        .route("/v1/sessions/:id/audit", get(handlers::get_session_audit))
        .route("/v1/sessions/:id/tools", get(handlers::get_session_tools))
        // Phase 3: Pipeline
        .route("/v1/pipeline/stats", get(handlers::pipeline_stats))
        // Phase 3: Analytics
        .route("/v1/analytics/costs", get(handlers::analytics_costs))
        .route("/v1/analytics/tokens", get(handlers::analytics_tokens))
        .route(
            "/v1/analytics/providers",
            get(handlers::analytics_providers),
        )
        .route("/v1/analytics/budgets", get(handlers::analytics_budgets))
        .route("/v1/analytics/sessions", get(handlers::analytics_sessions))
        .route("/v1/analytics/daily", get(handlers::analytics_daily))
        .route("/v1/analytics/models", get(handlers::analytics_models))
        // Phase 3: Security
        .route("/v1/security/threats", get(handlers::security_threats))
        .route(
            "/v1/security/permissions",
            get(handlers::security_permissions),
        )
        .route(
            "/v1/security/permissions",
            put(handlers::update_security_permissions),
        )
        .route("/v1/security/keys", get(handlers::security_keys))
        .route("/v1/security/allowlist", get(handlers::security_allowlist))
        .route(
            "/v1/security/allowlist",
            put(handlers::update_security_allowlist),
        )
        .route("/v1/security/audit", get(handlers::security_audit))
        // Phase 4: Projects
        .route("/v1/projects", get(handlers::list_projects))
        .route("/v1/projects", post(handlers::create_project))
        .route("/v1/projects/:id", get(handlers::get_project))
        .route("/v1/projects/:id", put(handlers::update_project))
        .route(
            "/v1/projects/:id/agents",
            put(handlers::assign_project_agents),
        )
        .route("/v1/projects/:id", delete(handlers::delete_project))
        // Phase 4: Agents CRUD + routing
        .route("/v1/agents/discover", get(handlers::fleet::discover_agents))
        .route("/v1/agents/team", post(handlers::create_agent_team))
        .route("/v1/agents/spawn", post(handlers::spawn_agent))
        .route("/v1/agents/run-task", post(handlers::run_task))
        .route(
            "/v1/agents/auto-spawn",
            post(handlers::agent_spawner::auto_spawn_agent),
        )
        .route(
            "/v1/agents/auto-spawn/jobs",
            get(handlers::agent_spawner::auto_spawn_jobs),
        )
        // Parallel dispatch (S60-T5) — spawn one agent per task
        .route(
            "/v1/agents/auto-spawn/batch",
            post(handlers::agent_spawner::batch_spawn),
        )
        .route(
            "/v1/agents/auto-spawn/status/:id",
            get(handlers::agent_spawner::auto_spawn_status),
        )
        // Reports (S59-T4)
        .route(
            "/v1/reports/generate",
            post(handlers::agent_spawner::generate_report),
        )
        .route("/v1/personas", get(handlers::list_personas))
        .route("/v1/personas/:name", get(handlers::get_persona))
        .route(
            "/v1/agents/from-persona/:name",
            post(handlers::create_agent_from_persona),
        )
        .route("/v1/agents", get(handlers::list_agents))
        .route("/v1/agents", post(handlers::create_agent))
        .route("/v1/agents/:id/chat", post(handlers::agent_chat))
        .route("/v1/agents/:id/send", post(handlers::send_to_agent))
        .route("/v1/agents/:id/status", get(handlers::agent_status))
        .route("/v1/agents/:id", get(handlers::get_agent))
        .route("/v1/agents/:id", put(handlers::update_agent))
        .route("/v1/agents/:id", delete(handlers::delete_agent))
        // Phase 4: Network
        .route("/v1/network/agents", get(handlers::network_agents))
        .route("/v1/network/discover", get(handlers::network_discover))
        .route("/v1/network/messages", get(handlers::network_messages))
        .route(
            "/v1/network/messages",
            post(handlers::receive_network_message),
        )
        .route("/v1/network/send", post(handlers::network_send))
        .route("/v1/network/broadcast", post(handlers::network_broadcast))
        // Outbound webhooks
        .route(
            "/v1/webhooks/outbound",
            get(handlers::list_outbound_webhooks),
        )
        .route(
            "/v1/webhooks/outbound",
            post(handlers::register_outbound_webhook),
        )
        .route(
            "/v1/webhooks/outbound/:id",
            delete(handlers::delete_outbound_webhook),
        )
        // Webhook triggers (automation pipelines)
        .route("/v1/webhooks/triggers", get(handlers::list_triggers))
        .route("/v1/webhooks/triggers", post(handlers::create_trigger))
        .route(
            "/v1/webhooks/triggers/:id",
            delete(handlers::delete_trigger),
        )
        .route(
            "/v1/webhooks/triggers/:id/enable",
            put(handlers::enable_trigger),
        )
        .route(
            "/v1/webhooks/triggers/:id/disable",
            put(handlers::disable_trigger),
        )
        // Provider catalog (onboarding wizard)
        .route("/v1/providers", get(handlers::list_providers))
        .route("/v1/providers/ollama/health", get(handlers::ollama_health))
        // Config providers / reload / history
        .route("/v1/config/providers", get(handlers::get_providers))
        .route("/v1/config/reload", post(handlers::reload_config))
        .route("/v1/config/history", get(handlers::config_history))
        // Auth
        .route("/v1/auth/login", post(handlers::auth_login))
        .route("/v1/auth/status", get(handlers::auth_status))
        .route("/v1/auth/token", post(handlers::auth_token))
        .route("/v1/auth/logout", post(handlers::auth_logout))
        .route("/v1/auth/refresh", post(handlers::auth_refresh))
        // Anthropic OAuth
        .route(
            "/v1/auth/anthropic/login",
            get(handlers::anthropic_oauth_login),
        )
        .route(
            "/v1/auth/anthropic/callback",
            get(handlers::anthropic_oauth_callback),
        )
        .route(
            "/v1/auth/anthropic/status",
            get(handlers::anthropic_oauth_status),
        )
        // Onboarding
        .route("/v1/onboarding/status", get(handlers::onboarding_status))
        .route(
            "/v1/onboarding/complete",
            post(handlers::onboarding_complete),
        )
        .route("/v1/onboarding/setup", post(handlers::onboarding_setup))
        .route("/v1/onboarding/config", get(handlers::onboarding_config))
        // Approvals
        .route("/v1/approvals", get(handlers::list_approvals))
        .route(
            "/v1/approvals/:id/approve",
            post(handlers::approve_execution),
        )
        .route("/v1/approvals/:id/deny", post(handlers::deny_execution))
        // Agent Tasks (S52-T1)
        .route("/v1/tasks", get(handlers::list_tasks).post(handlers::create_task))
        .route("/v1/tasks/active", get(handlers::get_active_tasks))
        .route("/v1/tasks/stats", get(handlers::task_stats))
        .route("/v1/tasks/:id", get(handlers::get_task).put(handlers::update_task).delete(handlers::delete_task))
        // Discord History (S52-T2)
        .route("/v1/discord/history", get(handlers::discord_history))
        .route("/v1/discord/history/search", get(handlers::discord_history_search))
        .route("/v1/discord/history/stats", get(handlers::discord_history_stats))
        // Slack History (S55-T11)
        .route("/v1/slack/history", get(handlers::slack_history))
        .route("/v1/slack/history/thread", get(handlers::slack_history_thread))
        .route("/v1/slack/history/search", get(handlers::slack_history_search))
        .route("/v1/slack/history/stats", get(handlers::slack_history_stats))
        // TTS
        .route("/v1/tts/providers", get(handlers::list_tts_providers))
        .route("/v1/tts/synthesize", post(handlers::tts_synthesize))
        .route(
            "/v1/tts/synthesize/stream",
            post(handlers::tts_synthesize_stream),
        )
        .route("/v1/tts/voices", get(handlers::list_tts_voices))
        // Sandbox
        .route("/v1/sandbox/policies", get(handlers::list_sandbox_policies))
        .route(
            "/v1/sandbox/policies",
            post(handlers::create_sandbox_policy),
        )
        .route("/v1/sandbox/execute", post(handlers::sandbox_execute))
        // Orchestra (Teams & Delegations)
        .route("/v1/teams", get(handlers::list_teams))
        .route("/v1/teams", post(handlers::create_team))
        .route("/v1/teams/:id", get(handlers::get_team))
        .route("/v1/teams/:id", put(handlers::update_team))
        .route("/v1/teams/:id", delete(handlers::delete_team))
        .route("/v1/delegations", get(handlers::list_delegations))
        .route("/v1/delegations", post(handlers::create_delegation))
        .route("/v1/routing/recommend", post(handlers::smart_route))
        // Schedules
        .route("/v1/schedules", get(handlers::list_schedules))
        .route("/v1/schedules", post(handlers::create_schedule))
        .route("/v1/schedules/:id", get(handlers::get_schedule))
        .route("/v1/schedules/:id", put(handlers::update_schedule))
        .route("/v1/schedules/:id", delete(handlers::delete_schedule))
        .route("/v1/schedules/:id/pause", post(handlers::pause_schedule))
        .route("/v1/schedules/:id/resume", post(handlers::resume_schedule))
        .route("/v1/schedules/:id/runs", get(handlers::list_schedule_runs))
        // Cost routing
        .route("/v1/routing/costs", get(handlers::routing_costs))
        .route("/v1/routing/budget", get(handlers::routing_budget))
        .route(
            "/v1/routing/cost-recommend",
            post(handlers::routing_recommend),
        )
        // Extensions
        .route("/v1/extensions", get(handlers::list_extensions))
        .route("/v1/extensions", post(handlers::install_extension))
        .route("/v1/extensions/:id", get(handlers::get_extension))
        .route("/v1/extensions/:id", put(handlers::update_extension))
        .route("/v1/extensions/:id", delete(handlers::delete_extension))
        .route("/v1/extensions/:id/start", post(handlers::start_extension))
        .route("/v1/extensions/:id/stop", post(handlers::stop_extension))
        // Image generation
        .route("/v1/images/generate", post(handlers::generate_image))
        .route("/v1/images", get(handlers::list_images))
        .route("/v1/images/:id", get(handlers::get_image))
        // Prometheus (strategic planning + coordination)
        .route(
            "/v1/prometheus/plan",
            post(handlers::prometheus_create_plan),
        )
        .route(
            "/v1/prometheus/plan/:id",
            get(handlers::prometheus_get_plan),
        )
        .route("/v1/prometheus/execute", post(handlers::prometheus_execute))
        .route("/v1/prometheus/state", get(handlers::prometheus_state))
        .route("/v1/system/auto-tune", post(handlers::auto_tune))
        // Replication (Conway-style agent reproduction)
        .route("/v1/replication/lineage", get(handlers::replication_lineage))
        .route("/v1/replication/replicate", post(handlers::replication_replicate))
        // Benchmarks (BenchmarkStore API)
        .route("/v1/benchmarks", get(handlers::list_benchmark_runs))
        .route("/v1/benchmarks/compare", get(handlers::compare_benchmark_runs))
        .route("/v1/benchmarks/:run_id", get(handlers::get_benchmark_run))
        // Workflows (chat -> DAG execution)
        .route(
            "/v1/workflows",
            get(handlers::list_workflows).post(handlers::workflow_from_chat),
        )
        .route("/v1/workflows/:id", get(handlers::get_workflow))
        .route(
            "/v1/workflows/:id/cancel",
            post(handlers::cancel_workflow),
        )
        .route(
            "/v1/workflows/:id/artifacts",
            get(handlers::workflow_artifacts),
        )
        .route(
            "/v1/workflows/:id/download",
            get(handlers::workflow_download),
        )
        // Orchestration Engine
        .route("/v1/orchestrate/start", post(handlers::orchestrate_start))
        .route(
            "/v1/orchestrate/respond",
            post(handlers::orchestrate_respond),
        )
        .route("/v1/orchestrate/:id", get(handlers::orchestrate_status))
        .route(
            "/v1/orchestrate/:id/confirm",
            post(handlers::orchestrate_confirm),
        )
        // Teams recommendation
        .route("/v1/teams/recommend", post(handlers::team_recommend))
        // Goals
        .route(
            "/v1/goals",
            get(handlers::goals_list).post(handlers::goals_create),
        )
        .route("/v1/goals/analyze", post(handlers::goals_analyze))
        .route("/v1/goals/:id", get(handlers::goals_get))
        .route("/v1/goals/:id/status", put(handlers::goals_update_status))
        // Peer Review
        .route("/v1/reviews", get(handlers::list_reviews))
        .route("/v1/reviews", post(handlers::submit_review))
        .route("/v1/reviews/:id", get(handlers::get_review))
        .route("/v1/reviews/:id/approve", post(handlers::approve_review))
        .route("/v1/reviews/:id/reject", post(handlers::reject_review))
        // Marketplace
        .route("/v1/marketplace/listings", get(handlers::marketplace_list))
        .route(
            "/v1/marketplace/listings",
            post(handlers::marketplace_publish),
        )
        .route("/v1/marketplace/trade", post(handlers::marketplace_trade))
        .route(
            "/v1/marketplace/ledger/:agent_id",
            get(handlers::marketplace_ledger),
        )
        .route(
            "/v1/marketplace/reputation/:agent_id",
            get(handlers::marketplace_reputation),
        )
        .route("/v1/marketplace/stats", get(handlers::marketplace_stats))
        .route(
            "/v1/marketplace/featured",
            get(handlers::marketplace_featured),
        )
        .route(
            "/v1/marketplace/categories",
            get(handlers::marketplace_categories),
        )
        .route("/v1/marketplace/search", get(handlers::marketplace_search))
        .route(
            "/v1/marketplace/ratings/:skill_id",
            get(handlers::marketplace_ratings),
        )
        .route(
            "/v1/marketplace/ratings/:skill_id",
            post(handlers::marketplace_add_rating),
        )
        .route("/v1/marketplace/sync", post(handlers::marketplace_sync))
        // Bounty Board (Agora social wiring)
        .route("/v1/marketplace/bounties", post(handlers::bounty_create))
        .route("/v1/marketplace/bounties", get(handlers::bounty_list))
        .route("/v1/marketplace/bounties/:id", get(handlers::bounty_get))
        .route(
            "/v1/marketplace/bounties/:id/claim",
            post(handlers::bounty_claim),
        )
        .route(
            "/v1/marketplace/bounties/:id/submit",
            post(handlers::bounty_submit),
        )
        .route(
            "/v1/marketplace/bounties/:id/verify",
            post(handlers::bounty_verify),
        )
        .route(
            "/v1/marketplace/bounties/:id/cancel",
            post(handlers::bounty_cancel),
        )
        // Reputation badges
        .route(
            "/v1/marketplace/reputation/:agent_id/badge",
            get(handlers::marketplace_reputation_badge),
        )
        // Deploy (Phase 4 — One-Click Deploy)
        .route("/v1/deploy/targets", get(handlers::deploy_list_targets))
        .route("/v1/deploy/targets", post(handlers::deploy_create_target))
        .route("/v1/deploy/targets/:id", get(handlers::deploy_get_target))
        .route(
            "/v1/deploy/targets/:id",
            delete(handlers::deploy_delete_target),
        )
        .route("/v1/deploy", post(handlers::deploy_create))
        .route("/v1/deploy/stats", get(handlers::deploy_stats))
        .route("/v1/deploy/history", get(handlers::deploy_history))
        .route(
            "/v1/deploy/history/:target_id",
            get(handlers::deploy_target_history),
        )
        .route("/v1/deploy/:id", get(handlers::deploy_get))
        .route("/v1/deploy/:id/status", put(handlers::deploy_update_status))
        .route("/v1/deploy/:id/execute", post(handlers::deploy_execute))
        .route("/v1/deploy/:id/logs", get(handlers::deploy_logs))
        .route("/v1/deploy/:id/rollback", post(handlers::deploy_rollback))
        .route("/v1/deploy/:id/snapshots", get(handlers::deploy_snapshots))
        .route("/v1/deploy/:id/preview", get(handlers::deploy_preview))
        // Agora — agent skill marketplace (zeus-agora)
        .route("/v1/agora/listings", get(handlers::agora_listings))
        .route("/v1/agora/listings", post(handlers::agora_list_skill))
        .route(
            "/v1/agora/listings/:agent_id",
            get(handlers::agora_agent_listings),
        )
        .route(
            "/v1/agora/listings/:agent_id/:skill",
            delete(handlers::agora_delist_skill),
        )
        .route("/v1/agora/search", get(handlers::agora_search))
        .route("/v1/agora/wallets/:agent_id", get(handlers::agora_wallet))
        .route(
            "/v1/agora/wallets/:agent_id",
            post(handlers::agora_register_wallet),
        )
        .route("/v1/agora/buy", post(handlers::agora_buy))
        .route("/v1/agora/transactions", get(handlers::agora_transactions))
        .route(
            "/v1/agora/reputation/:agent_id",
            get(handlers::agora_reputation),
        )
        // Studio (Phase 5 — Agent Studio / Super Cursor)
        .route("/v1/studio/sessions", post(handlers::studio_create_session))
        .route("/v1/studio/sessions", get(handlers::studio_list_sessions))
        .route(
            "/v1/studio/sessions/active",
            get(handlers::studio_active_sessions),
        )
        .route("/v1/studio/sessions/:id", get(handlers::studio_get_session))
        .route(
            "/v1/studio/sessions/:id",
            delete(handlers::studio_delete_session),
        )
        .route(
            "/v1/studio/sessions/:id/pause",
            post(handlers::studio_pause),
        )
        .route(
            "/v1/studio/sessions/:id/resume",
            post(handlers::studio_resume),
        )
        .route(
            "/v1/studio/sessions/:id/intervene",
            post(handlers::studio_intervene),
        )
        .route(
            "/v1/studio/sessions/:id/replay",
            get(handlers::studio_replay),
        )
        .route("/v1/studio/stats", get(handlers::studio_stats))
        .route(
            "/v1/studio/sessions/:id/puppet",
            get(handlers::studio_puppet_ws),
        )
        .route(
            "/v1/studio/sessions/:id/drive",
            post(handlers::studio_drive),
        )
        .route(
            "/v1/studio/sessions/:id/link-room",
            post(handlers::studio_link_room),
        )
        // Economy
        .route("/v1/economy/wallets", get(handlers::economy_wallets))
        .route(
            "/v1/economy/wallets/:agent_id",
            get(handlers::economy_wallet),
        )
        .route(
            "/v1/economy/transactions",
            get(handlers::economy_transactions),
        )
        .route("/v1/economy/stake", post(handlers::fleet::economy_stake))
        .route(
            "/v1/economy/unstake",
            post(handlers::fleet::economy_unstake),
        )
        .route(
            "/v1/economy/transfer",
            post(handlers::fleet::economy_transfer),
        )
        .route("/v1/economy/mint", post(handlers::fleet::economy_mint))
        // Phase 5 — Autonomous earning + agent teams + federation
        .route("/v1/economy/earn", post(handlers::fleet::economy_earn))
        .route(
            "/v1/economy/earnings/:agent_id",
            get(handlers::fleet::economy_earnings),
        )
        .route("/v1/teams/form", post(handlers::fleet::team_form))
        .route("/v1/teams/:id/wallet", get(handlers::fleet::team_wallet))
        .route("/v1/teams/:id/split", post(handlers::fleet::team_split))
        .route(
            "/v1/teams/:id/earnings",
            get(handlers::fleet::team_earnings),
        )
        .route(
            "/v1/federation/invoke",
            post(handlers::fleet::federation_invoke),
        )
        .route(
            "/v1/federation/discover",
            get(handlers::fleet::federation_discover),
        )
        .route(
            "/v1/federation/settle",
            post(handlers::fleet::federation_settle),
        )
        // Agent skill invocation (hiring)
        .route("/v1/agents/:id/invoke", post(handlers::fleet::invoke_agent))
        .route("/v1/agents/hire", post(handlers::fleet::hire_agent))
        // Uploads
        .route("/v1/uploads", post(upload_handlers::upload_file))
        .route("/v1/uploads", get(upload_handlers::list_uploads))
        .route("/v1/uploads/:id", get(upload_handlers::get_upload_metadata))
        .route(
            "/v1/uploads/:id/download",
            get(upload_handlers::download_file),
        )
        .route(
            "/v1/uploads/:id/thumbnail",
            get(upload_handlers::get_thumbnail),
        )
        .route("/v1/uploads/:id", delete(upload_handlers::delete_upload))
        // WebSocket
        .route("/v1/ws", get(websocket::ws_handler))
        .route("/v1/ws/pubkey", get(websocket::ws_pubkey_handler))
        // Node WebSocket (fleet agent connections)
        .route("/v1/ws/nodes", get(node_ws::node_ws_handler))
        // Node management (REST)
        .route("/v1/nodes", get(handlers::list_nodes))
        .route("/v1/nodes/broadcast", post(handlers::broadcast_nodes))
        .route("/v1/nodes/:id", get(handlers::get_node))
        .route("/v1/nodes/:id/invoke", post(handlers::invoke_node))
        .route("/v1/nodes/:id/event", post(handlers::send_node_event))
        // Security - key rotation
        .route(
            "/v1/security/rotate-key",
            post(key_middleware::handle_rotate_key),
        )
        .route(
            "/v1/security/rotation-status",
            get(key_middleware::handle_rotation_status),
        )
        // Cron jobs (Prometheus scheduler)
        .route("/v1/cron/jobs", get(handlers::cron::list_cron_jobs))
        .route("/v1/cron/jobs", post(handlers::cron::create_cron_job))
        .route(
            "/v1/cron/jobs/running",
            get(handlers::cron::list_running_cron_jobs),
        )
        .route("/v1/cron/jobs/:id", delete(handlers::cron::delete_cron_job))
        .route(
            "/v1/cron/jobs/:id/abort",
            post(handlers::cron::abort_cron_job),
        )
        .route(
            "/v1/cron/jobs/:id/history",
            get(handlers::cron::cron_job_history),
        )
        .route(
            "/v1/cron/templates",
            get(handlers::cron::list_cron_templates),
        )
        // Observatory Dashboard
        .route(
            "/v1/observatory/active-tasks",
            get(handlers::observatory::active_tasks),
        )
        .route(
            "/v1/observatory/agent-stats",
            get(handlers::observatory::agent_stats),
        )
        .route(
            "/v1/observatory/channel-health",
            get(handlers::observatory::channel_health),
        )
        .route(
            "/v1/observatory/cost-live",
            get(handlers::observatory::cost_live),
        )
        // Documentation site
        // Pantheon — multi-agent collaboration missions
        // Fleet agent registration (GlobalStateManager for Pantheon team assembly)
        .route("/v1/fleet", get(handlers::fleet::list_fleet_agents))
        .route("/v1/fleet/sync", post(handlers::fleet::github_webhook_sync))
        .route("/v1/fleet/register", post(handlers::fleet::register_agent))
        .route("/v1/fleet/execute", post(handlers::fleet::fleet_execute))
        .route("/v1/fleet/protocol", post(handlers::fleet::fleet_protocol))
        .route(
            "/v1/fleet/provision",
            post(handlers::fleet_provisioner::fleet_provision),
        )
        .route(
            "/v1/fleet/provision/jobs",
            get(handlers::fleet_provisioner::provision_jobs_list),
        )
        .route(
            "/v1/fleet/provision/status/:id",
            get(handlers::fleet_provisioner::provision_status),
        )
        .route("/v1/fleet/:id", get(handlers::fleet::get_fleet_agent))
        .route("/v1/fleet/:id", delete(handlers::fleet::deregister_agent))
        .route(
            "/v1/fleet/:id/heartbeat",
            post(handlers::fleet::fleet_heartbeat),
        )
        // Pantheon DMs (direct 1:1 rooms between agents)
        // S63: Office message stream
        .route("/v1/office/stream", get(handlers::office_message_stream))
        // S86: Office state (Star Office game)
        .route("/v1/office/state", get(handlers::office_state))
        .route("/v1/office/join", post(handlers::office_join))
        .route("/v1/office/leave", post(handlers::office_leave))
        .route("/v1/office/agents/stream", get(handlers::agent_status_stream))
        .route("/v1/agents/:id/zone", put(handlers::assign_agent_zone))
        .route("/v1/pantheon/dms", post(handlers::find_or_create_dm))
        .route("/v1/pantheon/dms", get(handlers::list_dms))
        // Pantheon rooms (war room chat)
        .route("/v1/pantheon/rooms", post(handlers::create_room))
        .route("/v1/pantheon/rooms", get(handlers::list_rooms))
        .route("/v1/pantheon/rooms/:id", get(handlers::get_room))
        .route("/v1/pantheon/rooms/:id/join", post(handlers::join_room))
        .route("/v1/pantheon/rooms/:id/leave", post(handlers::leave_room))
        .route(
            "/v1/pantheon/rooms/:id/messages",
            post(handlers::send_room_message),
        )
        .route(
            "/v1/pantheon/rooms/:id/messages",
            get(handlers::get_room_messages),
        )
        .route(
            "/v1/pantheon/rooms/:id/upload",
            post(handlers::upload_room_file),
        )
        .route(
            "/v1/pantheon/rooms/:id/members",
            get(handlers::list_room_members),
        )
        // Skill cards (Agora → Pantheon social wiring)
        .route(
            "/v1/pantheon/rooms/:id/skill-card",
            post(handlers::share_skill_card),
        )
        // Chat ops
        .route(
            "/v1/pantheon/rooms/:id/messages/:msg_id",
            delete(handlers::delete_room_message),
        )
        .route(
            "/v1/pantheon/rooms/:id/messages/:msg_id",
            put(handlers::edit_room_message),
        )
        .route(
            "/v1/pantheon/rooms/:id/messages/:msg_id/reactions",
            post(handlers::add_reaction),
        )
        .route(
            "/v1/pantheon/rooms/:id/messages/:msg_id/reactions",
            delete(handlers::remove_reaction),
        )
        .route(
            "/v1/pantheon/rooms/:id/messages/:msg_id/reactions",
            get(handlers::get_reactions),
        )
        // Identity
        .route("/v1/pantheon/identity", put(handlers::set_identity))
        .route("/v1/pantheon/identity/:id", get(handlers::get_identity))
        .route("/v1/pantheon/identities", get(handlers::list_identities))
        // Sentient Intelligence — Agent Reputation (S59-T5)
        .route("/v1/pantheon/reputation/:agent_id", get(handlers::pantheon::get_reputation))
        .route("/v1/pantheon/reputation/:agent_id", post(handlers::pantheon::update_reputation))
        .route("/v1/pantheon/leaderboard", get(handlers::pantheon::reputation_leaderboard))
        // Plan approval
        .route(
            "/v1/pantheon/plans/pending",
            get(handlers::list_pending_plans),
        )
        .route(
            "/v1/pantheon/plans/:id/approve",
            post(handlers::approve_plan),
        )
        .route("/v1/pantheon/plans/:id/reject", post(handlers::reject_plan))
        // Agora economy dashboard
        .route("/v1/pantheon/economy", get(handlers::pantheon_economy))
        // Pantheon missions
        .route("/v1/pantheon/missions", post(handlers::create_mission))
        .route("/v1/pantheon/missions", get(handlers::list_missions))
        .route("/v1/pantheon/missions/:id", get(handlers::get_mission))
        .route(
            "/v1/pantheon/missions/:id/intervene",
            post(handlers::intervene_mission),
        )
        .route(
            "/v1/pantheon/missions/:id/approve",
            post(handlers::approve_mission),
        )
        .route(
            "/v1/pantheon/missions/:id/feed",
            get(handlers::get_mission_feed),
        )
        .route(
            "/v1/pantheon/missions/:id/artifacts",
            get(handlers::get_mission_artifacts),
        )
        .route(
            "/v1/pantheon/missions/:id/artifacts/:name/download",
            get(handlers::download_mission_artifact),
        )
        .route(
            "/v1/pantheon/missions/:id/events",
            get(handlers::mission_events),
        )
        .route(
            "/v1/pantheon/missions/:id/review",
            post(handlers::review_task),
        )
        .route("/docs", get(docs::docs_index))
        .route("/docs/openapi.json", get(docs::docs_openapi))
        .route("/docs/tools", get(docs::docs_tools))
        .route("/docs/config", get(docs::docs_config))
        .route("/docs/getting-started", get(docs::docs_getting_started))
        // Live Canvas (A2UI)
        .route("/v1/canvas/render", post(handlers::canvas::canvas_render))
        .route(
            "/v1/canvas/components",
            get(handlers::canvas::canvas_components),
        )
        // DM Pairing / Channel Auth
        .route("/v1/channels/:id/pair", post(handlers::pair_channel))
        .route("/v1/channels/:id/verify", post(handlers::verify_channel))
        .route(
            "/v1/channels/:id/pairings",
            get(handlers::list_channel_pairings),
        )
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10 MB global body limit
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Add rate limiting if configured (apply first for efficiency)
    if let Some(config) = rate_limit_config {
        let limiter = HttpRateLimiter::new(config);
        limiter.start_cleanup_task();
        router = router.layer(RateLimitLayer::new(limiter));
    }

    // S95: Serve WebUI static files from ~/.zeus/web/ as fallback.
    // API routes take priority; anything else serves the SPA.
    let webui_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("web");
    if webui_dir.exists() {
        tracing::info!("Serving WebUI from {}", webui_dir.display());
        let serve_dir = ServeDir::new(&webui_dir)
            .append_index_html_on_directories(true)
            .fallback(ServeDir::new(&webui_dir).append_index_html_on_directories(true));
        router = router.fallback_service(serve_dir);
    }

    router
}

/// Normalize a URL path for safe prefix matching.
///
/// Decodes percent-encoded characters, collapses repeated slashes,
/// resolves `.` and `..` segments, and strips trailing slashes.
/// This prevents bypass attacks using URL encoding or path manipulation.
fn normalize_path(path: &str) -> String {
    // Percent-decode the path
    let decoded = urlencoding::decode(path)
        .unwrap_or(std::borrow::Cow::Borrowed(path))
        .to_string();

    // Collapse repeated slashes and resolve . / ..
    let mut segments: Vec<&str> = Vec::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }

    format!("/{}", segments.join("/"))
}

/// Check if a path should be publicly accessible (no auth required).
/// Blog read endpoints (GET) are public for the marketing site;
/// blog write endpoints (POST/PUT/DELETE) still require auth.
fn is_public_path(path: &str, method: &Method) -> bool {
    let normalized = normalize_path(path);

    // Always-public paths (any method)
    if normalized == "/"
        || normalized == "/health"
        || normalized == "/health/detailed"
        || normalized.starts_with("/docs")
        || (normalized.starts_with("/v1/auth/") && normalized.len() > "/v1/auth/".len())
        || (normalized.starts_with("/v1/onboarding/") && normalized.len() > "/v1/onboarding/".len())
        || normalized == "/v1/providers"
        || normalized == "/v1/config/providers"
        || normalized == "/v1/config/test"
        || normalized == "/v1/config/models"
        || normalized == "/v1/goals/analyze"
        || normalized == "/v1/config"
    {
        return true;
    }

    // Blog endpoints — only GET is public (read-only for marketing site)
    if method == Method::GET
        && (normalized == "/v1/blog/posts"
            || normalized.starts_with("/v1/blog/posts/")
            || normalized == "/v1/blog/tags"
            || normalized == "/v1/blog/images"
            || normalized.starts_with("/v1/blog/images/"))
    {
        return true;
    }

    false
}

/// Check if a path requires 2FA (blog write operations).
fn requires_2fa(path: &str, method: &Method) -> bool {
    if *method == Method::GET {
        return false;
    }
    let normalized = normalize_path(path);
    normalized.starts_with("/v1/blog/posts") || normalized.starts_with("/v1/blog/images")
}

/// TOTP 2FA middleware — enforces `X-Zeus-2FA-Token` JWT header on blog mutations.
///
/// Only active when TOTP is set up and enabled. If no TOTP user exists,
/// requests pass through (backward compatible).
async fn totp_2fa_middleware(
    req: Request,
    next: Next,
    state: SharedState,
) -> Result<Response, StatusCode> {
    if !requires_2fa(req.uri().path(), req.method()) {
        return Ok(next.run(req).await);
    }

    // Check if TOTP is enabled
    let store = state.read().await.totp_store.clone();
    let user = match store.get_user("blog_admin").await {
        Some(u) if u.enabled => u,
        _ => return Ok(next.run(req).await), // No TOTP setup or not enabled — pass through
    };
    drop(user);

    // Extract and validate 2FA token
    let token = req
        .headers()
        .get("x-zeus-2fa-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;

    // Validate JWT signature + expiry + purpose
    let _claims = handlers::blog_auth::validate_jwt(token).ok_or(StatusCode::FORBIDDEN)?;

    // Verify session exists in TotpStore (server-side revocation)
    let token_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    };
    if !store.validate_session(&token_hash).await {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}

/// Bearer token + API key authentication middleware.
///
/// Accepts either:
/// - `Authorization: Bearer <token>` header (existing)
/// - `X-Zeus-Api-Key: <key>` header (new)
async fn auth_middleware(
    req: Request,
    next: Next,
    expected_token: String,
    api_keys: ApiKeyValidator,
) -> Result<Response, StatusCode> {
    // Allow public endpoints without auth (using normalized path)
    if is_public_path(req.uri().path(), req.method()) {
        return Ok(next.run(req).await);
    }

    // Check Authorization: Bearer <token> header
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    if let Some(value) = auth_header
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        // Constant-time comparison to prevent timing attacks
        if token.len() == expected_token.len()
            && token
                .as_bytes()
                .iter()
                .zip(expected_token.as_bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        {
            return Ok(next.run(req).await);
        }
    }

    // Check X-Zeus-Api-Key header
    if let Some(api_key) = req
        .headers()
        .get("x-zeus-api-key")
        .and_then(|v| v.to_str().ok())
        && api_keys.validate(api_key)
    {
        return Ok(next.run(req).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

/// API key-only authentication middleware (when no Bearer token is configured).
async fn api_key_only_middleware(
    req: Request,
    next: Next,
    api_keys: ApiKeyValidator,
) -> Result<Response, StatusCode> {
    if is_public_path(req.uri().path(), req.method()) {
        return Ok(next.run(req).await);
    }

    if let Some(api_key) = req
        .headers()
        .get("x-zeus-api-key")
        .and_then(|v| v.to_str().ok())
        && api_keys.validate(api_key)
    {
        return Ok(next.run(req).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

/// Middleware applied when no auth token is configured.
/// When onboarding is complete, allows all requests (user has configured their instance).
/// Otherwise blocks all endpoints except health, auth, and onboarding.
async fn no_token_middleware(
    req: Request,
    next: Next,
    state: SharedState,
) -> Result<Response, StatusCode> {
    if is_public_path(req.uri().path(), req.method()) {
        return Ok(next.run(req).await);
    }
    // If onboarding is complete, the user has configured their instance — allow all requests.
    // This enables ZeusWeb to function without requiring a separate ZEUS_API_TOKEN.
    if state.read().await.config.onboarding_complete {
        return Ok(next.run(req).await);
    }
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

/// Build a CORS layer with restricted origins (defaults to localhost only)
fn build_cors_layer(allowed_origins: &[String]) -> CorsLayer {
    use tower_http::cors::AllowOrigin;

    let origins: Vec<HeaderValue> = if allowed_origins.is_empty() {
        // Default: localhost + marketing site (.230)
        vec![
            HeaderValue::from_static("http://127.0.0.1"),
            HeaderValue::from_static("http://localhost"),
        ]
    } else {
        allowed_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect()
    };

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-zeus-api-key"),
            axum::http::HeaderName::from_static("x-zeus-2fa-token"),
        ])
}
