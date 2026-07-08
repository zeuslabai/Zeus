//! Zeus TUI library — onboarding wizard + production interface, plus the
//! integrated entrypoint [`run`] used by the root `zeus` binary.

pub mod api;
pub mod theme;
pub mod crash_log;
pub mod widgets;
pub mod screens;
pub mod prod;
pub mod app;
pub mod model_fetch;
pub mod awaken;
pub mod gateway_target;

pub use app::{App, FooterFocus};
pub use gateway_target::{GatewayTargetOverride, resolve_gateway_target};

/// Launch the Zeus TUI integrated with the gateway described by `config`.
///
/// Called by the root `zeus` binary (`zeus` / `zeus tui`). The standalone
/// `zeus-tui` binary instead calls [`app::run_standalone`] directly.
///
/// Phase 1 launches the new TUI; Phase 2 wires live gateway data (tools,
/// channels, chat/status, …) into the `App` from `config` before the loop.
///
/// Returns `Ok(true)` iff onboarding *just completed this run* — i.e. the
/// `onboarding_complete` flag flipped `false → true` between launch and exit.
/// The integrated parent (`run_tui` in the root `zeus` binary) uses this as
/// the unambiguous signal to launch the gateway (AWAKEN-B). An install that
/// was already onboarded (flag `true` at launch) returns `Ok(false)` — no
/// double-launch. The in-memory flip is the correct signal; re-reading config
/// post-run cannot distinguish "just onboarded" from "was already onboarded".
pub async fn run(config: zeus_core::Config) -> anyhow::Result<bool> {
    run_with_force(config, false).await
}

/// Like [`run`], but when `force_onboard` is true the onboarding flow is
/// entered unconditionally — bypassing `needs_onboarding()`. This is the
/// `zeus onboard` / `--reconfigure` path: a healthy config (non-empty model,
/// `loaded_from_default=false`) would otherwise short-circuit `needs_onboarding()`
/// to false and never onboard. Forcing the flag false here drives the App into
/// the onboarding screen regardless of disk state, so a single `zeus onboard`
/// can repair a nuked/wiped config.
pub async fn run_with_force(
    config: zeus_core::Config,
    force_onboard: bool,
) -> anyhow::Result<bool> {
    run_with_force_and_gateway(config, force_onboard, None).await
}

/// Like [`run_with_force`], but with an optional non-persistent gateway target
/// override from `zeus tui --port/--gateway-url`.
pub async fn run_with_force_and_gateway(
    config: zeus_core::Config,
    force_onboard: bool,
    gateway_override: Option<GatewayTargetOverride>,
) -> anyhow::Result<bool> {
    use std::sync::{Arc, Mutex};

    let gateway_target = resolve_gateway_target(&config, gateway_override.as_ref());
    let host = gateway_target.display_host.clone();
    let port = gateway_target.display_port;
    let gateway_url = gateway_target.base_url;
    let agent_name = config
        .agent
        .as_ref()
        .and_then(|a| a.name.as_deref())
        .or(config.name.as_deref())
        // #296: no match-all "zeus" default — sentinel for an unnamed agent.
        .unwrap_or("<unnamed agent>")
        .to_string();

    // Shared app state: background tasks update it while the render loop reads.
    let app = Arc::new(Mutex::new(app::App::new_from_disk()));
    // #267 FORCE: `zeus onboard` (and `--reconfigure`) must enter onboarding
    // even on a healthy config. `new_from_disk` sets `onboarding_complete` from
    // `!needs_onboarding()`, which is true (skip onboarding) for any config with
    // a non-empty model — so without this override the wizard never re-runs.
    // Force the flag false BEFORE the launch snapshot so the false→true
    // completion flip still fires AWAKEN-B's gateway launch.
    if force_onboard {
        let mut a = app.lock().unwrap_or_else(|e| e.into_inner());
        a.onboarding_complete = false;
    }
    // AWAKEN-B signal: snapshot the onboarding flag at launch. If it flips
    // false→true during this run, onboarding just completed and the parent
    // must launch the gateway. An already-onboarded install starts true here.
    let onboarded_at_launch = {
        let a = app.lock().unwrap_or_else(|e| e.into_inner());
        a.onboarding_complete
    };
    // Clone the Arc so we can read the final flag after `run_loop` consumes
    // the original `app` via `spawn_blocking`.
    let awaken_app = app.clone();
    {
        let mut a = app.lock().unwrap_or_else(|e| e.into_inner());
        a.gateway_host = host;
        a.gateway_port = port;
        a.agent_name = agent_name;
        a.conn_state = prod::top_bar::ConnState::Connecting;
    }

    // Live connection probe: poll /v1/status every 5s, reflect it in the top
    // bar, and refresh the agent name from the gateway when it reports one.
    // Also stores the full StatusResponse for TopBar/StatusBar live data (#235).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                // Bound the probe so a *wedged* gateway (accepts the TCP
                // connect but never responds — distinct from connection-refused,
                // which errors fast) flips the badge red on the next cycle
                // instead of being held green for the shared 30s
                // NON_STREAMING_TIMEOUT. 4s < 5s poll interval (#276).
                let probe = tokio::time::timeout(
                    std::time::Duration::from_secs(4),
                    client.status(),
                )
                .await;
                let (state, status_resp) = match probe {
                    Ok(Ok(st)) => {
                        let name = st.agent_name.clone();
                        let resp = Some(st);
                        if !name.is_empty()
                            && let Ok(mut a) = app.lock() {
                                a.agent_name = name;
                            }
                        (prod::top_bar::ConnState::Connected, resp)
                    }
                    // Probe error OR timeout (wedged gateway) → disconnected.
                    _ => (prod::top_bar::ConnState::Disconnected, None),
                };
                if let Ok(mut a) = app.lock() {
                    a.conn_state = state;
                    a.prod_status = status_resp;
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    // Live channels: poll /v1/channels every 10s and render the real gateway
    // adapter rows. The Channels tab displays `—` for fields the API omits.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(channels) = client.channels().await
                    && let Ok(mut a) = app.lock()
                {
                    a.prod_channels = Some(channels);
                }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Tool registry: fetch /v1/tools once at startup and leak into a 'static
    // slice. The registry is stable for the session, so a single bounded leak
    // (lives for the program's lifetime, like a const) keeps the render simple.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            if let Ok(tools) = client.tools().await
                && !tools.is_empty() {
                    let entries: Vec<prod::tools_tab::ToolEntry> = tools
                        .into_iter()
                        .map(|t| prod::tools_tab::ToolEntry {
                            name: Box::leak(t.name.into_boxed_str()),
                            category: Box::leak(t.category.into_boxed_str()),
                            desc: Box::leak(t.description.into_boxed_str()),
                            danger: false,
                            schema: Box::leak(
                                serde_json::to_string(&t.parameters)
                                    .unwrap_or_else(|_| "{}".to_string())
                                    .into_boxed_str(),
                            ),
                        })
                        .collect();
                    let leaked: &'static [prod::tools_tab::ToolEntry] =
                        Box::leak(entries.into_boxed_slice());
                    if let Ok(mut a) = app.lock() {
                        a.live_tools = Some(leaked);
                    }
                }
        });
    }

    // Live config: poll GET /v1/config every 10s and reflect the sanitized
    // map into App state. The Settings tab overlays these live values onto its
    // static section schema; until the first fetch lands the tab shows const
    // placeholders. GOTCHA: `app` is std::sync::Mutex — never .await the lock.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(cfg) = client.config().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_config_rows = Some(cfg);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live tasks: poll GET /v1/tasks/active every 2s. The chat tab's task-
    // tracker widget (#280) overlays the live agent todo list (fed by
    // todo_write) onto the message area, Claude-Code style. A 2s cadence keeps
    // it in step with the chat stream without hammering the gateway. GOTCHA:
    // `app` is std::sync::Mutex — never .await the lock.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(tasks) = client.active_tasks().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_active_tasks = Some(tasks);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
    }

    // Live memory: poll GET /v1/memory/files every 10s. The Memory→Workspace
    // sub-tab overlays the live file tree onto its const placeholder until the
    // first fetch lands. GOTCHA: `app` is std::sync::Mutex — never .await the lock.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(files) = client.memory_files().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_memory_files = Some(files);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live agents: poll GET /v1/network/agents every 10s. The Advanced→Agents
    // subview overlays live name/status/local onto the const roster (#185).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(agents) = client.agents().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_agents = Some(agents);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live skills: poll GET /v1/skills every 10s. The Advanced→Skills subview
    // overlays live name/category/enabled onto the const list (#185 wiring).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(skills) = client.skills().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_skills = Some(skills);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live MCP servers: poll GET /v1/mcp/servers every 10s. The Advanced→MCP
    // subview overlays live name/transport onto the const list (#185 wiring).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(servers) = client.mcp_servers().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_mcp = Some(servers);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live TTS providers + voices: poll GET /v1/tts/providers + /v1/tts/voices
    // every 10s. The Advanced→Voice subview overlays live provider/voice data
    // onto the TTS PROVIDER card; STT/Twilio/Recordings have no backend and
    // stay const (honest in-panel stub) (#185 wiring).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(providers) = client.tts_providers().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_tts_providers = Some(providers);
                    }
                if let Ok(voices) = client.tts_voices().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_tts_voices = Some(voices);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live workflows: poll GET /v1/workflows every 10s. The Advanced→Canvas
    // subview overlays live chat→DAG instances onto the node graph (each
    // workflow = one node; status→state, completed/total→progress). Falls back
    // to the const placeholder graph until the first fetch (#185 wiring).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(workflows) = client.workflows().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_workflows = Some(workflows);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live extensions: poll GET /v1/extensions every 10s. The
    // Advanced→Extensions subview overlays live name/version/status/runtime
    // onto the const list, falling back until the first fetch (#185 wiring).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(extensions) = client.extensions().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_extensions = Some(extensions);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live projects: poll GET /v1/projects every 10s. The Advanced→Projects
    // subview overlays live name/status/agent-count onto the const roster
    // (lead/progress unbacked → `—`/0), falling back until the first fetch.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(projects) = client.projects().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_projects = Some(projects);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live nodes: poll GET /v1/nodes every 10s. The Advanced→NodeComms
    // FLEET LINKS section overlays live peer←node_id (up=true; transport/rtt
    // honestly `—`), falling back to the const roster until the first fetch.
    // RECENT MESSAGES stays a const honest-stub (no message-feed backend).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(nodes) = client.nodes().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_nodes = Some(nodes);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live spawns: poll GET /v1/spawner/active every 10s. The Advanced→Spawner
    // subview overlays live name←agent_id / task / runtime←(now−started_at),
    // status="running" (endpoint lists only active spawns), channels honestly 0
    // (unbacked by tracker), falling back to the const roster until first fetch.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(spawns) = client.spawner_active().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_spawns = Some(spawns);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live vector stores: poll GET /v1/vector_stores every 10s. The
    // Advanced→VectorStores COLLECTIONS section overlays live name·files·status;
    // the design's vectors/dim/model have no backend (file ≠ vector; no dim/
    // model field) so they're honest-dashed (server-extension gap). SEMANTIC
    // SEARCH stays a const honest-stub (POST-only endpoint, no history store).
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(stores) = client.vector_stores().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_vector_stores = Some(stores);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live KG communities: poll GET /v1/memory/communities every 10s. The
    // Advanced→Knowledge-Graph COMMUNITY·NODES columns overlay live name +
    // entity_count; per-community EDGES have no backend (Community carries no
    // edge field; edges are global-only) so they're honest-dashed (server-
    // extension gap). The summary's community/node totals are live-derived;
    // facts has no backing field and stays dashed.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(comms) = client.communities().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_communities = Some(comms);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live deploy: poll GET /v1/deploy/targets + /history + /stats every 10s.
    // The Advanced→Deploy panel overlays live targets (name/provider/env/url/
    // active) and recent deployments (target/version/status/trigger); the
    // summary line uses live stats. Replaces the old daemon-health mock — the
    // panel's real subject is the deploy-target store (merakizzz "wire all").
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(targets) = client.deploy_targets().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_deploy_targets = Some(targets);
                    }
                if let Ok(history) = client.deploy_history().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_deploy_history = Some(history);
                    }
                if let Ok(stats) = client.deploy_stats().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_deploy_stats = Some(stats);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Economy: poll GET /v1/economy/wallets (AGORA WALLET card) and
    // /v1/economy/transactions (RECENT TRANSACTIONS) every 10s. Read-side only
    // — balances/transactions live off the token ledger (merakizzz "wire all";
    // #190 human→titan token-SEND stays future-scope). Replaces the old
    // "$ 247.83 USDC" mock; credits are an integer ledger balance.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(wallets) = client.economy_wallets().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_economy_wallets = Some(wallets);
                    }
                if let Ok(txs) = client.economy_transactions().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_economy_txs = Some(txs);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live approvals: poll GET /v1/approvals every 10s. The Approvals tab
    // overlays live pending tool-approval requests onto the const catalog
    // (#235). Falls back to DEFAULT_PENDING until the first fetch lands.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(approvals) = client.approvals().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_approvals = Some(approvals);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live Pantheon missions: poll GET /v1/pantheon/missions every 10s. The
    // Pantheon tab overlays live name/status/agent_count onto the const
    // MISSIONS catalog (#235). Falls back to const until the first fetch lands.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(missions) = client.pantheon_missions().await
                    && let Ok(mut a) = app.lock() {
                        a.prod_pantheon_missions = Some(missions);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live sessions: poll GET /v1/sessions (cap 50) every 10s. The
    // Memory→Sessions sub-tab overlays live summaries onto const rows.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(sessions) = client.sessions(50).await
                    && let Ok(mut a) = app.lock() {
                        a.prod_sessions = Some(sessions);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Live Mnemosyne: seed the Memory→Mnemosyne sub-tab with a broad initial
    // search so the panel shows real hits instead of const placeholders. There
    // is no search-input UI yet (Wave-2); a single fixed query de-mocks the
    // panel and refreshes every 30s. GOTCHA: empty query is rejected server
    // side, so use a stable broad term.
    {
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            loop {
                if let Ok(hits) = client.memory_search("zeus", 20).await
                    && let Ok(mut a) = app.lock() {
                        a.prod_memory_search = Some(hits);
                    }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }

    // Chat worker: bridge the (sync) UI submit channel to async gateway calls.
    // The UI loop runs on a blocking thread, so submits arrive over an mpsc
    // channel; this task calls the gateway and appends the reply to the app.
    {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        {
            let mut a = app.lock().unwrap_or_else(|e| e.into_inner());
            a.chat_tx = Some(tx);
        }
        // Dedicated handles for the fetch + ollama-probe workers below, cloned
        // BEFORE the chat worker shadows `app` with its own move-clone.
        let fetch_app = app.clone();
        let probe_app = app.clone();
        let app = app.clone();
        let client = api::ApiClient::new(&gateway_url);
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                let app_cb = app.clone();
                let reply = client.chat_stream(&message, move |event| {
                    use crate::api::SseEvent;
                    if let Ok(mut a) = app_cb.lock() {
                        match event {
                            SseEvent::Token(token) => {
                                a.push_stream_token(token);
                            }
                            SseEvent::Thinking(text) => {
                                a.push_stream_thinking(text);
                            }
                            SseEvent::ToolStart { name, input } => {
                                a.push_tool_start(name, input);
                            }
                            SseEvent::ToolEnd { name, output } => {
                                a.push_tool_end(name, output);
                            }
                            SseEvent::Iter(n) => {
                                a.push_iter(n);
                            }
                            SseEvent::Usage { .. } => {}
                        }
                    }
                }).await;
                let reply_text = match reply {
                    Ok(text) => text,
                    Err(e) => format!("[gateway error] {e}"),
                };
                if let Ok(mut a) = app.lock() {
                    a.push_assistant_reply(reply_text);
                }
            }
        });

        // Live model-fetch worker (#239/#240): mirrors the chat worker above.
        // Advancing past Auth sends (provider_id, api_key); we call the
        // provider's /v1/models with a bounded 9s timeout so a dead endpoint
        // can't wedge onboarding, then write the result back under the lock.
        // GOTCHA: `app` is std::sync::Mutex (not tokio) — never .await the lock.
        let (fetch_tx, mut fetch_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        {
            let mut a = fetch_app.lock().unwrap_or_else(|e| e.into_inner());
            a.fetch_tx = Some(fetch_tx);
        }
        let fetch_app = fetch_app.clone();
        tokio::spawn(async move {
            while let Some((provider_id, api_key)) = fetch_rx.recv().await {
                let state = match tokio::time::timeout(
                    std::time::Duration::from_secs(9),
                    crate::model_fetch::fetch_models(&provider_id, &api_key),
                )
                .await
                {
                    Ok(Ok(models)) => crate::model_fetch::ModelFetchState::Done(models),
                    Ok(Err(e)) => crate::model_fetch::ModelFetchState::Failed(e),
                    Err(_) => crate::model_fetch::ModelFetchState::Failed(
                        "timed out after 9s".to_string(),
                    ),
                };
                if let Ok(mut a) = fetch_app.lock() {
                    a.model_fetch_state = state;
                }
            }
        });

        // #260: LIVE Ollama probe for the Memory step's `● DETECTED` badge/banner.
        // One-shot, fire-and-forget: GET `{ollama_url}/api/tags` with a 2s
        // bounded timeout so a missing Ollama can't hang onboarding. Writes the
        // boolean result under the app lock; the Memory screen renders an honest
        // "not detected" until/unless this confirms reachability (#251 rule).
        let ollama_url = config.ollama.url.trim_end_matches('/').to_string();
        tokio::spawn(async move {
            let endpoint = format!("{ollama_url}/api/tags");
            let reachable = match reqwest::Client::new()
                .get(&endpoint)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                Ok(resp) => resp.status().is_success(),
                Err(_) => false,
            };
            if let Ok(mut a) = probe_app.lock() {
                a.memory_screen.set_ollama_detected(reachable);
            }
        });
    }

    // Run the blocking terminal loop off the async workers so the status poll
    // keeps progressing regardless of the runtime flavor.
    tokio::task::spawn_blocking(move || app::run_loop(app))
        .await
        .map_err(|e| anyhow::anyhow!("TUI task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("TUI exited with error: {e}"))?;

    // AWAKEN-B: did onboarding complete this run? (false→true flip only)
    let just_onboarded = {
        let a = awaken_app.lock().unwrap_or_else(|e| e.into_inner());
        a.onboarding_complete && !onboarded_at_launch
    };
    Ok(just_onboarded)
}
