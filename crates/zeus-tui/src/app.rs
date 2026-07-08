//! The Zeus TUI application — onboarding wizard + production interface.
//!
//! Lives in the library so both the standalone `zeus-tui` binary (`main.rs`) and
//! the integrated entrypoint `zeus_tui::run(config)` (called by the root `zeus`
//! binary) drive the same code. See [`run_standalone`] / [`run_loop`].

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};
use std::io;

// Bring sibling module names into scope so inline `prod::`, `screens::`, `theme::`
// paths resolve from within this library module (they were crate-root in the bin).
use crate::{prod, screens, theme};

use crate::screens::complete::SummaryRow;
use crate::screens::{
    AgentScreen, AuthScreen, ChanConfigScreen, ChannelsScreen, CompleteScreen, FallbackScreen,
    FeaturesScreen, GatewayScreen, ImagesScreen, InstanceScreen, MemoryScreen, ModeScreen,
    ModelScreen, OrchestrationScreen, ProviderScreen, SecurityScreen, SkillsScreen, VoiceScreen,
    WelcomeScreen, WorkspaceScreen,
};
use crate::widgets::{StatusBar, StepHeader, StepIndicator, TopBar};

use crate::prod::advanced::{ADVANCED_TABS, AdvTabDef};
use crate::prod::chat_tab::{ChatMessage, Role, StreamState};
use crate::prod::stub_tabs::StubTab;
use crate::prod::tab_bar::PRIMARY_TABS;
use crate::prod::top_bar::ConnState;
use crate::prod::{
    AdvancedOverlay, ApprovalsTab, ChannelsTab, ChatTab, ProdStatusBar, ProdTabBar, ProdTopBar,
    ToolsTab,
};

/// Onboarding step index — indexes into widgets::top_bar::STEPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome = 0,
    Mode = 1,
    Instance = 2,
    Provider = 3,
    Auth = 4,
    Model = 5,
    Fallback = 6,
    Channels = 7,
    ChannelConfig = 8,
    Gateway = 9,
    Agent = 10,
    Workspace = 11,
    Security = 12,
    Features = 13,
    Voice = 14,
    Images = 15,
    Orchestration = 16,
    Memory = 17,
    Skills = 18,
    Complete = 19,
}

impl Step {
    fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Step::Welcome),
            1 => Some(Step::Mode),
            2 => Some(Step::Instance),
            3 => Some(Step::Provider),
            4 => Some(Step::Auth),
            5 => Some(Step::Model),
            6 => Some(Step::Fallback),
            7 => Some(Step::Channels),
            8 => Some(Step::ChannelConfig),
            9 => Some(Step::Gateway),
            10 => Some(Step::Agent),
            11 => Some(Step::Workspace),
            12 => Some(Step::Security),
            13 => Some(Step::Features),
            14 => Some(Step::Voice),
            15 => Some(Step::Images),
            16 => Some(Step::Orchestration),
            17 => Some(Step::Memory),
            18 => Some(Step::Skills),
            19 => Some(Step::Complete),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome to Zeus",
            Step::Mode => "Setup Mode",
            Step::Instance => "Choose instance",
            Step::Provider => "Pick your LLM provider",
            Step::Auth => "Authenticate",
            Step::Model => "Pick a model",
            Step::Fallback => "Backup LLMs",
            Step::Channels => "Pick messaging channels",
            Step::ChannelConfig => "Configure channels",
            Step::Gateway => "Configure gateway",
            Step::Agent => "Agent persona",
            Step::Workspace => "Workspace paths",
            Step::Security => "Security level",
            Step::Features => "Feature flags",
            Step::Voice => "Voice synthesis",
            Step::Images => "Image generation",
            Step::Orchestration => "Orchestration mode",
            Step::Memory => "Memory backend",
            Step::Skills => "Install skills",
            Step::Complete => "Setup complete",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Step::Welcome => "Autonomous AI agents on your hardware",
            Step::Mode => "Choose how you want to configure Zeus",
            Step::Instance => "Default ~/.zeus or preview a named instance home",
            Step::Provider => "Primary model that powers agent reasoning",
            Step::Auth => "Enter your API key",
            Step::Model => {
                "From the provider's catalog. You can change anytime via zeus config set model ..."
            }
            Step::Fallback => "Pick 0-3 fallback providers for when your primary is down.",
            Step::Channels => {
                "Select which channels Zeus should bridge. Per-channel credentials collected next."
            }
            Step::ChannelConfig => "Per-channel credentials for the channels you selected.",
            Step::Gateway => "The gateway hosts the API, WebUI, and agent processing loop.",
            Step::Agent => {
                "Pick an archetype to seed your agent's SOUL.md. Customize freely after onboarding."
            }
            Step::Workspace => {
                "Where Zeus stores your agent's working memory, sessions, and journal."
            }
            Step::Security => {
                "Sandbox aggressiveness for tool execution. Approval pipeline always active."
            }
            Step::Features => {
                "Toggle which Zeus crates are active in this deployment. Disabled crates compile but don't load."
            }
            Step::Voice => "Give your agent a voice. Pick a TTS provider or skip with None.",
            Step::Images => "Powers image_generate and image_edit. Writes to [talos.image].",
            Step::Orchestration => "How Zeus runs background work — heartbeat, cron, watchdog.",
            Step::Memory => {
                "Mnemosyne — semantic search over agent history. Pick embedding provider."
            }
            Step::Skills => "Pre-built capabilities for your agent. Toggle to install.",
            Step::Complete => {
                "Review your setup before launch. All settings persist to ~/.zeus/config.toml."
            }
        }
    }
}

/// App state.
/// Footer focus target. `None` = focus is on the screen's own fields (or the
/// screen has none); `Back`/`Next` = the global footer button is highlighted
/// and Enter/Space activates it. App-owned (per A2): the Tab counter — not any
/// screen's internal `focused_field` — drives this, so Tab-reachability of the
/// footer is consistent across every screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FooterFocus {
    Back,
    Next,
}

/// AWAKEN handoff dwell, in `anim_tick` units. At ~250ms/tick this is ~1.5s —
/// long enough for the "⚡ Launching Zeus…" frame to register as a real action
/// before the production UI takes over.
const LAUNCH_DWELL_TICKS: u64 = 6;

pub struct App {
    pub current_step: usize,
    /// Number of Tab presses since entering the current screen. Reset to 0 on
    /// every step-enter. Drives the App-owned footer focus cycle: the first
    /// `footer_field_count(step)` Tabs walk the screen's own fields, then the
    /// next two land on BACK → NEXT, then it wraps back to the screen.
    pub tab_cursor: usize,
    /// Which footer button (if any) is currently focused. Derived from
    /// `tab_cursor` on each Tab; the render reads it for the highlight.
    pub footer_focus: Option<FooterFocus>,
    mode_selected: usize,     // 0=quickstart, 1=full, 2=custom
    provider_selected: usize, // index into PROVIDERS
    instance_screen: InstanceScreen,
    model_screen: ModelScreen,
    pub fallback_screen: FallbackScreen,
    pub channels_screen: ChannelsScreen,
    pub chanconfig_screen: ChanConfigScreen,
    pub gateway_screen: GatewayScreen,
    pub agent_screen: AgentScreen,
    pub security_screen: SecurityScreen,
    pub features_screen: FeaturesScreen,
    voice_screen: VoiceScreen,
    images_screen: ImagesScreen,
    orchestration_screen: OrchestrationScreen,
    pub memory_screen: MemoryScreen,
    pub skills_screen: SkillsScreen,
    pub complete_screen: CompleteScreen,
    pub existing_config: bool,
    should_quit: bool,
    /// Animation clock — monotonically incremented once per ~250ms tick by the
    /// run loop (see `run_loop`). Drives time-based animation without coupling
    /// render to keypresses: blinking cursor phase = `anim_tick % 2`, and any
    /// frame-cycling widget indexes its frame list by `anim_tick % len`.
    /// Renderers read it; only the loop's `tick()` writes it.
    pub anim_tick: u64,
    // Auth screen state
    auth_mode: usize, // 0=key, 1=token, 2=browser
    auth_api_key: String,
    auth_test_status: Option<&'static str>,
    // Workspace screen state
    pub workspace_path: String,
    pub sessions_path: String,
    pub mnemosyne_path: String,
    workspace_existing_detected: bool,
    workspace_memory_facts: usize,
    workspace_session_count: usize,
    pub workspace_focused_field: usize,
    // Production UI state
    pub onboarding_complete: bool,
    /// AWAKEN handoff: while true, the run loop renders the "⚡ Launching
    /// Zeus…" frame instead of jumping silently into the production UI. The
    /// tick clock flips this → `onboarding_complete` after a short visible
    /// dwell so pressing AWAKEN clearly DOES something (was an instant,
    /// invisible in-place swap that read as "nothing happened").
    pub launching: bool,
    /// `anim_tick` value captured when AWAKEN fired — the dwell is measured
    /// against this so the handoff is tick-driven, not wall-clock-blocking.
    launch_started_tick: u64,
    /// AWAKEN-B seam: the fn invoked at the dwell-flip to bring the gateway
    /// live before the prod UI renders. Defaults to the real detached spawn
    /// (`awaken::spawn_gateway_detached`); tests swap it for a no-launch probe
    /// so the flip→spawn wiring is asserted without forking a real process.
    awaken_spawn: fn(),
    /// Latch so the dwell-flip fires `awaken_spawn` exactly once (the flip
    /// condition is edge-triggered by `launching`, but this guards against any
    /// future re-entry resetting `launching` and double-firing).
    awaken_fired: bool,
    prod_active_tab: usize,         // index into PRIMARY_TABS
    prod_active_adv: Option<usize>, // grid cursor in advanced overlay, None if overlay not open
    prod_adv_detail: Option<usize>, // drilled-into subview (index into ADVANCED_TABS), None = grid
    prod_chat_messages: Vec<ChatMessage>,
    /// Index of the assistant draft currently receiving stream tokens.
    ///
    /// Users can queue another message while a response is still streaming; in
    /// that case the last chat row becomes `Role::User`, but late tokens still
    /// belong to the earlier assistant draft. Track the draft explicitly so
    /// markdown blocks (especially tables) stay contiguous instead of being
    /// split by the queued user echo.
    prod_stream_assistant_idx: Option<usize>,
    pub prod_settings_section: prod::SettingsSection,
    /// Live config from `GET /v1/config` (sanitized). `None` until the config
    /// poll-worker lands the first fetch; the Settings tab overlays these live
    /// values onto its static section schema, falling back to the const
    /// placeholder when a key is absent.
    pub prod_config_rows: Option<serde_json::Value>,
    /// Live workspace memory files (`GET /v1/memory/files`). `None` until the
    /// memory poll-worker lands the first fetch; the Memory→Workspace sub-tab
    /// falls back to the const file tree while absent.
    pub prod_memory_files: Option<Vec<crate::api::MemoryFileEntry>>,
    /// Live session summaries (`GET /v1/sessions`). `None` until first fetch;
    /// the Memory→Sessions sub-tab falls back to const rows while absent.
    pub prod_sessions: Option<Vec<crate::api::SessionSummary>>,
    /// Live Mnemosyne search hits (`POST /v1/memory/search`). `None` until the
    /// initial query lands; the Memory→Mnemosyne sub-tab falls back to const
    /// results while absent.
    pub prod_memory_search: Option<Vec<crate::api::MemorySearchHit>>,
    /// Live installed skills (`GET /v1/skills`). `None` until first fetch; the
    /// Advanced→Skills subview falls back to the const list while absent.
    pub prod_agents: Option<Vec<crate::api::AgentResponse>>,
    pub prod_skills: Option<Vec<crate::api::SkillResponse>>,
    pub prod_mcp: Option<Vec<crate::api::McpServerResponse>>,
    pub prod_tts_providers: Option<Vec<crate::api::TtsProviderResponse>>,
    pub prod_tts_voices: Option<Vec<crate::api::TtsVoiceResponse>>,
    pub prod_workflows: Option<Vec<crate::api::WorkflowResponse>>,
    pub prod_extensions: Option<Vec<crate::api::ExtensionResponse>>,
    pub prod_projects: Option<Vec<crate::api::ProjectResponse>>,
    pub prod_nodes: Option<Vec<crate::api::NodeResponse>>,
    pub prod_spawns: Option<Vec<crate::api::SpawnResponse>>,
    pub prod_vector_stores: Option<Vec<crate::api::VectorStoreResponse>>,
    pub prod_communities: Option<Vec<crate::api::CommunityResponse>>,
    pub prod_deploy_targets: Option<Vec<crate::api::DeployTargetResponse>>,
    pub prod_deploy_history: Option<Vec<crate::api::DeploymentResponse>>,
    pub prod_deploy_stats: Option<crate::api::DeployStatsResponse>,
    pub prod_economy_wallets: Option<Vec<crate::api::EconomyWalletResponse>>,
    pub prod_economy_txs: Option<Vec<crate::api::EconomyTxResponse>>,
    /// Live pending approvals from `GET /v1/approvals` (#235 Approvals tab).
    pub prod_approvals: Option<Vec<crate::api::ApprovalResponse>>,
    /// Live Pantheon missions from `GET /v1/pantheon/missions` (#235 Pantheon tab).
    pub prod_pantheon_missions: Option<Vec<crate::api::PantheonMissionResponse>>,
    /// Live gateway status from `GET /v1/status` (#235 TopBar de-mock).
    /// Polled every 5s; carries sessions_count, tools, model, uptime, version.
    pub prod_status: Option<crate::api::StatusResponse>,
    /// Live active agent tasks from `GET /v1/tasks/active` (#280). Backs the
    /// Claude-Code-style task-tracker panel in the chat tab; None until the
    /// first poll lands, empty Vec → no active tasks → panel hidden.
    pub prod_active_tasks: Option<Vec<crate::api::TaskResponse>>,
    prod_chat_input: String,
    prod_chat_scroll: usize,
    prod_stream_state: StreamState,
    /// Live tool-usage feed items during a streaming cook.
    prod_tool_feed: Vec<crate::prod::chat_tab::ToolFeedItem>,
    /// Current cook iteration count (from `iter` SSE events).
    prod_iter_count: u32,
    prod_slash_open: bool,
    prod_queue_count: usize,
    prod_tools_selected_category: Option<String>,
    prod_tools_selected_tool: String,
    prod_office_focused: Option<u8>, // Office tab — focused agent index (None = NO FOCUS)
    prod_office_show_memo: bool,
    prod_office_show_help: bool,
    prod_pantheon_selected: usize, // Pantheon tab — selected mission index
    prod_wallet_view: prod::WalletView, // Wallet tab — active sub-view (1–6)
    prod_wallet_titan_sel: usize,  // Wallet tab — selected fleet titan index
    prod_tools_filter: String,
    prod_tools_scroll: usize,
    // --- Gateway integration (Phase 2): live connection + identity ---
    pub gateway_host: String,
    pub gateway_port: u16,
    pub conn_state: ConnState,
    pub agent_name: String,
    /// Sync→async bridge: chat submits from the UI go here; an async worker
    /// spawned by `run()` calls the gateway and appends the reply. `None` in
    /// standalone mode (no gateway).
    pub chat_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Sync→async bridge for the live `/v1/models` fetch (#239/#240). Pressing
    /// advance on Auth sends `(provider_id, api_key)` here; the worker spawned
    /// by `run()` calls `model_fetch::fetch_models` (8–10s timeout) and writes
    /// the result into `model_fetch_state`. `None` in standalone mode (no
    /// runtime to spawn into → static-list fallback).
    pub fetch_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, String)>>,
    /// State machine for the live model fetch. `Idle` until the user advances
    /// past Auth; `Fetching` drives the spinner; `Done(list)` is consumed by
    /// the Model page (P3); `Failed(err)` blocks advance + shows the error.
    pub model_fetch_state: crate::model_fetch::ModelFetchState,
    /// Live channel adapters from `/v1/channels`, polled from the gateway by
    /// run(). None in standalone mode or before the first successful fetch.
    pub prod_channels: Option<Vec<crate::api::ChannelResponse>>,
    /// Live tool registry from the gateway (leaked to 'static, fetched once by
    /// run()). None in standalone mode → tools_tab falls back to its seed.
    pub live_tools: Option<&'static [crate::prod::tools_tab::ToolEntry]>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Construct the app in its default (fresh-onboarding) state.
    ///
    /// 🔴 **PURE — NO DISK I/O.** This constructor must never read
    /// `~/.zeus/config.toml`. The startup disk-read that decides whether to skip
    /// the wizard lives in [`App::new_from_disk`], which the production entry
    /// point ([`crate::run`]) calls. Keeping `new()` disk-free is load-bearing:
    /// the ~25 integration tests construct `App::new()` directly and run in
    /// parallel — if `new()` read the *real* `~/.zeus` it would contend/hang the
    /// full suite (this is the Commit-2 hang incident). Tests use `new()`;
    /// production uses `new_from_disk()`.
    pub fn new() -> Self {
        let existing_config = false;
        let onboarding_complete = false;
        Self {
            current_step: 0,
            tab_cursor: 0,
            footer_focus: None,
            mode_selected: 0,
            provider_selected: 0,
            instance_screen: InstanceScreen::new(),
            model_screen: ModelScreen::new("anthropic".to_string()),
            fallback_screen: FallbackScreen::new("anthropic".to_string()),
            channels_screen: ChannelsScreen::new(),
            chanconfig_screen: ChanConfigScreen::new(),
            gateway_screen: GatewayScreen::new(),
            agent_screen: AgentScreen::new(),
            security_screen: SecurityScreen::new(),
            features_screen: FeaturesScreen::default(),
            voice_screen: VoiceScreen::new(),
            images_screen: ImagesScreen::new(),
            orchestration_screen: OrchestrationScreen::new(),
            memory_screen: MemoryScreen::new(),
            skills_screen: SkillsScreen::new(),
            complete_screen: CompleteScreen::new(),
            existing_config,
            should_quit: false,
            anim_tick: 0,
            auth_mode: 0,
            auth_api_key: String::new(),
            auth_test_status: None,
            workspace_path: "~/.zeus/workspace".to_string(),
            sessions_path: "~/.zeus/sessions".to_string(),
            mnemosyne_path: "~/.zeus/mnemosyne.db".to_string(),
            workspace_existing_detected: false,
            workspace_memory_facts: 0,
            workspace_session_count: 0,
            workspace_focused_field: 0,
            // Production UI state
            onboarding_complete,
            launching: false,
            launch_started_tick: 0,
            awaken_spawn: crate::awaken::spawn_gateway_detached,
            awaken_fired: false,
            prod_active_tab: 0,
            prod_active_adv: None,
            prod_adv_detail: None,
            prod_settings_section: prod::SettingsSection::Llm,
            // Chat initializes empty — no seeded demo conversation (#266).
            // Real messages are appended live as the user chats (see `submit`
            // / stream handlers below). A fresh session shows the empty-state
            // composer placeholder rendered by `chat_tab`.
            prod_chat_messages: Vec::new(),
            prod_stream_assistant_idx: None,
            prod_config_rows: None,
            prod_memory_files: None,
            prod_sessions: None,
            prod_memory_search: None,
            prod_agents: None,
            prod_skills: None,
            prod_mcp: None,
            prod_tts_providers: None,
            prod_tts_voices: None,
            prod_workflows: None,
            prod_extensions: None,
            prod_projects: None,
            prod_nodes: None,
            prod_spawns: None,
            prod_vector_stores: None,
            prod_communities: None,
            prod_deploy_targets: None,
            prod_deploy_history: None,
            prod_deploy_stats: None,
            prod_economy_wallets: None,
            prod_economy_txs: None,
            prod_approvals: None,
            prod_pantheon_missions: None,
            prod_status: None,
            prod_active_tasks: None,
            prod_chat_input: String::new(),
            prod_chat_scroll: 0,
            prod_stream_state: StreamState::Idle,
            prod_tool_feed: Vec::new(),
            prod_iter_count: 0,
            prod_slash_open: false,
            prod_queue_count: 0,
            prod_tools_selected_category: None,
            prod_tools_selected_tool: "shell".to_string(),
            prod_tools_filter: String::new(),
            prod_tools_scroll: 0,
            prod_office_focused: None,
            prod_office_show_memo: false,
            prod_office_show_help: false,
            prod_pantheon_selected: 0,
            prod_wallet_view: prod::WalletView::Balance,
            prod_wallet_titan_sel: 0,
            // Gateway integration defaults — overridden by run(config).
            gateway_host: "localhost".to_string(),
            gateway_port: 8080,
            conn_state: ConnState::Disconnected,
            agent_name: "zeus".to_string(),
            chat_tx: None,
            fetch_tx: None,
            model_fetch_state: crate::model_fetch::ModelFetchState::default(),
            prod_channels: None,
            live_tools: None,
        }
    }

    /// Construct the app, reading disk state at startup to decide whether the
    /// onboarding wizard should run.
    ///
    /// This is the **production** constructor (the READ half that closes the
    /// re-onboard-every-launch loop: Commit 1 writes the `onboarding_complete`
    /// marker; this reads it). A completed/legacy install skips the wizard and
    /// drops straight into the production UI; a fresh install (or a failed load)
    /// falls back to onboarding (safe default).
    ///
    /// Kept separate from [`App::new`] so the pure constructor stays disk-free
    /// for the parallel test suite (see `new()`'s note).
    pub fn new_from_disk() -> Self {
        // `Config::load()` returns defaults (loaded_from_default = true) when no
        // config.toml exists, so presence-of-Ok is NOT proof of a real config.
        // A real config on disk = loaded AND not the default fallback.
        let loaded = zeus_core::Config::load().ok();
        let existing_config = loaded
            .as_ref()
            .map(|cfg| !cfg.loaded_from_default)
            .unwrap_or(false);
        let onboarding_complete = loaded
            .as_ref()
            .map(|cfg| !cfg.needs_onboarding())
            .unwrap_or(false);
        let mut app = App::new();
        if let Some(cfg) = loaded.as_ref().filter(|cfg| !cfg.loaded_from_default) {
            app.hydrate_from_config(cfg);
        }
        app.existing_config = existing_config;
        app.onboarding_complete = onboarding_complete;
        app
    }

    /// Pre-populate wizard-owned state from an existing config. This makes an
    /// interactive re-run of onboarding on a configured box a no-op unless the
    /// user actually changes fields, instead of letting fresh wizard defaults
    /// overwrite top-level config values on AWAKEN.
    fn hydrate_from_config(&mut self, cfg: &zeus_core::Config) {
        let (provider, model) = cfg.parse_model();
        let canonical_provider = provider.name();
        let provider_id = match canonical_provider {
            "google-gemini-cli" => "gemini-cli",
            "moonshot" => "kimi",
            "zai" => "glm",
            "xiaomimimo" => "mimo",
            other => other,
        };
        if let Some(idx) = crate::screens::providers::PROVIDERS
            .iter()
            .position(|p| p.id == provider_id)
        {
            self.provider_selected = idx;
            self.model_screen.set_provider(provider_id);
            self.fallback_screen.set_primary(provider_id);
        }
        self.model_screen.select_model_id(&model);

        if let Some(fallbacks) = cfg.fallback_models.as_ref() {
            self.fallback_screen.chain = fallbacks.clone();
        }

        let agent = cfg.agent.as_ref();
        let persona = agent
            .and_then(|a| a.persona.as_deref())
            .or(cfg.persona.as_deref());
        if let Some(persona) = persona {
            self.agent_screen.select_persona_name(persona);
        }
        if let Some(name) = agent
            .and_then(|a| a.name.as_deref())
            .or(cfg.name.as_deref())
            .filter(|name| !name.trim().is_empty())
        {
            self.agent_screen.name = name.to_string();
            self.agent_name = name.to_string();
        }
        if let Some(role) = agent.and_then(|a| a.role.as_deref()) {
            self.agent_screen.role = role.to_string();
        }

        self.workspace_path = cfg.workspace.display().to_string();
        self.sessions_path = cfg.sessions.display().to_string();

        if let Some(gateway) = cfg.gateway.as_ref() {
            self.gateway_screen.host = gateway.host.clone();
            self.gateway_screen.port = gateway.port.to_string();
            self.gateway_host = gateway.host.clone();
            self.gateway_port = gateway.port;
        }

        if !cfg.enabled_skills.is_empty() {
            self.skills_screen.installed = cfg.enabled_skills.clone();
        }
    }

    /// Append a user message synchronously before dispatching to the async
    /// gateway worker. This is the send-feedback invariant: pressing Enter must
    /// immediately paint the user's message plus a visible pending indicator so
    /// quick consecutive sends never look dropped.
    pub fn push_user_send_feedback(&mut self, text: String) {
        self.prod_chat_messages.push(ChatMessage {
            role: Role::User,
            text,
            tool_name: None,
        });
        self.prod_chat_scroll = 0;
        self.prod_stream_state = StreamState::Queued;
        self.prod_tool_feed.clear();
        self.prod_iter_count = 0;
    }

    /// Append an assistant reply from the gateway and clear the streaming state.
    /// Called by the async chat worker (`run()`) under the app lock.
    pub fn push_assistant_reply(&mut self, text: String) {
        if !text.trim().is_empty() {
            let mut replaced_stream_draft = false;
            if let Some(idx) = self.prod_stream_assistant_idx
                && let Some(msg) = self.prod_chat_messages.get_mut(idx)
                && msg.role == Role::Assistant
                && msg.tool_name.is_none()
            {
                msg.text = text.clone();
                replaced_stream_draft = true;
            }

            if !replaced_stream_draft {
                match self.prod_chat_messages.last_mut() {
                    // If live token deltas already painted this assistant turn,
                    // replace/confirm the draft instead of appending a duplicate.
                    Some(msg) if msg.role == Role::Assistant && msg.tool_name.is_none() => {
                        msg.text = text;
                    }
                    _ => self.prod_chat_messages.push(ChatMessage {
                        role: Role::Assistant,
                        text,
                        tool_name: None,
                    }),
                }
            }
        }
        self.finish_chat_stream();
    }

    /// Paint a streamed assistant token immediately so the chat never looks
    /// frozen while the gateway is still working. The final reply coalesces
    /// with this draft in `push_assistant_reply`.
    pub fn push_stream_token(&mut self, token: String) {
        if token.is_empty() {
            return;
        }
        self.prod_stream_state = StreamState::Streaming;

        if let Some(idx) = self.prod_stream_assistant_idx
            && let Some(msg) = self.prod_chat_messages.get_mut(idx)
            && msg.role == Role::Assistant
            && msg.tool_name.is_none()
        {
            msg.text.push_str(&token);
            self.prod_chat_scroll = 0;
            return;
        }

        let idx = self.prod_chat_messages.len();
        self.prod_chat_messages.push(ChatMessage {
            role: Role::Assistant,
            text: token,
            tool_name: None,
        });
        self.prod_stream_assistant_idx = Some(idx);
        self.prod_chat_scroll = 0;
    }

    /// Record that the stream is alive even if the provider is currently
    /// emitting thinking-only deltas. This keeps the composer/face in working
    /// state without appending a blank assistant row.
    pub fn push_stream_thinking(&mut self, _text: String) {
        self.prod_stream_state = StreamState::Streaming;
        self.prod_chat_scroll = 0;
    }

    fn finish_chat_stream(&mut self) {
        self.prod_stream_state = StreamState::Idle;
        // Clear the live tool feed — cook is done.
        self.prod_tool_feed.clear();
        self.prod_iter_count = 0;
        self.prod_stream_assistant_idx = None;
        // Jump-to-bottom on new content: resume follow so the freshly-arrived
        // reply is visible. A user who had scrolled up to read history gets
        // pulled back to the latest message (matches Discord/Slack behaviour).
        self.prod_chat_scroll = 0;
    }

    /// Record a tool-start event from the streaming cook.
    pub fn push_tool_start(&mut self, name: String, input: String) {
        use crate::prod::chat_tab::ToolFeedItem;
        let summary: String = input.chars().take(60).collect();
        self.prod_stream_state = StreamState::Streaming;
        self.prod_tool_feed.push(ToolFeedItem {
            name,
            input_summary: summary,
            done: false,
            output_summary: String::new(),
        });
        self.prod_chat_scroll = 0;
    }

    /// Record a tool-end event from the streaming cook.
    pub fn push_tool_end(&mut self, name: String, output: String) {
        let summary: String = output.chars().take(60).collect();
        self.prod_stream_state = StreamState::Streaming;
        // Find the last matching tool that isn't done yet.
        if let Some(item) = self
            .prod_tool_feed
            .iter_mut()
            .rev()
            .find(|t| t.name == name && !t.done)
        {
            item.done = true;
            item.output_summary = summary;
        }
        self.prod_chat_scroll = 0;
    }

    /// Record an iteration boundary from the streaming cook.
    pub fn push_iter(&mut self, n: u32) {
        self.prod_stream_state = StreamState::Streaming;
        self.prod_iter_count = n;
        self.prod_chat_scroll = 0;
    }

    /// Upper bound for the line-granular chat scroll counter. The chat widget
    /// bottom-anchors by *line* (not message), so the true max depends on
    /// wrapped/markdown line counts the widget computes at render time — and it
    /// re-clamps `scroll_offset` every frame. This is a cheap, generous ceiling
    /// (sum of per-message line estimates) that simply stops the key handler's
    /// counter from running away above any plausible content height.
    fn chat_scroll_max(&self) -> usize {
        // ~1 prefix line + 1 blank + an estimate of body lines per message.
        // Over-estimating is safe (widget clamps down); under-estimating would
        // make the top of history unreachable, so we err high.
        self.prod_chat_messages
            .iter()
            .map(|m| 2 + m.text.lines().count().max(1) + m.text.len() / 40)
            .sum()
    }

    fn handle_mouse_prod(&mut self, kind: MouseEventKind) {
        if !self.onboarding_complete
            || self.prod_active_tab != 0
            || self.prod_active_adv.is_some()
            || self.prod_adv_detail.is_some()
        {
            return;
        }

        match kind {
            MouseEventKind::ScrollUp => {
                self.prod_chat_scroll = (self.prod_chat_scroll + 3).min(self.chat_scroll_max());
            }
            MouseEventKind::ScrollDown => {
                self.prod_chat_scroll = self.prod_chat_scroll.saturating_sub(3);
            }
            _ => {}
        }
    }

    /// Production UI key handling — Tab/Shift-Tab cycles tabs, Enter selects advanced.
    fn handle_key_prod(&mut self, key: KeyCode) {
        match key {
            // Bare-letter shortcuts (q/c/f) MUST NOT fire while the chat input
            // is focused (chat tab 0, no advanced overlay) — otherwise typing
            // these letters quits the app or is swallowed instead of inserting
            // into `prod_chat_input` (the `discord`→`disord` bug). The guard is
            // the negation of the chat-focus predicate used by the insert arm
            // below (`prod_active_tab == 0 && prod_active_adv.is_none()`); when
            // it fails, the arm doesn't match and the char falls through to the
            // chat-input catch-all.
            KeyCode::Char('q') if self.prod_active_tab != 0 || self.prod_active_adv.is_some() => {
                self.should_quit = true
            }
            KeyCode::Char('c') if self.prod_active_tab != 0 || self.prod_active_adv.is_some() => {
                // Ctrl+C is handled by crossterm as Char('c') with ctrl modifier
                // For now, just 'q' quits
            }
            KeyCode::Tab => {
                let tab_id = PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id);
                if tab_id == Some("office") && self.prod_active_adv.is_none() {
                    self.prod_office_focused = None;
                    self.prod_active_tab = (self.prod_active_tab + 1) % PRIMARY_TABS.len();
                } else if self.prod_active_adv.is_some() {
                    // Cycle within advanced tabs
                    let adv = self.prod_active_adv.unwrap_or(0);
                    self.prod_active_adv = Some((adv + 1) % ADVANCED_TABS.len());
                } else {
                    // Cycle primary tabs
                    self.prod_active_tab = (self.prod_active_tab + 1) % PRIMARY_TABS.len();
                }
            }
            KeyCode::BackTab => {
                if self.prod_active_adv.is_some() {
                    let adv = self.prod_active_adv.unwrap_or(0);
                    self.prod_active_adv =
                        Some((adv + ADVANCED_TABS.len() - 1) % ADVANCED_TABS.len());
                } else {
                    self.prod_active_tab =
                        (self.prod_active_tab + PRIMARY_TABS.len() - 1) % PRIMARY_TABS.len();
                }
            }
            KeyCode::Enter => {
                let tab_id = PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id);
                if tab_id == Some("advanced") && self.prod_active_adv.is_none() {
                    // Open advanced overlay (grid)
                    self.prod_active_adv = Some(0);
                } else if self.prod_adv_detail.is_some() {
                    // Already in a subview — Enter is a no-op here
                } else if let Some(idx) = self.prod_active_adv {
                    // Select advanced tab → drill into its subview
                    self.prod_adv_detail = Some(idx);
                } else if tab_id == Some("chat") {
                    // Submit chat message → gateway (async) when connected.
                    let msg = self.prod_chat_input.trim().to_string();
                    if !msg.is_empty() {
                        self.prod_chat_input.clear();
                        self.push_user_send_feedback(msg.clone());
                        if let Some(tx) = &self.chat_tx {
                            // Dispatch to the async chat worker; the reply is
                            // appended when it returns. UI already shows the
                            // optimistic user echo + queued/sending indicator.
                            let _ = tx.send(msg);
                        } else {
                            // Standalone (no gateway wired) — no live backend.
                            self.prod_chat_messages.push(ChatMessage {
                                role: Role::Assistant,
                                text:
                                    "(not connected to a gateway — launch via `zeus` for live chat)"
                                        .to_string(),
                                tool_name: None,
                            });
                            self.prod_stream_state = StreamState::Idle;
                        }
                    }
                }
            }
            KeyCode::Esc => {
                let tab_id = PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id);
                if self.prod_adv_detail.is_some() {
                    // In a subview → back to the grid
                    self.prod_adv_detail = None;
                } else if self.prod_active_adv.is_some() {
                    // In the grid → close the overlay
                    self.prod_active_adv = None;
                } else if self.prod_slash_open {
                    self.prod_slash_open = false;
                } else if self.prod_active_tab == 0 && self.prod_chat_scroll > 0 {
                    // Chat history scrolled up → jump back to the live bottom.
                    self.prod_chat_scroll = 0;
                } else if tab_id == Some("office") {
                    // Office P3: Esc closes memo/help overlays and clears focus.
                    self.prod_office_show_memo = false;
                    self.prod_office_show_help = false;
                    self.prod_office_focused = None;
                }
            }
            KeyCode::Char('f') if self.prod_active_tab != 0 || self.prod_active_adv.is_some() => {
                // Back-compat focus shortcut; Tab is the prototype key for Office focus.
                let tab_id = PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id);
                if tab_id == Some("office") && self.prod_active_adv.is_none() {
                    self.prod_office_focused = prod::cycle_focused(self.prod_office_focused);
                }
            }
            KeyCode::Char('m') | KeyCode::Char('M')
                if PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id) == Some("office")
                    && self.prod_active_adv.is_none() =>
            {
                self.prod_office_show_memo = !self.prod_office_show_memo;
                if self.prod_office_show_memo {
                    self.prod_office_show_help = false;
                }
            }
            KeyCode::Char('?')
                if PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id) == Some("office")
                    && self.prod_active_adv.is_none() =>
            {
                self.prod_office_show_help = !self.prod_office_show_help;
                if self.prod_office_show_help {
                    self.prod_office_show_memo = false;
                }
            }
            KeyCode::Char('/') => {
                if self.prod_active_tab == 0 && self.prod_chat_input.is_empty() {
                    // Open slash command overlay
                    self.prod_slash_open = true;
                } else {
                    self.prod_chat_input.push('/');
                }
            }
            // Wallet tab — sub-view switch (1–6) + titan nav (j/k).
            KeyCode::Char(c)
                if PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id) == Some("wallet") =>
            {
                match c {
                    '1'..='6' => {
                        let n = (c as u8 - b'0') as usize;
                        if let Some(v) = prod::WalletView::from_key(n) {
                            self.prod_wallet_view = v;
                        }
                    }
                    'j' => {
                        // #274: nav over LIVE fleet wallets (empty → no-op).
                        let count = self
                            .prod_economy_wallets
                            .as_ref()
                            .map(|w| w.len())
                            .unwrap_or(0);
                        if count > 0 {
                            self.prod_wallet_titan_sel = (self.prod_wallet_titan_sel + 1) % count;
                        }
                    }
                    'k' => {
                        let count = self
                            .prod_economy_wallets
                            .as_ref()
                            .map(|w| w.len())
                            .unwrap_or(0);
                        if count > 0 {
                            self.prod_wallet_titan_sel =
                                (self.prod_wallet_titan_sel + count - 1) % count;
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Char(c) if self.prod_active_tab == 0 && self.prod_active_adv.is_none() => {
                // Chat input
                self.prod_chat_input.push(c);
            }
            KeyCode::Backspace if self.prod_active_tab == 0 => {
                self.prod_chat_input.pop();
            }
            KeyCode::PageUp => {
                if self.prod_active_tab == 0
                    && self.prod_active_adv.is_none()
                    && self.prod_adv_detail.is_none()
                {
                    self.prod_chat_scroll =
                        (self.prod_chat_scroll + 10).min(self.chat_scroll_max());
                }
            }
            KeyCode::PageDown => {
                if self.prod_active_tab == 0
                    && self.prod_active_adv.is_none()
                    && self.prod_adv_detail.is_none()
                {
                    self.prod_chat_scroll = self.prod_chat_scroll.saturating_sub(10);
                }
            }
            KeyCode::Up => {
                if self.prod_adv_detail.is_some() {
                    // no-op inside a subview (subviews own their own scroll later)
                } else if let Some(idx) = self.prod_active_adv {
                    // Grid: move up one row (3 columns)
                    if idx >= 3 {
                        self.prod_active_adv = Some(idx - 3);
                    }
                } else if PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id) == Some("settings") {
                    self.prod_settings_section = self.prod_settings_section.prev();
                } else if self.prod_active_tab == 0
                    && self.prod_chat_scroll < self.chat_scroll_max()
                {
                    // Scroll up one LINE (chat scroll is line-granular now —
                    // the widget bottom-anchors by line). The widget re-clamps
                    // to the true max each frame; this bound just keeps the
                    // counter from running away on an over-tall estimate.
                    self.prod_chat_scroll += 1;
                }
            }
            KeyCode::Down => {
                if self.prod_adv_detail.is_some() {
                    // no-op inside a subview
                } else if let Some(idx) = self.prod_active_adv {
                    // Grid: move down one row (3 columns)
                    if idx + 3 < ADVANCED_TABS.len() {
                        self.prod_active_adv = Some(idx + 3);
                    }
                } else if PRIMARY_TABS.get(self.prod_active_tab).map(|t| t.id) == Some("settings") {
                    self.prod_settings_section = self.prod_settings_section.next();
                } else if self.prod_active_tab == 0 && self.prod_chat_scroll > 0 {
                    self.prod_chat_scroll -= 1;
                }
            }
            KeyCode::Left => {
                if self.prod_adv_detail.is_some() {
                    // ← Advanced: back out of the subview to the grid
                    self.prod_adv_detail = None;
                } else if let Some(idx) = self.prod_active_adv {
                    // Grid: move left one column (column 0 stays)
                    if idx % 3 != 0 {
                        self.prod_active_adv = Some(idx - 1);
                    }
                } else {
                    self.prod_active_tab =
                        (self.prod_active_tab + PRIMARY_TABS.len() - 1) % PRIMARY_TABS.len();
                }
            }
            KeyCode::Right => {
                if self.prod_adv_detail.is_none() {
                    if let Some(idx) = self.prod_active_adv {
                        // Grid: move right one column
                        if idx % 3 != 2 && idx + 1 < ADVANCED_TABS.len() {
                            self.prod_active_adv = Some(idx + 1);
                        }
                    } else {
                        self.prod_active_tab = (self.prod_active_tab + 1) % PRIMARY_TABS.len();
                    }
                }
            }
            _ => {}
        }
    }

    /// Sync state that a step needs when it becomes active.
    pub fn on_step_enter(&mut self) {
        // Reset the App-owned footer Tab cursor on every screen transition so
        // Tab-reachability of BACK/NEXT is consistent regardless of where the
        // screen's internal field focus (e.g. ↑/↓-driven) happens to be.
        self.tab_cursor = 0;
        self.footer_focus = None;
        if Step::from_index(self.current_step) == Some(Step::Model) {
            // Sync the model catalog to the provider picked on the Provider
            // screen. Without this, the catalog stays frozen at the
            // construction-time provider ("anthropic") and a user who picks
            // e.g. openai sees Anthropic models + a mismatched summary row.
            let picked = screens::provider::provider_id_at(self.provider_selected);
            self.model_screen.set_provider(picked);
            // Populate from the LIVE `/v1/models` list when the P2 fetch
            // succeeded (the advance-gate only lets us reach Model on `Done`).
            // Any other state → clear to the static per-provider catalog
            // (standalone mode, or proceed-after-failure fallback).
            match &self.model_fetch_state {
                crate::model_fetch::ModelFetchState::Done(names) if !names.is_empty() => {
                    self.model_screen.set_live_models(names.clone());
                }
                _ => self.model_screen.clear_live_models(),
            }
        }
        if Step::from_index(self.current_step) == Some(Step::Fallback) {
            // Sync the fallback candidate list to the picked primary. Without
            // this, FallbackScreen stays frozen at its construction-time
            // primary ("anthropic") and a user who picks e.g. glm sees glm
            // offered as its own fallback + anthropic wrongly excluded.
            let picked = screens::provider::provider_id_at(self.provider_selected);
            self.fallback_screen.set_primary(picked);
        }
        if Step::from_index(self.current_step) == Some(Step::Complete) {
            self.complete_screen.summary = self.build_summary();
        }
        if Step::from_index(self.current_step) == Some(Step::ChannelConfig) {
            self.chanconfig_screen.toggled = self.channels_screen.selected_ids();
            // Reset focus onto the first focusable field of the new toggled set.
            self.chanconfig_screen.field_cursor = 0;
            self.chanconfig_screen.focus_prev();
            self.chanconfig_screen.focus_next();
        }
    }

    /// Collect every onboarding selection into a `zeus_core::Config` and persist
    /// it to disk — the completion path that turns the render-only wizard into a
    /// functional one. Called by BOTH completion sites (`advance_step` on the
    /// Complete screen + the Enter-on-Complete AWAKEN arm) BEFORE they flip
    /// `onboarding_complete`, so the config is on disk before the run loop
    /// transitions to the production UI.
    ///
    /// 🔴 SACRED (CLAUDE.md primary orders): config/.env are never clobbered.
    /// - config.toml: we start from `Config::load()` (preserves every existing
    ///   section — `[council]`, hand-edited `[gateway]` sub-keys, etc.), mutate
    ///   ONLY the onboarding-owned fields, then `save_unchecked()` (the onboarding
    ///   path; serializes the in-memory Config wholesale, so loading first is
    ///   load-bearing — building from `Config::default()` would wipe non-onboarding
    ///   sections).
    /// - .env: the provider API key lands via `persist_env_key()`, a MERGE writer
    ///   that updates/appends only the one `KEY=...` line and preserves every other
    ///   secret (`DISCORD_BOT_TOKEN`, …) byte-for-byte.
    ///
    /// Returns the persisted `Config` on success (used by tests to assert the
    /// round-trip). Errors are surfaced to the caller, which records them on the
    /// Complete screen rather than silently swallowing them.
    fn collect_and_persist(&self) -> Result<zeus_core::Config, String> {
        // Start from the on-disk config so every non-onboarding section survives.
        let mut cfg = zeus_core::Config::load().map_err(|e| format!("load config: {e}"))?;

        // ── model = "{provider}/{model}" ──
        // Canonicalize the display id (e.g. "glm", "kimi") → the canonical
        // Provider id (e.g. "zai", "moonshot") so the persisted model string is
        // byte-identical to the proven-working old onboarding / spark config and
        // resolves identically at the gateway read-side. Generic for ALL providers.
        let display_id = screens::provider::provider_id_at(self.provider_selected);
        let provider_id = zeus_core::Provider::from_prefix(display_id).name();
        let model_id = self.model_screen.selected_model_id();
        cfg.model = format!("{provider_id}/{model_id}");

        // ── fallback LLMs ──
        if !self.fallback_screen.chain.is_empty() {
            cfg.fallback_models = Some(self.fallback_screen.chain.clone());
        }

        // ── persona + agent section ──
        let persona = self.agent_screen.persona_name().to_string();
        cfg.persona = Some(persona.clone());
        let agent = cfg.agent.get_or_insert_with(Default::default);
        let wizard_name = self.agent_screen.summary_name();
        let wizard_name_was_explicit = !self.agent_screen.name.trim().is_empty();
        let existing_name_empty = agent
            .name
            .as_deref()
            .map(|name| name.trim().is_empty())
            .unwrap_or(true);
        if wizard_name_was_explicit || existing_name_empty {
            agent.name = Some(wizard_name);
        }
        agent.persona = Some(persona);

        // ── workspace / sessions paths ──
        cfg.workspace = std::path::PathBuf::from(&self.workspace_path);
        cfg.sessions = std::path::PathBuf::from(&self.sessions_path);

        // ── gateway host/port ──
        // #290: `get_or_insert_with` so a FRESH onboarding (no `[gateway]`
        // section yet → `cfg.gateway == None`) still persists the chosen
        // host/port. The prior `if let Some(gw)` guard silently dropped both
        // values whenever the section was absent, so a custom port never
        // reached `config.toml` without hand-editing. When the section already
        // exists we mutate it in place, preserving every other sub-key.
        let gw = cfg.gateway.get_or_insert_with(Default::default);
        gw.host = self.gateway_screen.host.clone();
        if let Ok(port) = self.gateway_screen.port.parse::<u16>() {
            gw.port = port;
        }

        // ── skills ──
        if !self.skills_screen.installed.is_empty() {
            cfg.enabled_skills = self.skills_screen.installed.to_vec();
        }

        // ── voice (TTS) — the picker selection was previously DROPPED on persist
        //    (shown in the review summary, never written to config.toml). Wire the
        //    selected provider into cfg.voice; "none" disables TTS without nuking
        //    a pre-existing voice section's provider-specific extras. ──
        let voice_id = self.voice_screen.selected_id();
        {
            let voice = cfg.voice.get_or_insert_with(Default::default);
            if voice_id == "none" {
                voice.enabled = false;
            } else {
                voice.provider = Some(voice_id.to_string());
                voice.enabled = true;
            }
        }

        // ── image generator — map the picker id → ImageGenProviderType enum
        //    (provider is a typed enum here, NOT a free string like voice). Skip
        //    on "none" to leave any existing image_gen section untouched. ──
        let image_id = self.images_screen.selected_id();
        if image_id != "none" {
            use zeus_core::ImageGenProviderType;
            let provider = match image_id {
                "openai" => Some(ImageGenProviderType::OpenAi),
                "comfyui" => Some(ImageGenProviderType::ComfyUi),
                "fooocus" => Some(ImageGenProviderType::Fooocus),
                "a1111" => Some(ImageGenProviderType::Automatic1111),
                "openai-custom" => Some(ImageGenProviderType::OpenAiCompatible),
                _ => None,
            };
            if let Some(p) = provider {
                cfg.image_gen.get_or_insert_with(Default::default).provider = p;
            }
        }

        // ── memory (embeddings) — the picker id selects the embedding provider.
        //    Map ollama/openai → EmbeddingProvider + enable embeddings; "none"
        //    leaves the mnemosyne section's embedding config untouched. ──
        let memory_id = self.memory_screen.selected_id();
        if memory_id != "none" {
            use zeus_core::EmbeddingProvider;
            let provider = match memory_id {
                "ollama" => Some(EmbeddingProvider::Ollama),
                "openai" => Some(EmbeddingProvider::OpenAI),
                _ => None,
            };
            if let Some(p) = provider {
                let mn = cfg.mnemosyne.get_or_insert_with(Default::default);
                mn.enable_embeddings = true;
                mn.embedding_providers = vec![p];
            }
        }

        // ── channels (relay creds) — for each TOGGLED channel, build its
        //    per-channel config from the collected field values and write it
        //    into cfg.channels.<channel>. Parent ChannelsConfig derives Default
        //    (get_or_insert_with), but the per-channel structs do NOT — so each
        //    is constructed in full (required fields populated, optionals None).
        //    Only channels onboarding collects struct-matching creds for persist;
        //    the rest are deferred (#247 follow-ups), see comment below. ──
        {
            use std::collections::HashMap;
            use zeus_core::{
                DiscordChannelConfig, IrcChannelConfig, MatrixChannelConfig, SlackChannelConfig,
                TelegramChannelConfig, XTwitterChannelConfig,
            };

            let cv = &self.chanconfig_screen.config_values;
            let toggled = &self.chanconfig_screen.toggled;
            let val = |k: &str| cv.get(k).cloned().unwrap_or_default();
            // Per-channel bot-message policy selected in the chanconfig step.
            // Persisted verbatim as `allow_bots = "<choice>"` (default "mentions").
            let bot_policy =
                |ch_id: &str| Some(self.chanconfig_screen.bot_policy(ch_id).to_string());

            if toggled.iter().any(|c| c == "telegram") {
                // api_id is i32 in the struct (NOT i64) — parse, default 0 on empty/garbage.
                let api_id = val("telegram.api_id").trim().parse::<i32>().unwrap_or(0);
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.telegram = Some(TelegramChannelConfig {
                    api_id,
                    api_hash: val("telegram.api_hash"),
                    phone: val("telegram.phone"),
                    bot_token: None,
                    session_file: None,
                    policy: None,
                    accounts: HashMap::new(),
                    allow_bots: bot_policy("telegram"),
                });
            }

            if toggled.iter().any(|c| c == "discord") {
                // NOTE: onboarding also collects discord.server_id / discord.channel_id,
                // but DiscordChannelConfig has no struct home for them — they are
                // relay-channel targeting that belongs in ~/.zeus/.env
                // (DISCORD_RELAY_CHANNEL_IDS); deferred as a #247 env-wiring follow-up.
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.discord = Some(DiscordChannelConfig {
                    token: val("discord.token"),
                    application_id: None,
                    policy: None,
                    voice: None,
                    accounts: HashMap::new(),
                    allow_bots: bot_policy("discord"),
                });
            }

            if toggled.iter().any(|c| c == "slack") {
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.slack = Some(SlackChannelConfig {
                    bot_token: val("slack.bot_token"),
                    app_token: val("slack.app_token"),
                    policy: None,
                    accounts: HashMap::new(),
                    allow_bots: bot_policy("slack"),
                });
            }

            if toggled.iter().any(|c| c == "irc") {
                // Onboarding collects irc.{server,port,nick,channels,password}.
                // All map cleanly onto IrcChannelConfig: the comma-separated
                // `channels` field splits into the Vec<String> the struct wants;
                // the collected `password` is the NickServ password.
                let port = val("irc.port").trim().parse::<u16>().unwrap_or(6697);
                let channels: Vec<String> = val("irc.channels")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let nickserv = val("irc.password");
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.irc = Some(IrcChannelConfig {
                    server: val("irc.server"),
                    port,
                    nick: val("irc.nick"),
                    username: None,
                    channels,
                    use_tls: port == 6697,
                    nickserv_password: if nickserv.is_empty() {
                        None
                    } else {
                        Some(nickserv)
                    },
                    policy: None,
                    allow_bots: bot_policy("irc"),
                });
            }

            if toggled.iter().any(|c| c == "matrix") {
                // Onboarding collects matrix.{homeserver,username,password} — a
                // password-login set that maps cleanly onto MatrixChannelConfig
                // (homeserver + username + password). access_token left empty
                // (password login is the alternative path); rooms join on demand.
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.matrix = Some(MatrixChannelConfig {
                    homeserver: val("matrix.homeserver"),
                    access_token: String::new(),
                    username: Some(val("matrix.username")),
                    password: Some(val("matrix.password")),
                    user_id: None,
                    rooms: Vec::new(),
                    display_name: None,
                    policy: None,
                    accounts: HashMap::new(),
                    allow_bots: bot_policy("matrix"),
                });
            }

            if toggled.iter().any(|c| c == "x_twitter") {
                // Onboarding now uses X's official credential names:
                // bearer_token, consumer_key, consumer_key_secret,
                // access_token, access_token_secret, client_id, client_secret.
                // The core struct preserves legacy api_key/api_secret aliases,
                // but the wizard writes the canonical names from here on.
                let ch = cfg.channels.get_or_insert_with(Default::default);
                ch.x_twitter = Some(XTwitterChannelConfig {
                    bearer_token: val("x_twitter.bearer_token"),
                    consumer_key: val("x_twitter.consumer_key"),
                    consumer_key_secret: val("x_twitter.consumer_key_secret"),
                    access_token: val("x_twitter.access_token"),
                    access_token_secret: val("x_twitter.access_token_secret"),
                    client_id: val("x_twitter.client_id"),
                    client_secret: val("x_twitter.client_secret"),
                    poll_interval_secs: None,
                    auto_reply: false,
                    policy: None,
                    fanout: Vec::new(),
                });
            }

            // DEFERRED (intentionally NOT persisted here):
            //   • email — onboarding collects only smtp_host/smtp_port/username/
            //     password, but EmailChannelConfig also requires imap_host/
            //     imap_port/use_tls. Partial set → skip rather than write a
            //     half-config that can't connect.
            //   • whatsapp — onboarding collects phone_id/access_token, but
            //     WhatsAppChannelConfig's required fields don't match that pair
            //     (field↔struct mismatch); skip until the steps align.
            //   • signal — the QR-pair step collects no creds matching
            //     SignalChannelConfig (signal_cli_path); deferred.
            //   • discord.server_id / discord.channel_id — env-wiring task
            //     (DISCORD_RELAY_CHANNEL_IDS), not config.toml.
        }

        // ── mark onboarding done so the run loop never re-onboards ──
        cfg.onboarding_complete = true;

        // ── persist the provider credential INTO cfg BEFORE the whole-Config
        //    serialize, so the credential lands in config.toml (the single
        //    source of truth). The canonical Provider→env-var name comes from
        //    the same map the runtime reads (`Provider::env_key`); we derive the
        //    Provider from the model string via `parse_model()`.
        //
        //    CRITICAL (P0 #185): plain API keys MUST go into `cfg.credentials` —
        //    the `[credentials]` HashMap — the ONLY path the gateway reads for
        //    api-key auth. At startup `zeus-api` iterates `config.credentials`
        //    and `set_var`s each into the process env (lib.rs S70-A1 bridge);
        //    zeus-llm then resolves auth via `env::var(provider.env_key())`.
        //    OAuth setup-tokens route to `[provider_credentials.{provider}]`
        //    (branch-4 → AuthMethod::OAuth). No `.env` mirror — config.toml is
        //    the single source of truth. ──
        let key = self.auth_api_key.trim();
        if !key.is_empty() {
            let (provider, _) = cfg.parse_model();
            // OAuth tokens MUST route to `[provider_credentials.{provider}]`
            // cred_type="oauth" — the ONLY onboarding-reachable store the gateway
            // resolves as OAuth (zeus-llm from_config branch-4 → AuthMethod::OAuth
            // → Bearer). A plain API key goes to `[credentials]` keyed by env_key,
            // which the S70 bridge exports → process env → AuthMethod::ApiKey
            // (x-api-key). config.toml is the SINGLE source of truth — no .env.
            //
            // OAuth is signalled by the user's explicit auth-mode choice
            // (1=Setup Token, 2=Browser OAuth), NOT by the `sk-ant-oat` prefix.
            // (#257) The old prefix-only check was Anthropic-specific: every
            // non-Anthropic OAuth token (gemini-cli/qwen/minimax) failed the
            // `sk-ant-oat` test → fell into the api-key branch keyed by
            // env_key(). OAuth-only providers were silently dropped, while
            // API-key providers wrote the token to the wrong auth path.
            // Routing on auth_mode fixes all four providers;
            // the prefix is kept as a belt-and-suspenders OR for the Anthropic
            // setup-token paste path that may land in API-Key mode.
            let is_oauth = matches!(self.auth_mode, 1 | 2) || key.starts_with("sk-ant-oat");
            if is_oauth {
                if !cfg.provider_credentials.set_oauth(provider, key) {
                    // Provider not routable as OAuth in the read-side enum;
                    // fall back to the api-key store so auth isn't silently lost.
                    // (For gemini-cli/minimax with empty env_key this still
                    // drops — but set_oauth covers all 4 OAuth-edge providers,
                    // so this fallback is only hit for genuinely non-OAuth
                    // providers that shouldn't be in an OAuth mode anyway.)
                    let env_key = provider.env_key();
                    if !env_key.is_empty() {
                        cfg.credentials.insert(env_key.to_string(), key.to_string());
                    }
                }
            } else {
                let env_key = provider.env_key();
                if !env_key.is_empty() {
                    // Canonical store the gateway bridge exports → process env.
                    cfg.credentials.insert(env_key.to_string(), key.to_string());
                }
            }
        }

        // ── persist config.toml AFTER the credential write (whole-Config
        //    serialize; preserves sections we loaded and didn't touch). The
        //    credential is now part of `cfg`, so it lands in the file — this is
        //    the ordering fix: the prior code saved BEFORE writing the key, so
        //    `[credentials]` was never persisted (only the dropped .env mirror
        //    made it "work"). ──
        cfg.save_unchecked()
            .map_err(|e| format!("save config.toml: {e}"))?;

        Ok(cfg)
    }

    /// Build the 14 summary rows for the Complete screen from App state —
    /// mirrors JSX summary prop wiring (docs/zeus-tui-onboarding.jsx:2150-2172).
    fn build_summary(&self) -> Vec<SummaryRow> {
        use screens::complete::RowStatus::{Configured, Skipped};
        let row = |name: &str, value: String, status: screens::complete::RowStatus| SummaryRow {
            name: name.to_string(),
            value,
            status,
        };

        // Canonical id (display "glm" → "zai") so the review row matches the
        // persisted config.model exactly.
        let provider_id = zeus_core::Provider::from_prefix(screens::provider::provider_id_at(
            self.provider_selected,
        ))
        .name();
        let auth_label = screens::auth::AUTH_MODES[self.auth_mode.min(2)].label;
        let voice_id = self.voice_screen.selected_id();
        let image_id = self.images_screen.selected_id();
        let channels_n = self.channels_screen.selected.len();
        let fallback_n = self.fallback_screen.chain.len();
        let features_n = self.features_screen.toggled.iter().filter(|t| **t).count();
        let skills_n = self.skills_screen.installed.len();

        vec![
            row(
                "LLM Provider",
                format!("{}/{}", provider_id, self.model_screen.selected_model_id()),
                Configured,
            ),
            row(
                "Authentication",
                format!("{} · ✓ tested", auth_label),
                Configured,
            ),
            row(
                "Backup LLMs",
                format!("{} configured", fallback_n),
                if fallback_n > 0 { Configured } else { Skipped },
            ),
            row(
                "Channels",
                format!("{} bridged", channels_n),
                if channels_n > 0 { Configured } else { Skipped },
            ),
            row(
                "Gateway",
                format!("{}:{}", self.gateway_screen.host, self.gateway_screen.port),
                Configured,
            ),
            row(
                "Agent Persona",
                format!(
                    "{} ({})",
                    self.agent_screen.persona_name(),
                    self.agent_screen.summary_name()
                ),
                Configured,
            ),
            row("Workspace", self.workspace_path.clone(), Configured),
            row(
                "Security",
                format!("aegis level: {}", self.security_screen.config_level_value()),
                Configured,
            ),
            row(
                "Features",
                format!("{} subsystems on", features_n),
                Configured,
            ),
            row(
                "Voice (TTS)",
                voice_id.to_string(),
                if voice_id == "none" {
                    Skipped
                } else {
                    Configured
                },
            ),
            row(
                "Image Generator",
                image_id.to_string(),
                if image_id == "none" {
                    Skipped
                } else {
                    Configured
                },
            ),
            row(
                "Orchestration",
                self.orchestration_screen.selected_mode().to_string(),
                Configured,
            ),
            row(
                "Memory",
                format!("embeddings: {}", self.memory_screen.selected_id()),
                Configured,
            ),
            row(
                "Skills",
                format!("{} installed", skills_n),
                if skills_n > 0 { Configured } else { Skipped },
            ),
        ]
    }

    /// Run the real backend checks behind [TEST ALL BACKENDS] on the Complete
    /// screen. Two checks today:
    ///
    /// 1. **Provider API key** — the entered key is non-empty and matches the
    ///    provider's expected prefix (`key_fmt`, e.g. `sk-ant-...`). This is the
    ///    same lightweight format gate the Auth screen shows inline; it catches
    ///    paste-errors and wrong-provider keys without a network round-trip.
    /// 2. **Gateway reachability** — if the gateway service is enabled, attempt
    ///    a TCP connect to `host:port` (default `127.0.0.1:8080`). A successful
    ///    connect means an instance is already listening; a refused connect is
    ///    the expected happy path pre-AWAKEN (nothing running yet) and is
    ///    reported as pass with an informational detail. Only a malformed
    ///    host/port fails the check.
    ///
    /// Returns one [`TestResult`] per check; an empty vec is impossible here so
    /// the screen never falls into its "nothing verifiable" Failed branch.
    fn run_backend_checks(&self) -> Vec<screens::complete::TestResult> {
        use screens::complete::TestResult;
        let mut results = Vec::new();

        // --- Check 1: provider API key format ---
        // Derive the provider name + expected key format from the user's
        // provider selection, the same source the Auth screen renders from
        // (`provider_display`). The entered key lives in `auth_api_key`.
        let (provider_name, _color, key_fmt) =
            screens::provider::provider_display(self.provider_selected);
        let key = self.auth_api_key.trim();
        let prefix = key_fmt.replace("...", "");
        let (passed, detail) = if key.is_empty() {
            (false, "no API key entered".to_string())
        } else if !prefix.is_empty() && !key.starts_with(prefix.as_str()) {
            (false, format!("key does not match {key_fmt} format"))
        } else {
            (true, format!("{provider_name} key format OK"))
        };
        results.push(TestResult {
            name: "Provider API key".to_string(),
            passed,
            detail,
        });

        // --- Check 2: gateway TCP reachability ---
        // The gateway host:port is always configured in onboarding; validate
        // the port parses and probe whether anything is already bound. A
        // refused connect pre-AWAKEN is the expected happy path (nothing is
        // running yet), so it passes; only a malformed/unresolvable address
        // fails the check.
        let host = if self.gateway_screen.host.trim().is_empty() {
            "127.0.0.1"
        } else {
            self.gateway_screen.host.trim()
        };
        let (passed, detail) = match self.gateway_screen.port.trim().parse::<u16>() {
            Err(_) => (
                false,
                format!("invalid port '{}'", self.gateway_screen.port),
            ),
            Ok(port) => {
                use std::net::{TcpStream, ToSocketAddrs};
                use std::time::Duration;
                let addr = format!("{host}:{port}");
                match addr.to_socket_addrs() {
                    Err(_) => (false, format!("cannot resolve {addr}")),
                    Ok(mut addrs) => match addrs.next() {
                        None => (false, format!("cannot resolve {addr}")),
                        Some(sock) => {
                            match TcpStream::connect_timeout(&sock, Duration::from_millis(300)) {
                                Ok(_) => (true, format!("instance already listening on {addr}")),
                                // Connection refused pre-AWAKEN is expected:
                                // nothing is running yet. Treat as pass.
                                Err(_) => (true, format!("{addr} free (will bind on AWAKEN)")),
                            }
                        }
                    },
                }
            }
        };
        results.push(TestResult {
            name: "Gateway".to_string(),
            passed,
            detail,
        });

        results
    }

    /// Advance the animation clock one frame. Called by `run_loop` on each
    /// ~250ms tick — decoupling animation from keypresses so the cursor blinks
    /// and frame-cycling widgets animate even when the user is idle. Pure state
    /// mutation; the next `terminal.draw` reflects it.
    pub fn tick(&mut self) {
        self.anim_tick = self.anim_tick.wrapping_add(1);
        // Mirror the animation counter into stateful screens so their
        // `render(&self)` paths can cycle frames under `frame(f, app: &App)`.
        self.complete_screen.anim_tick = self.anim_tick;

        // AWAKEN handoff: once the "⚡ Launching Zeus…" frame has dwelled for
        // LAUNCH_DWELL_TICKS (~1.5s at 250ms/tick), complete the transition
        // into the production UI. Tick-driven so it never blocks the loop.
        // `wrapping_sub` keeps it correct even across the u64 wrap boundary.
        if self.launching
            && self.anim_tick.wrapping_sub(self.launch_started_tick) >= LAUNCH_DWELL_TICKS
        {
            self.launching = false;
            self.onboarding_complete = true;
            // AWAKEN-B: bring the gateway live BEFORE the prod UI renders, so
            // the in-process status-poll (every 5s) has a backend to find. The
            // spawn is `:8080`-guarded + `setsid`-detached, so it's idempotent
            // and survives this process. Fire exactly once at the flip edge.
            if !self.awaken_fired {
                self.awaken_fired = true;
                (self.awaken_spawn)();
            }
        }
        // Mirror the cursor-blink phase into stateful `&self`-rendered screens
        // so their insertion caret blinks under the global tick.
        let cursor_on = self.cursor_visible();
        self.gateway_screen.cursor_on = cursor_on;
        self.agent_screen.cursor_on = cursor_on;
    }

    /// True while the AWAKEN handoff frame should be shown. Renderers gate the
    /// "⚡ Launching Zeus…" splash on this.
    pub fn is_launching(&self) -> bool {
        self.launching
    }

    /// Cursor-blink phase derived from the animation clock: `true` = caret
    /// visible. Renderers gate the caret glyph on this so the blink is global
    /// and tick-driven (every text field shares one phase). At ~250ms/tick the
    /// caret shows for ~250ms then hides for ~250ms (~2Hz blink).
    pub fn cursor_visible(&self) -> bool {
        self.anim_tick.is_multiple_of(2)
    }

    /// Read-only view of the Auth-screen API key buffer. Exposes the entered
    /// text without leaking a mutable handle to the private field — lets
    /// integration tests assert the char→state→backspace input path on the
    /// Auth screen (the most-used text field, previously untested end-to-end).
    pub fn auth_api_key(&self) -> &str {
        &self.auth_api_key
    }

    /// Seed the entered API key. Public so integration tests (and any external
    /// driver) can populate the Auth field without the keystroke loop — needed
    /// since the #240 advance gate now blocks an unseeded Auth step.
    pub fn set_auth_api_key(&mut self, key: impl Into<String>) {
        self.auth_api_key = key.into();
    }

    /// Whether the entered API key is valid enough to advance past the Auth
    /// step (#240). A lightweight, network-free format gate: non-empty AND
    /// matches the selected provider's expected prefix (`key_fmt`, e.g.
    /// `sk-ant-...`). Same predicate the Auth screen renders inline and that
    /// `run_backend_checks` applies on the Complete screen — single source of
    /// truth so the advance gate, the inline ✓/✕, and the backend check never
    /// disagree. merakizzz's live test: a bare `"asdf"` advanced today because
    /// nothing gated the Auth Enter arm; this blocks that.
    pub fn auth_key_valid(&self) -> bool {
        let (_name, _color, key_fmt) = screens::provider::provider_display(self.provider_selected);
        let key = self.auth_api_key.trim();
        if key.is_empty() {
            return false;
        }
        let prefix = key_fmt.replace("...", "");
        prefix.is_empty() || key.starts_with(prefix.as_str())
    }

    /// Modifier-aware key entry point. Intercepts the universal advance
    /// control (Ctrl+N) — which works on EVERY onboarding screen, including
    /// the multiselect/grid screens (Channels, Voice, Features, …) where no
    /// printable key can advance because Enter toggles and Space toggles.
    /// All other keys fall through to the code-only `handle_key`.
    pub fn handle_key_mods(&mut self, key: KeyCode, mods: KeyModifiers) {
        // Ctrl+N = universal "Continue / Next" — only during onboarding.
        if !self.onboarding_complete
            && mods.contains(KeyModifiers::CONTROL)
            && matches!(key, KeyCode::Char('n') | KeyCode::Char('N'))
        {
            self.advance_step();
            return;
        }
        self.handle_key(key);
    }

    /// Universal step advance. On the Complete screen it fires AWAKEN (same as
    /// Enter); otherwise it steps forward one and runs the on-enter sync.
    /// This is the single advance path Ctrl+N drives on every screen.
    pub fn advance_step(&mut self) {
        if Step::from_index(self.current_step) == Some(Step::Complete) {
            // On Complete, advance = AWAKEN (mirror the Enter AWAKEN arm).
            // Persist every selection to config.toml + .env BEFORE transitioning.
            if let Err(e) = self.collect_and_persist() {
                self.complete_screen.set_persist_error(e);
                return;
            }
            self.onboarding_complete = true;
        } else if Step::from_index(self.current_step) == Some(Step::Auth) {
            // Auth (#239/#240): the fetch IS the validation. Advancing fires a
            // live `/v1/models` call; success populates the Model page and
            // advances, failure blocks + shows the error. No bypass path —
            // Ctrl+N / footer NEXT route here too.
            use crate::model_fetch::ModelFetchState;
            // Cheap format pre-check so an empty/obviously-malformed key never
            // fires a doomed call (instant ✕, no spinner).
            if !self.auth_key_valid() {
                // no-op; inline ✕ explains the block.
            } else {
                match &self.model_fetch_state {
                    ModelFetchState::Idle | ModelFetchState::Failed(_) => {
                        // Fire (or retry) the fetch; spinner renders next frame.
                        let provider_id =
                            screens::provider::provider_id_at(self.provider_selected).to_string();
                        let key = self.auth_api_key.trim().to_string();
                        if let Some(tx) = &self.fetch_tx {
                            self.model_fetch_state = ModelFetchState::Fetching;
                            let _ = tx.send((provider_id, key));
                        } else {
                            // Standalone: no runtime to fetch into → fall through
                            // to advance (static-list fallback on the Model page).
                            self.model_fetch_state = ModelFetchState::Idle;
                            self.current_step += 1;
                            self.on_step_enter();
                        }
                        // Don't advance — wait on the result.
                    }
                    ModelFetchState::Fetching => {
                        // In flight — no-op (the spinner is up).
                    }
                    ModelFetchState::Done(_) => {
                        // Validated + list pulled → advance.
                        self.current_step += 1;
                        self.on_step_enter();
                    }
                }
            }
        } else if self.current_step < Step::Complete as usize {
            self.current_step += 1;
            self.on_step_enter();
        }
    }

    /// Universal step BACK — one step earlier + on-enter sync. No-op on the
    /// first step. Mirrors the ESC/Left step-back idiom; this is the path the
    /// focusable footer BACK button drives.
    pub fn step_back(&mut self) {
        if self.current_step > 0 {
            self.current_step -= 1;
            self.on_step_enter();
        }
    }

    /// How many of the current screen's OWN focusable fields the Tab key walks
    /// before it should fall through to the footer (BACK → NEXT → wrap). Grid /
    /// multiselect screens that track no Tab-field return 0 → Tab goes straight
    /// to the footer (per A2 + the grid-screen rule). App-owned so the footer's
    /// Tab-reachability never depends on a screen's internal `focused_field`.
    fn footer_field_count(&self) -> usize {
        match Step::from_index(self.current_step) {
            // Screens whose Tab arm cycles internal fields (see handle_key Tab).
            Some(Step::Instance) => self.instance_screen.field_count(),
            Some(Step::Agent) => 3, // AgentScreen::FIELD_COUNT
            Some(Step::Voice) => self.voice_screen.field_count(),
            Some(Step::Images) => self.images_screen.field_count(),
            Some(Step::Memory) => self.memory_screen.field_count(),
            // Orchestration / Skills / Complete have their own Tab semantics
            // (handle_tab / next_category / focus_next_button) — treat them as
            // grid-like (0) so Tab reaches the footer directly; their own keys
            // still work via the existing arms when footer is unfocused.
            _ => 0,
        }
    }

    /// Advance the App-owned Tab cursor one stop and recompute `footer_focus`.
    /// Cycle: [screen fields 0..n) → BACK → NEXT → wrap(0). When the cursor is
    /// within the screen-field range, the existing per-screen Tab arm runs and
    /// `footer_focus` is `None`; at n it's BACK, at n+1 NEXT, at n+2 it wraps.
    fn tab_advance(&mut self) {
        let n = self.footer_field_count();
        let total = n + 2; // screen fields + BACK + NEXT
        // tab_cursor counts Tab presses since screen-enter (1-indexed after the
        // increment). Presses 1..=n walk the screen's own fields (footer None);
        // press n+1 = BACK; press n+2 = NEXT; then it wraps to 0 (the next Tab
        // re-enters the field range at 1).
        self.tab_cursor = (self.tab_cursor + 1) % total;
        self.footer_focus = if self.tab_cursor == n + 1 {
            Some(FooterFocus::Back)
        } else if self.tab_cursor == 0 {
            // Wrapped: cursor 0 means we just passed NEXT (the (n+2)th press
            // mod total == 0). That press IS the NEXT stop before the wrap.
            Some(FooterFocus::Next)
        } else {
            None
        };
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        // Production UI key handling
        if self.onboarding_complete {
            self.handle_key_prod(key);
            return;
        }

        match key {
            // NOTE: there is deliberately NO bare `Char('q')` quit arm here.
            // It used to live first and unconditionally set `should_quit`, which
            // swallowed `q` in every onboarding text field (Auth API key,
            // ChanConfig creds, Agent/Orchestration/Memory/Workspace paths,
            // Skills filter) — typing `q` quit instead of inserting. Quit is
            // fully covered upstream: Ctrl+C / Ctrl+Q hard-quit in the event
            // loop (see `run_loop`, before dispatch) and Esc steps back. Every
            // printable char — including `q` — now falls through to the
            // `Char(c)` text-input catch-all below.
            // ESC = back one step (does NOT quit). On the first step it is a
            // no-op. Mirrors the Left step-back idiom; on Mode it steps back
            // off the screen rather than moving the card selection.
            KeyCode::Esc if self.current_step > 0 => {
                self.current_step -= 1;
                self.on_step_enter();
            }
            KeyCode::Right => {
                // Mode screen lays its 3 cards out horizontally → ←/→ move the
                // card selection (not step-nav). Enter advances past Mode.
                if Step::from_index(self.current_step) == Some(Step::Mode) {
                    if self.mode_selected < 2 {
                        self.mode_selected += 1;
                    }
                } else if Step::from_index(self.current_step) == Some(Step::Channels) {
                    // Channels is a 2-col grid → → moves focus to the next card
                    // (not step-nav). Enter/Space toggles; Esc backs out.
                    self.channels_screen.move_right();
                } else if Step::from_index(self.current_step) == Some(Step::Agent) {
                    // Agent persona is a 2-col grid → → moves persona column
                    // (not step-nav). Tab cycles identity fields; Esc backs out.
                    self.agent_screen.move_right();
                } else if Step::from_index(self.current_step) == Some(Step::Security) {
                    // Security is a 4-col level grid → → moves the selected
                    // level card (not step-nav). Enter advances; Esc backs out.
                    self.security_screen.select_next();
                } else if Step::from_index(self.current_step) == Some(Step::Gateway) {
                    // Gateway service picker is a 4-col grid → → moves the
                    // selected service card (not step-nav). Esc backs out.
                    self.gateway_screen.move_right();
                } else if Step::from_index(self.current_step) == Some(Step::Orchestration) {
                    // Orchestration is a horizontal 3-col mode grid → → moves the
                    // selected mode (not step-nav). Tab cycles timing fields.
                    self.orchestration_screen.move_right();
                } else if Step::from_index(self.current_step) == Some(Step::Skills) {
                    // Skills: ←/→ switch the category tab (grid-local, not
                    // step-nav). Filter via typing; Space/Enter toggle install.
                    self.skills_screen.next_category();
                } else if self.current_step < Step::Complete as usize {
                    self.advance_step();
                }
            }
            KeyCode::Enter => {
                // Focusable footer takes priority: if BACK/NEXT is Tab-focused,
                // Enter activates it (step_back / advance_step) instead of the
                // screen's own Enter action. Mirrors complete.rs button-focus.
                if let Some(f) = self.footer_focus {
                    match f {
                        FooterFocus::Back => self.step_back(),
                        FooterFocus::Next => self.advance_step(),
                    }
                    return;
                }
                match Step::from_index(self.current_step) {
                    Some(Step::Fallback) => {
                        self.fallback_screen.toggle();
                    }
                    // Grid/multiselect screens: Enter ADVANCES the flow
                    // (Space owns toggle here — see the Char(' ') arm). This is
                    // the merakizzz wedge fix: Enter felt stuck because it was
                    // consumed by the per-screen toggle/test instead of nav.
                    Some(Step::Channels) | Some(Step::Features) | Some(Step::Voice)
                    | Some(Step::Images) => {
                        self.advance_step();
                    }
                    Some(Step::ChannelConfig) => {
                        // On the test button, Enter fires the test; on the
                        // allow_bots selector, Enter cycles the policy;
                        // otherwise Enter advances (server_id etc. are
                        // arrow-navigable).
                        if self.chanconfig_screen.focused_field.starts_with("test:") {
                            self.chanconfig_screen.trigger_test();
                        } else if let Some(ch_id) = self
                            .chanconfig_screen
                            .focused_field
                            .strip_prefix("allowbots:")
                            .map(str::to_string)
                        {
                            self.chanconfig_screen.cycle_bot_policy(&ch_id);
                        } else {
                            self.advance_step();
                        }
                    }
                    Some(Step::Gateway) => {
                        self.gateway_screen.toggle_service();
                    }
                    Some(Step::Skills) => {
                        self.skills_screen.toggle_selected();
                    }
                    Some(Step::Complete) => {
                        if self.complete_screen.focused_button == 0 {
                            let results = self.run_backend_checks();
                            self.complete_screen.run_test_all(results);
                        } else {
                            // AWAKEN ZEUS — persist config.toml + .env, then
                            // transition to the production UI.
                            if let Err(e) = self.collect_and_persist() {
                                self.complete_screen.set_persist_error(e);
                            } else {
                                // Begin the visible AWAKEN handoff instead of
                                // an instant silent swap into the prod UI. The
                                // tick clock flips `onboarding_complete` after
                                // LAUNCH_DWELL_TICKS so the "⚡ Launching Zeus…"
                                // frame is actually seen.
                                self.launching = true;
                                self.launch_started_tick = self.anim_tick;
                            }
                        }
                    }
                    // Auth (#239/#240): Enter is another NEXT activation, so it
                    // must route through `advance_step()` where the live
                    // `/v1/models` probe is the gate. The old direct
                    // `current_step += 1` path skipped the probe after only the
                    // cheap format pre-check.
                    Some(Step::Auth) => {
                        self.advance_step();
                    }
                    _ => {
                        if self.current_step < Step::Complete as usize {
                            self.current_step += 1;
                            self.on_step_enter();
                        }
                    }
                }
            }
            KeyCode::Left => {
                if Step::from_index(self.current_step) == Some(Step::Mode) {
                    if self.mode_selected > 0 {
                        self.mode_selected -= 1;
                    }
                } else if Step::from_index(self.current_step) == Some(Step::Channels) {
                    // Channels is a 2-col grid → ← moves focus to the previous card
                    // (not step-back). Esc backs out of the step.
                    self.channels_screen.move_left();
                } else if Step::from_index(self.current_step) == Some(Step::Agent) {
                    // Agent persona is a 2-col grid → ← moves persona column
                    // (not step-back). Esc backs out of the step.
                    self.agent_screen.move_left();
                } else if Step::from_index(self.current_step) == Some(Step::Security) {
                    // Security is a 4-col level grid → ← moves the selected
                    // level card (not step-back). Esc backs out of the step.
                    self.security_screen.select_prev();
                } else if Step::from_index(self.current_step) == Some(Step::Gateway) {
                    // Gateway service picker is a 4-col grid → ← moves the
                    // selected service card (not step-back). Esc backs out.
                    self.gateway_screen.move_left();
                } else if Step::from_index(self.current_step) == Some(Step::Orchestration) {
                    // Orchestration is a horizontal 3-col mode grid → ← moves the
                    // selected mode (not step-back). Esc backs out.
                    self.orchestration_screen.move_left();
                } else if Step::from_index(self.current_step) == Some(Step::Skills) {
                    // Skills: ←/→ switch the category tab (grid-local, not
                    // step-back). Esc backs out of the step.
                    self.skills_screen.prev_category();
                } else if self.current_step > 0 {
                    self.current_step -= 1;
                    self.on_step_enter();
                }
            }
            KeyCode::Up => match Step::from_index(self.current_step) {
                Some(Step::Mode) if self.mode_selected > 0 => {
                    self.mode_selected -= 1;
                }
                Some(Step::Provider) if self.provider_selected > 0 => {
                    self.provider_selected -= 1;
                }
                Some(Step::Model) => {
                    self.model_screen.move_up();
                }
                Some(Step::Auth) if self.auth_mode > 0 => {
                    self.auth_mode -= 1;
                }
                Some(Step::Fallback) => {
                    self.fallback_screen.move_up();
                }
                Some(Step::Channels) => {
                    self.channels_screen.move_up();
                }
                Some(Step::ChannelConfig) => {
                    self.chanconfig_screen.focus_prev();
                }
                Some(Step::Gateway) => {
                    self.gateway_screen.move_up();
                }
                Some(Step::Agent) => {
                    self.agent_screen.move_up();
                }
                Some(Step::Workspace) if self.workspace_focused_field > 0 => {
                    self.workspace_focused_field -= 1;
                }
                Some(Step::Security) => {
                    self.security_screen.select_prev();
                }
                Some(Step::Features) => {
                    self.features_screen.move_up();
                }
                Some(Step::Voice) => {
                    self.voice_screen.select_prev();
                }
                Some(Step::Images) => {
                    self.images_screen.select_prev();
                }
                Some(Step::Orchestration) => {
                    self.orchestration_screen.handle_up();
                }
                Some(Step::Memory) => {
                    self.memory_screen.select_prev();
                }
                Some(Step::Skills) => {
                    self.skills_screen.move_up();
                }
                _ => {}
            },
            KeyCode::Down => match Step::from_index(self.current_step) {
                Some(Step::Mode) if self.mode_selected < 2 => {
                    self.mode_selected += 1;
                }
                Some(Step::Provider)
                    if self.provider_selected < crate::screens::providers::PROVIDERS.len() - 1 =>
                {
                    self.provider_selected += 1;
                }
                Some(Step::Model) => {
                    self.model_screen.move_down();
                }
                Some(Step::Auth) if self.auth_mode < 2 => {
                    self.auth_mode += 1;
                }
                Some(Step::Fallback) => {
                    self.fallback_screen.move_down();
                }
                Some(Step::Channels) => {
                    self.channels_screen.move_down();
                }
                Some(Step::ChannelConfig) => {
                    self.chanconfig_screen.focus_next();
                }
                Some(Step::Gateway) => {
                    self.gateway_screen.move_down();
                }
                Some(Step::Agent) => {
                    self.agent_screen.move_down();
                }
                Some(Step::Workspace) if self.workspace_focused_field < 2 => {
                    self.workspace_focused_field += 1;
                }
                Some(Step::Security) => {
                    self.security_screen.select_next();
                }
                Some(Step::Features) => {
                    self.features_screen.move_down();
                }
                Some(Step::Voice) => {
                    self.voice_screen.select_next();
                }
                Some(Step::Images) => {
                    self.images_screen.select_next();
                }
                Some(Step::Orchestration) => {
                    self.orchestration_screen.handle_down();
                }
                Some(Step::Memory) => {
                    self.memory_screen.select_next();
                }
                Some(Step::Skills) => {
                    self.skills_screen.move_down();
                }
                _ => {}
            },
            KeyCode::Char(c) => {
                // Space activates a Tab-focused footer button (BACK/NEXT),
                // mirroring the Enter footer arm. Only when the footer is
                // focused — otherwise Space falls through to its per-screen use
                // (toggle install on Skills/Channels/Features text input etc.).
                if Step::from_index(self.current_step) == Some(Step::Instance)
                    && c == ' '
                    && self.footer_focus.is_none()
                {
                    self.instance_screen.toggle_target();
                    return;
                }
                if let (' ', Some(f)) = (c, self.footer_focus) {
                    match f {
                        FooterFocus::Back => self.step_back(),
                        FooterFocus::Next => self.advance_step(),
                    }
                    return;
                }
                if Step::from_index(self.current_step) == Some(Step::Instance) {
                    self.instance_screen.handle_char(c);
                    return;
                }
                // Fallback: [ / ] reorder the chain (matches JSX hint).
                if self.current_step == Step::Fallback as usize && (c == '[' || c == ']') {
                    if c == '[' {
                        self.fallback_screen.chain_move_up();
                    } else {
                        self.fallback_screen.chain_move_down();
                    }
                } else if self.current_step == Step::Channels as usize && c == ' ' {
                    // Space toggles the focused channel (Space/Enter per spec).
                    self.channels_screen.toggle_focused();
                } else if self.current_step == Step::Features as usize && c == ' ' {
                    // Space toggles the focused subsystem (Space/Enter per spec;
                    // toggle_focused skips mandatory talos@macOS).
                    self.features_screen.toggle_focused();
                } else if self.current_step == Step::Auth as usize {
                    self.auth_api_key.push(c);
                    self.auth_test_status = None;
                } else if self.current_step == Step::ChannelConfig as usize {
                    self.chanconfig_screen.input_char(c);
                } else if self.current_step == Step::Orchestration as usize {
                    self.orchestration_screen.input_char(c);
                } else if self.current_step == Step::Agent as usize {
                    self.agent_screen.input_char(c);
                } else if self.current_step == Step::Gateway as usize {
                    // Gateway: 1/2/3 toggle the three FEATURES pills; other
                    // chars type into the focused BIND field (0=host, 1=port).
                    match c {
                        '1' => self.gateway_screen.toggle_feature(0),
                        '2' => self.gateway_screen.toggle_feature(1),
                        '3' => self.gateway_screen.toggle_feature(2),
                        _ => {
                            if self.gateway_screen.focused_field == 0 {
                                self.gateway_screen.host.push(c);
                            } else {
                                self.gateway_screen.port.push(c);
                            }
                        }
                    }
                } else if self.current_step == Step::Workspace as usize {
                    // Type into the focused path field (0=workspace,
                    // 1=sessions, 2=mnemosyne).
                    match self.workspace_focused_field {
                        0 => self.workspace_path.push(c),
                        1 => self.sessions_path.push(c),
                        2 => self.mnemosyne_path.push(c),
                        _ => {}
                    }
                } else if self.current_step == Step::Voice as usize {
                    self.voice_screen.input_char(c);
                } else if self.current_step == Step::Images as usize {
                    self.images_screen.input_char(c);
                } else if self.current_step == Step::Memory as usize {
                    self.memory_screen.input_char(c);
                } else if self.current_step == Step::Skills as usize {
                    // Space toggles install on the focused skill; any other
                    // printable char feeds the live filter (JSX filter input).
                    if c == ' ' {
                        self.skills_screen.toggle_selected();
                    } else {
                        self.skills_screen.filter_push(c);
                    }
                }
            }
            KeyCode::Tab => {
                // Complete is the TERMINAL screen — no global BACK/NEXT footer
                // (there's nothing after it). Its Tab drives the screen's OWN
                // TEST/AWAKEN button focus, untouched by the footer cycle.
                if self.current_step == Step::Complete as usize {
                    self.complete_screen.focus_next_button();
                    return;
                }
                // App-owned cycle: walk the screen's own fields, then BACK →
                // NEXT → wrap. `tab_advance` recomputes `footer_focus`; when it
                // lands inside the screen-field range (footer_focus == None) we
                // run that screen's existing per-screen Tab arm.
                self.tab_advance();
                if self.footer_focus.is_none() {
                    if self.current_step == Step::Instance as usize {
                        self.instance_screen.focus_next();
                    } else if self.current_step == Step::Agent as usize {
                        self.agent_screen.focus_next_field();
                    } else if self.current_step == Step::Voice as usize {
                        self.voice_screen.focus_next();
                    } else if self.current_step == Step::Images as usize {
                        self.images_screen.focus_next();
                    } else if self.current_step == Step::Orchestration as usize {
                        self.orchestration_screen.handle_tab();
                    } else if self.current_step == Step::Memory as usize {
                        self.memory_screen.focus_next();
                    } else if self.current_step == Step::Skills as usize {
                        self.skills_screen.next_category();
                    }
                }
            }
            KeyCode::Backspace => {
                if self.current_step == Step::Instance as usize {
                    self.instance_screen.backspace();
                } else if self.current_step == Step::Auth as usize {
                    self.auth_api_key.pop();
                    self.auth_test_status = None;
                } else if self.current_step == Step::ChannelConfig as usize {
                    self.chanconfig_screen.input_backspace();
                } else if self.current_step == Step::Orchestration as usize {
                    self.orchestration_screen.input_backspace();
                } else if self.current_step == Step::Agent as usize {
                    self.agent_screen.input_backspace();
                } else if self.current_step == Step::Gateway as usize {
                    if self.gateway_screen.focused_field == 0 {
                        self.gateway_screen.host.pop();
                    } else {
                        self.gateway_screen.port.pop();
                    }
                } else if self.current_step == Step::Workspace as usize {
                    match self.workspace_focused_field {
                        0 => {
                            self.workspace_path.pop();
                        }
                        1 => {
                            self.sessions_path.pop();
                        }
                        2 => {
                            self.mnemosyne_path.pop();
                        }
                        _ => {}
                    }
                } else if self.current_step == Step::Voice as usize {
                    self.voice_screen.input_backspace();
                } else if self.current_step == Step::Images as usize {
                    self.images_screen.input_backspace();
                } else if self.current_step == Step::Memory as usize {
                    self.memory_screen.input_backspace();
                } else if self.current_step == Step::Skills as usize {
                    self.skills_screen.filter_pop();
                }
            }
            _ => {}
        }
    }
}

/// The frame() contract — §0.2 of the rebuild plan.
/// Every screen is drawn by this single helper:
///   1. Paint opaque full-screen background FIRST (#0a0a0f)
///   2. Chrome (TopBar + StatusBar)
///   3. Content body
/// #270 — clear an onboarding screen's content region before the active screen
/// renders into it. The top-of-`frame()` `bg` paint only rewrites cell *style*,
/// NOT the symbol (the gate-b trap documented in provider.rs), and onboarding
/// screens don't fill their whole `body[1]` (they paint cards/panels +
/// `set_string` individual lines, leaving gap cells) — so without this, glyphs
/// from the prior step survive in the gaps, producing the bleed-through (worst
/// on the providers list). A bare `Clear` resets symbols to spaces but reverts
/// to the terminal-default bg, punching holes in the themed background; so we
/// Clear-then-repaint `theme::BG` — the proven #250 idiom — giving every screen
/// a clean, correctly-colored slate. Idempotent + dispatcher-level = no
/// per-screen drift (provider.rs/model.rs keep their finer per-region clears on
/// top). Pinned by `onboarding_dispatcher_clears_bleed_between_steps`.
fn clear_body_region(area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
    Clear.render(area, buf);
    Block::default()
        .style(Style::default().bg(theme::BG))
        .render(area, buf);
}

pub fn frame(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();

    // 1. Opaque background — kills bleed-through / transparency bugs structurally
    let bg = Block::default().style(Style::default().bg(theme::BG));
    bg.render(area, f.buffer_mut());

    // AWAKEN handoff: a visible "⚡ Launching Zeus…" splash so pressing AWAKEN
    // clearly DOES something. The tick clock flips `launching` → `onboarding_complete`
    // after LAUNCH_DWELL_TICKS, at which point the prod route below takes over.
    if app.launching {
        frame_launching(f, app, area);
        return;
    }

    // Route to production UI if onboarding is complete
    if app.onboarding_complete {
        frame_prod(f, app, area);
        return;
    }

    // 2. Chrome layout: TopBar | StepIndicator | Body (StepHeader + Content) | StatusBar
    let chrome = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // TopBar
            Constraint::Length(1), // StepIndicator (windowed progress rail)
            Constraint::Min(0),    // Body
            Constraint::Length(1), // StatusBar
        ])
        .split(area);

    // TopBar
    let top_bar = TopBar {
        step_idx: app.current_step,
        hostname: "~/.zeus/config.toml".to_string(),
    };
    f.render_widget(top_bar, chrome[0]);

    // StepIndicator — windowed progress rail (current ±4 + first/last anchors).
    f.render_widget(
        StepIndicator {
            current: app.current_step,
        },
        chrome[1],
    );

    // Body: StepHeader + screen content
    let step = Step::from_index(app.current_step);
    if let Some(step) = step {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // StepHeader
                Constraint::Min(0),    // Screen content
            ])
            .split(chrome[2]);

        // StepHeader
        let header = StepHeader {
            step_idx: app.current_step,
            title: step.title(),
            subtitle: step.subtitle(),
        };
        f.render_widget(header, body[0]);

        // Screen content — uniform Clear-then-bg on the content region before
        // delegating to the active screen (#270). The full-screen `bg` paint at
        // the top of `frame()` only rewrites cell *style*, NOT the cell symbol
        // (the gate-b trap documented in provider.rs) — so glyphs from the prior
        // step's screen survive in `body[1]` when steps change, producing the
        // onboarding bleed-through (worst on the providers list). A bare `Clear`
        // resets symbols to spaces but reverts to the terminal-default bg,
        // punching holes in the themed background; so we Clear-then-repaint
        // `theme::BG` — the proven #250 idiom — giving every screen a clean,
        // correctly-colored slate. Idempotent + dispatcher-level = no per-screen
        // drift (provider.rs/model.rs keep their finer per-region clears on top).
        clear_body_region(body[1], f.buffer_mut());

        match step {
            Step::Welcome => {
                let screen = WelcomeScreen {
                    existing_config: app.existing_config,
                    anim_tick: app.anim_tick,
                };
                f.render_widget(screen, body[1]);
            }
            Step::Mode => {
                let screen = ModeScreen {
                    selected: app.mode_selected,
                };
                f.render_widget(screen, body[1]);
            }
            Step::Instance => {
                app.instance_screen.render_with_cursor(
                    body[1],
                    f.buffer_mut(),
                    app.cursor_visible(),
                );
            }
            Step::Provider => {
                let screen = ProviderScreen {
                    selected: app.provider_selected,
                };
                f.render_widget(screen, body[1]);
            }
            Step::Auth => {
                // Drive Auth from the provider the user actually selected.
                let (pname, pcolor, pkey) =
                    screens::provider::provider_display(app.provider_selected);
                // Canonical registry id for the selected provider — drives the
                // config-write preview's `[credentials] <ENV_KEY>` line (#268).
                let pid = screens::provider::provider_id_at(app.provider_selected);
                // Derive test_status from the live fetch state (#239/#240) so
                // the existing spinner/error arms drive off the real call.
                // AuthScreen is rebuilt every frame from App state, so this is
                // the wire — no struct change. Fetching→spinner ("testing"),
                // Failed→error; otherwise fall back to the manual test status.
                let test_status = match &app.model_fetch_state {
                    crate::model_fetch::ModelFetchState::Fetching => Some("testing"),
                    crate::model_fetch::ModelFetchState::Failed(_) => Some("error"),
                    _ => app.auth_test_status,
                };
                let screen = AuthScreen {
                    provider_name: pname,
                    provider_id: pid,
                    provider_color: pcolor,
                    key_fmt: pkey,
                    selected_mode: app.auth_mode,
                    api_key: app.auth_api_key.clone(),
                    test_status,
                    cursor_on: app.cursor_visible(),
                };
                screen.render(body[1], f.buffer_mut());
            }
            Step::Model => {
                app.model_screen.render(body[1], f.buffer_mut());
            }
            Step::Fallback => {
                app.fallback_screen.render(body[1], f.buffer_mut());
            }
            Step::Channels => {
                app.channels_screen.render(body[1], f.buffer_mut());
            }
            Step::ChannelConfig => {
                app.chanconfig_screen.render_with_cursor(
                    body[1],
                    f.buffer_mut(),
                    app.cursor_visible(),
                );
            }
            Step::Gateway => {
                app.gateway_screen.render(body[1], f.buffer_mut());
            }
            Step::Agent => {
                app.agent_screen.render(body[1], f.buffer_mut());
            }
            Step::Workspace => {
                let screen = WorkspaceScreen {
                    workspace_path: app.workspace_path.clone(),
                    sessions_path: app.sessions_path.clone(),
                    mnemosyne_path: app.mnemosyne_path.clone(),
                    existing_detected: app.workspace_existing_detected,
                    memory_facts: app.workspace_memory_facts,
                    session_count: app.workspace_session_count,
                    // Static example per dispatch; FOLLOW-UP: real dir mtime scan.
                    existing_mtime: "2 minutes ago".to_string(),
                    focused_field: app.workspace_focused_field,
                    cursor_on: app.cursor_visible(),
                };
                screen.render(body[1], f.buffer_mut());
            }
            Step::Security => {
                app.security_screen.render(body[1], f.buffer_mut());
            }
            Step::Features => {
                let screen = FeaturesScreen {
                    toggled: app.features_screen.toggled.clone(),
                    focused: app.features_screen.focused,
                    platform: app.features_screen.platform,
                };
                f.render_widget(screen, body[1]);
            }
            Step::Voice => {
                app.voice_screen.render(body[1], f.buffer_mut());
            }
            Step::Images => {
                app.images_screen
                    .render_with_cursor(body[1], f.buffer_mut(), app.cursor_visible());
            }
            Step::Orchestration => {
                app.orchestration_screen.render_with_cursor(
                    body[1],
                    f.buffer_mut(),
                    app.cursor_visible(),
                );
            }
            Step::Memory => {
                app.memory_screen
                    .render_with_cursor(body[1], f.buffer_mut(), app.cursor_visible());
            }
            Step::Skills => {
                app.skills_screen
                    .render_with_cursor(body[1], f.buffer_mut(), app.cursor_visible());
            }
            Step::Complete => {
                app.complete_screen.render(body[1], f.buffer_mut());
            }
        }
    }

    // StatusBar
    let status_bar = StatusBar {
        can_back: app.current_step > 0,
        can_continue: app.current_step < Step::Complete as usize,
        footer_highlight: match app.footer_focus {
            Some(FooterFocus::Back) => crate::widgets::status_bar::FooterHighlight::Back,
            Some(FooterFocus::Next) => crate::widgets::status_bar::FooterHighlight::Next,
            None => crate::widgets::status_bar::FooterHighlight::None,
        },
    };
    f.render_widget(status_bar, chrome[3]);
}

/// Production UI frame — renders after onboarding completes.
/// TopBar + TabBar + active tab content + StatusBar.
/// The AWAKEN handoff splash. Rendered while `app.launching` is true (a short
/// tick-driven dwell) so pressing AWAKEN gives clear, visible feedback before
/// the production UI replaces the onboarding wizard. A trailing ellipsis cycles
/// off the animation clock so the frame reads as active, not frozen.
fn frame_launching(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::layout::Alignment;
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    // Animated ellipsis (0–3 dots) driven by the shared tick clock.
    let dots = ".".repeat((app.anim_tick % 4) as usize);
    let headline = format!("⚡ Launching Zeus{dots}");

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            headline,
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Onboarding complete — handing off to the production interface",
            Style::default().fg(theme::DIM),
        )),
    ];

    // Vertically center the splash block in the available area.
    let block_height = lines.len() as u16;
    let top_pad = area.height.saturating_sub(block_height) / 2;
    let centered = ratatui::layout::Rect {
        x: area.x,
        y: area.y.saturating_add(top_pad),
        width: area.width,
        height: block_height.min(area.height),
    };

    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .render(centered, f.buffer_mut());
}

/// Reset every cell in `area` to a blank space with the default body background.
///
fn frame_prod(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    // Production chrome: ProdTopBar | ProdTabBar | Content | ProdStatusBar
    let chrome = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // ProdTopBar
            Constraint::Length(1), // ProdTabBar
            Constraint::Min(0),    // Tab content
            Constraint::Length(1), // ProdStatusBar
        ])
        .split(area);

    // ProdTopBar — live gateway host/port + connection state from App.
    let top_bar = ProdTopBar {
        hostname: app.gateway_host.clone(),
        port: app.gateway_port,
        conn_state: app.conn_state,
        ctx_percent: 0, // TODO Phase 2: real context-window % from session stats
    };
    f.render_widget(top_bar, chrome[0]);

    // ProdTabBar
    let tab_bar = ProdTabBar {
        active_idx: app.prod_active_tab,
    };
    f.render_widget(tab_bar, chrome[1]);

    // Tab content — clear the body region FIRST so stale glyphs from the
    // previous tab can't bleed through (gate-b fix). Idempotent: tabs that
    // paint their own opaque bg are unaffected.
    let tab_id = PRIMARY_TABS.get(app.prod_active_tab).map(|t| t.id);
    clear_body_region(chrome[2], f.buffer_mut());

    // If drilled into an advanced subview, render header + subview body
    if let Some(detail_idx) = app.prod_adv_detail {
        if let Some(tab) = ADVANCED_TABS.get(detail_idx) {
            render_advanced_detail(f, chrome[2], tab, app);
        }
    } else if let Some(adv_idx) = app.prod_active_adv {
        // Advanced overlay open — the 3-column grid selector
        let overlay = AdvancedOverlay {
            selected: Some(adv_idx),
        };
        f.render_widget(overlay, chrome[2]);
    } else if tab_id == Some("advanced") {
        // Show advanced grid with no selection
        let overlay = AdvancedOverlay { selected: None };
        f.render_widget(overlay, chrome[2]);
    } else if tab_id == Some("chat") {
        // Chat tab — fully functional
        let chat = ChatTab {
            messages: &app.prod_chat_messages,
            input: &app.prod_chat_input,
            stream_state: app.prod_stream_state,
            scroll_offset: app.prod_chat_scroll,
            slash_open: app.prod_slash_open,
            cursor_on: app.cursor_visible(),
            anim_tick: app.anim_tick,
            tool_feed: &app.prod_tool_feed,
            iter_count: app.prod_iter_count,
            active_tasks: app.prod_active_tasks.as_deref(),
            model_badge: app.prod_status.as_ref().map(|s| s.model.as_str()),
            ctx_percent: 0,
        };
        f.render_widget(chat, chrome[2]);
    } else if tab_id == Some("tools") {
        // Tools tab — full 3-column browser (JSX ToolsTab line 816)
        let tools = ToolsTab {
            selected_category: app.prod_tools_selected_category.as_deref(),
            selected_tool: &app.prod_tools_selected_tool,
            tool_filter: &app.prod_tools_filter,
            scroll_offset: app.prod_tools_scroll,
            tools: app.live_tools,
        };
        f.render_widget(tools, chrome[2]);
    } else if tab_id == Some("approvals") {
        // Approvals tab — overlay live pending approvals from /v1/approvals (#235).
        // Falls back to DEFAULT_PENDING const catalog until the first fetch lands.
        let approvals = ApprovalsTab::with_live(app.prod_approvals.as_deref());
        f.render_widget(approvals, chrome[2]);
    } else if tab_id == Some("channels") {
        let channels = ChannelsTab::with_live(app.prod_channels.as_deref());
        f.render_widget(channels, chrome[2]);
    } else if tab_id == Some("wallet") {
        // Wallet tab — web4 economy (zeus-economy CR + zeus-wallet ZEUS). #190.
        // Live economy data overlay (#235): CR balance from /v1/economy/wallets,
        // transactions from /v1/economy/transactions. Falls back to mock sample.
        let live = prod::WalletLive {
            wallets: app.prod_economy_wallets.as_deref(),
            transactions: app.prod_economy_txs.as_deref(),
        };
        let wallet =
            prod::WalletTab::with_live(app.prod_wallet_view, app.prod_wallet_titan_sel, live);
        f.render_widget(wallet, chrome[2]);
    } else if tab_id == Some("memory") {
        // Memory tab — overlay live gateway data (workspace files, sessions,
        // Mnemosyne hits) onto the JSX-parity schema. Each `MemoryLive` field
        // is `None` until its poll-worker (lib.rs run()) lands the first fetch;
        // the matching sub-tab falls back to const placeholders while absent.
        use prod::memory_tab::{MemoryLive, MemorySubTab, render_memory_tab};
        let live = MemoryLive {
            files: app.prod_memory_files.as_deref(),
            sessions: app.prod_sessions.as_deref(),
            search: app.prod_memory_search.as_deref(),
        };
        render_memory_tab(chrome[2], f.buffer_mut(), MemorySubTab::Workspace, 0, live);
    } else if tab_id == Some("settings") {
        // Settings tab — overlay live `GET /v1/config` values onto the static
        // section schema. `prod_config_rows` is `None` until the config
        // poll-worker (lib.rs run()) lands the first fetch; the tab falls back
        // to const placeholders per-key when live data is absent.
        let settings = prod::SettingsTab::with_config(app.prod_config_rows.as_ref())
            .with_active(app.prod_settings_section);
        f.render_widget(settings, chrome[2]);
    } else if tab_id == Some("office") {
        // Office tab — pixel canvas + 280-wide sidebar.
        // Live agent data overlay (#235): status/task/active-idle counts from
        // /v1/network/agents. Falls back to const AGENTS roster.
        let live = prod::OfficeLive {
            agents: app.prod_agents.as_deref(),
            status: app.prod_status.as_ref(),
        };
        let office = prod::OfficeTab::with_live(app.prod_office_focused, live)
            .with_tick(app.anim_tick)
            .with_memo(app.prod_office_show_memo)
            .with_help(app.prod_office_show_help);
        f.render_widget(office, chrome[2]);
    } else if tab_id == Some("pantheon") {
        // Pantheon tab — mission list + war room + live events (JSX PantheonTab 647).
        // Live mission data overlay (#235): name/status/agent_count from
        // /v1/pantheon/missions. Falls back to const MISSIONS catalog.
        let live = prod::PantheonLive {
            missions: app.prod_pantheon_missions.as_deref(),
        };
        let pantheon = prod::PantheonTab::with_live(app.prod_pantheon_selected, live);
        f.render_widget(pantheon, chrome[2]);
    } else {
        // Stub tab — all primaries are real now.
        let (name, glyph) = ("unknown", "?");
        let stub = StubTab { name, glyph };
        f.render_widget(stub, chrome[2]);
    }

    // ProdStatusBar
    let status_bar = ProdStatusBar {
        queue_count: app.prod_queue_count,
        is_streaming: app.prod_stream_state == StreamState::Streaming,
    };
    f.render_widget(status_bar, chrome[3]);
}

/// Render an advanced subview: JSX AdvancedTab detail header (line 1382)
/// + the per-tab subview body (dispatched via prod::advanced_sub::render).
fn render_advanced_detail(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    tab: &AdvTabDef,
    app: &App,
) {
    use ratatui::style::Modifier;

    if area.height < 3 || area.width < 12 {
        return;
    }

    // Header band (3 rows): "← Advanced  /  [GLYPH]  Name  · desc"
    let header_rows = 3u16;
    let hy = area.y + 1;
    let mut hx = area.x + 2;

    // Back affordance
    let back = "← Advanced";
    f.buffer_mut().set_string(
        hx,
        hy,
        back,
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    hx += back.chars().count() as u16 + 2;

    // Separator
    f.buffer_mut()
        .set_string(hx, hy, "/", Style::default().fg(theme::MUTED));
    hx += 2;

    // Glyph badge (colored bg, bg-colored text) — JSX 32x18 box
    let badge = format!(" {} ", tab.glyph);
    f.buffer_mut().set_string(
        hx,
        hy,
        &badge,
        Style::default()
            .fg(theme::BG)
            .bg(tab.color)
            .add_modifier(Modifier::BOLD),
    );
    hx += badge.chars().count() as u16 + 1;

    // Name
    f.buffer_mut().set_string(
        hx,
        hy,
        tab.name,
        Style::default()
            .fg(theme::WHITE)
            .add_modifier(Modifier::BOLD),
    );
    hx += tab.name.chars().count() as u16 + 1;

    // · desc
    let desc = format!("· {}", tab.desc);
    f.buffer_mut()
        .set_string(hx, hy, &desc, Style::default().fg(theme::DIM));

    // Bottom border of the header band
    let border_y = area.y + header_rows - 1;
    for bx in area.x..area.right() {
        f.buffer_mut()[(bx, border_y)]
            .set_symbol("─")
            .set_style(Style::default().fg(theme::MUTED));
    }

    // Subview body below the header
    let body = ratatui::layout::Rect {
        x: area.x,
        y: area.y + header_rows,
        width: area.width,
        height: area.height.saturating_sub(header_rows),
    };
    let live = prod::advanced_sub::AdvancedLive {
        agents: app.prod_agents.as_deref(),
        skills: app.prod_skills.as_deref(),
        mcp: app.prod_mcp.as_deref(),
        tts_providers: app.prod_tts_providers.as_deref(),
        tts_voices: app.prod_tts_voices.as_deref(),
        workflows: app.prod_workflows.as_deref(),
        extensions: app.prod_extensions.as_deref(),
        projects: app.prod_projects.as_deref(),
        nodes: app.prod_nodes.as_deref(),
        spawns: app.prod_spawns.as_deref(),
        vector_stores: app.prod_vector_stores.as_deref(),
        communities: app.prod_communities.as_deref(),
        deploy_targets: app.prod_deploy_targets.as_deref(),
        deploy_history: app.prod_deploy_history.as_deref(),
        deploy_stats: app.prod_deploy_stats.as_ref(),
        economy_wallets: app.prod_economy_wallets.as_deref(),
        economy_txs: app.prod_economy_txs.as_deref(),
    };
    prod::advanced_sub::render(tab.id, body, f.buffer_mut(), &live);
}

/// Launch the standalone TUI with seed/default state (no gateway wiring).
/// Used directly by the `zeus-tui` binary and by `zeus_tui::run()` in Phase 1.
pub fn run_standalone() -> io::Result<()> {
    run_loop(std::sync::Arc::new(std::sync::Mutex::new(App::new())))
}

/// Drive a fully-constructed `App` through the terminal render/event loop.
/// The caller (`run()`) may pre-seed `app` with live gateway data.
pub fn run_loop(app: std::sync::Arc<std::sync::Mutex<App>>) -> io::Result<()> {
    // Restore the terminal on panic so a crash never strands the user in raw
    // mode / alt-screen, and the panic message stays readable + diagnosable.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        original_hook(info);
    }));

    // Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Animation clock: drive ~250ms ticks independent of keypresses so the
    // cursor blinks and frame-cycling widgets animate while the user is idle.
    // The event poll already wakes the loop every ≤100ms; we accumulate elapsed
    // wall-time and fire `tick()` each time it crosses the 250ms threshold.
    const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();

    // Main loop
    loop {
        // Advance the animation clock if a tick interval has elapsed. Done
        // before render so the frame reflects the new phase immediately.
        if last_tick.elapsed() >= TICK_INTERVAL {
            app.lock().unwrap_or_else(|e| e.into_inner()).tick();
            last_tick = std::time::Instant::now();
        }

        // Render under a short-lived lock, released before the event wait so
        // background tasks (status poll, live fetches) can update state.
        {
            let a = app.lock().unwrap_or_else(|e| e.into_inner());
            terminal.draw(|f| frame(f, &a))?;
            if a.should_quit {
                break;
            }
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Global hard-quit: Ctrl+C / Ctrl+Q always exits, on any
                    // screen (raw mode suppresses SIGINT, so we handle it here).
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
                    {
                        break;
                    }
                    app.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .handle_key_mods(key.code, key.modifiers);
                }
                Event::Mouse(mouse) => {
                    app.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .handle_mouse_prod(mouse.kind);
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    crossterm::execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

#[cfg(test)]
mod persist_tests {
    //! Proof tests for the onboarding completion persist path
    //! (`collect_and_persist`). These mutate the process-global `ZEUS_HOME`
    //! env var, so they share a serializing mutex — `cargo test` runs `#[test]`
    //! fns in parallel threads within one binary, and an unguarded env race
    //! would let one test's tempdir leak into another's persist.

    use super::*;
    use std::sync::Mutex;

    // Serializes every test that sets ZEUS_HOME (process-global).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// #309: Re-running onboarding on an existing config must be a no-op for
    /// wizard-owned top-level fields when the user just presses through it.
    #[test]
    fn new_from_disk_hydrates_existing_config_before_persist() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        let home = dirs::home_dir().expect("home dir");
        let workspace = home.join(".zeus/workspace-real");
        let sessions = home.join(".zeus/sessions-real");
        let seeded = format!(
            r#"model = "sakana/fugu-ultra"
onboarding_complete = false
workspace = "{}"
sessions = "{}"
thinking_level = "xhigh"
enabled_skills = ["custom-alpha", "custom-beta"]
persona = "Innovator"

[agent]
name = "zeus-titan"
persona = "Innovator"
role = "Forensics lead"

[gateway]
host = "192.0.2.55"
port = 9099
"#,
            workspace.display(),
            sessions.display()
        );
        std::fs::write(tmp.path().join("config.toml"), seeded).unwrap();

        let app = App::new_from_disk();
        let cfg = app.collect_and_persist().expect("persist should succeed");

        assert_eq!(cfg.model, "sakana/fugu-ultra");
        assert_eq!(cfg.thinking_level.as_deref(), Some("xhigh"));
        assert_eq!(cfg.enabled_skills, vec!["custom-alpha", "custom-beta"]);
        assert_eq!(cfg.workspace, workspace);
        assert_eq!(cfg.sessions, sessions);
        assert_eq!(cfg.persona.as_deref(), Some("Innovator"));
        let agent = cfg.agent.as_ref().expect("[agent] must survive");
        assert_eq!(agent.name.as_deref(), Some("zeus-titan"));
        assert_eq!(agent.persona.as_deref(), Some("Innovator"));
        assert_eq!(agent.role.as_deref(), Some("Forensics lead"));
        let gateway = cfg.gateway.as_ref().expect("[gateway] must survive");
        assert_eq!(gateway.host, "192.0.2.55");
        assert_eq!(gateway.port, 9099);

        let saved = zeus_core::Config::load_from(tmp.path().join("config.toml"))
            .expect("saved config should reload");
        assert_eq!(saved.model, "sakana/fugu-ultra");
        assert_eq!(
            saved.agent.and_then(|a| a.name).as_deref(),
            Some("zeus-titan")
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// #309 defense-in-depth: if persist is invoked with a fresh/default wizard
    /// while a real config already has `[agent].name`, do not overwrite it with
    /// the host-derived suggestion.
    #[test]
    fn persist_preserves_existing_agent_name_when_wizard_name_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        std::fs::write(
            tmp.path().join("config.toml"),
            r#"model = "anthropic/claude-opus-4-8"
onboarding_complete = false

[agent]
name = "zeus-real"
persona = "Innovator"
"#,
        )
        .unwrap();

        let app = App::new();
        assert!(
            app.agent_screen.name.is_empty(),
            "fresh wizard has no explicit name; summary_name would be suggested host name"
        );
        let cfg = app.collect_and_persist().expect("persist should succeed");
        assert_eq!(cfg.agent.and_then(|a| a.name).as_deref(), Some("zeus-real"));

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// 🔴 SACRED no-clobber guard at the config.toml layer: a pre-existing
    /// non-onboarding section (`[council]`) must survive the persist round-trip.
    /// Mirrors the 8 `.env`-merge tests that pin the no-clobber guarantee at the
    /// `.env` layer — this pins it at the config.toml layer. If
    /// `collect_and_persist` ever rebuilt from `Config::default()` instead of
    /// `Config::load()`, this section would vanish and this test would fail.
    #[test]
    fn council_section_survives_persist_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        // Seed a config.toml with a hand-set [council] section + a valid model
        // so Config::load() returns a real (non-default) config. The [council]
        // sub-keys use the REAL `CouncilCoreConfig` field names (enabled/models/
        // chairman) — a known Config field, so it round-trips through the struct
        // that `save_unchecked()` serializes.
        let seeded = "model = \"anthropic/claude-opus-4-8\"\n\
                      onboarding_complete = false\n\n\
                      [council]\n\
                      enabled = true\n\
                      models = [\"anthropic/claude-sonnet-4-6\", \"openai/gpt-4o\"]\n\
                      chairman = \"anthropic/claude-opus-4-8\"\n";
        std::fs::write(tmp.path().join("config.toml"), seeded).unwrap();

        let app = App::new();
        let cfg = app.collect_and_persist().expect("persist should succeed");

        // The returned in-memory config carries the onboarding flip AND the
        // pre-existing [council] section (not clobbered by the load→mutate→save).
        assert!(cfg.onboarding_complete, "onboarding_complete must be set");
        let council = cfg.council.as_ref().expect("[council] must survive load");
        assert!(council.enabled, "[council].enabled must survive");
        assert_eq!(
            council.chairman, "anthropic/claude-opus-4-8",
            "[council].chairman must survive intact"
        );

        // The on-disk config.toml must still contain the [council] section
        // intact (no clobber through the persist round-trip).
        let written = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            written.contains("[council]"),
            "[council] section must survive the persist round-trip; got:\n{written}"
        );
        assert!(
            written.contains("anthropic/claude-sonnet-4-6") && written.contains("openai/gpt-4o"),
            "[council] models must survive intact; got:\n{written}"
        );
        assert!(
            written.contains("onboarding_complete = true"),
            "persisted config must mark onboarding complete; got:\n{written}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// #290: a fresh onboarding config has no `[gateway]` section yet, so
    /// collecting gateway host/port must create one instead of silently dropping
    /// the user's custom endpoint.
    #[test]
    fn gateway_host_port_persist_when_section_absent() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\n",
        )
        .unwrap();

        let mut app = App::new();
        app.gateway_screen.host = "0.0.0.0".to_string();
        app.gateway_screen.port = "9099".to_string();

        let cfg = app.collect_and_persist().expect("persist should succeed");
        let gateway = cfg.gateway.as_ref().expect("[gateway] must be created");
        assert_eq!(gateway.host, "0.0.0.0");
        assert_eq!(gateway.port, 9099);

        let written = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            written.contains("[gateway]"),
            "persisted config must include [gateway]; got:\n{written}"
        );
        assert!(
            written.contains("host = \"0.0.0.0\""),
            "persisted config must include custom gateway host; got:\n{written}"
        );
        assert!(
            written.contains("port = 9099"),
            "persisted config must include custom gateway port; got:\n{written}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// P0 #185: a plain provider API key lands in config.toml `[credentials]`
    /// (the single source of truth the S70 bridge reads), NOT `.env`. A
    /// pre-existing `.env` is left completely untouched — config.toml is the
    /// only source, no mirror.
    #[test]
    fn plain_api_key_persists_to_config_credentials_not_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        // Pre-existing .env with a Discord token that must NOT be touched.
        std::fs::write(
            tmp.path().join(".env"),
            "DISCORD_BOT_TOKEN=keep-me-secret\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\n",
        )
        .unwrap();

        let mut app = App::new();
        app.provider_selected = 0; // Anthropic
        app.auth_api_key = "sk-ant-realkey-123".to_string();
        app.collect_and_persist().expect("persist should succeed");

        // Key lands in config.toml [credentials], keyed by env_key.
        let toml = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            toml.contains("ANTHROPIC_API_KEY = \"sk-ant-realkey-123\""),
            "plain key must persist to config.toml [credentials]; got:\n{toml}"
        );

        // .env is untouched — pre-existing secret intact, no key mirrored.
        let env = std::fs::read_to_string(tmp.path().join(".env")).unwrap();
        assert!(
            env.contains("DISCORD_BOT_TOKEN=keep-me-secret"),
            "pre-existing .env secret must be untouched; got:\n{env}"
        );
        assert!(
            !env.contains("ANTHROPIC_API_KEY"),
            "no .env mirror — config.toml is the single source; got:\n{env}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// P0 #185: an OAuth setup-token (`sk-ant-oat…`) routes to
    /// `[provider_credentials.anthropic]` with `cred_type="oauth"` — the only
    /// onboarding-reachable store the gateway resolves as OAuth (Bearer auth).
    /// It must NOT land in `[credentials]` (that path sends x-api-key → 401).
    #[test]
    fn oauth_token_routes_to_provider_credentials_not_credentials() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\n",
        )
        .unwrap();

        let mut app = App::new();
        app.provider_selected = 0; // Anthropic
        app.auth_api_key = "sk-ant-oat01-realoauthtoken-456".to_string();
        app.collect_and_persist().expect("persist should succeed");

        let toml = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            toml.contains("[provider_credentials.anthropic]"),
            "OAuth token must create [provider_credentials.anthropic]; got:\n{toml}"
        );
        assert!(
            toml.contains("cred_type = \"oauth\""),
            "OAuth credential must set cred_type=oauth; got:\n{toml}"
        );
        assert!(
            toml.contains("sk-ant-oat01-realoauthtoken-456"),
            "OAuth token value must be persisted; got:\n{toml}"
        );
        assert!(
            !toml.contains("ANTHROPIC_API_KEY"),
            "OAuth token must NOT go into [credentials] (→ x-api-key 401); got:\n{toml}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// MiniMax appears in the Auth screen as a normal API-key provider
    /// (`sk-api-...`). It must therefore persist to the canonical `[credentials]`
    /// env-key map too; an empty `Provider::Minimax.env_key()` silently dropped
    /// this value in TUI onboarding.
    #[test]
    fn minimax_plain_api_key_persists_to_config_credentials() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\n",
        )
        .unwrap();

        let mut app = App::new();
        app.provider_selected = 8; // MiniMax in screens/providers.rs
        app.model_screen.set_provider("minimax");
        app.auth_api_key = "sk-api-minimax-realkey-123".to_string();
        app.collect_and_persist().expect("persist should succeed");

        let toml = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            toml.contains("model = \"minimax/MiniMax-M3\""),
            "selected MiniMax provider/model must persist; got:\n{toml}"
        );
        assert!(
            toml.contains("MINIMAX_API_KEY = \"sk-api-minimax-realkey-123\""),
            "MiniMax API key must persist under [credentials] MINIMAX_API_KEY; got:\n{toml}"
        );
        assert!(
            !toml.contains("[provider_credentials.minimax]"),
            "plain MiniMax API keys must not be stored as OAuth/provider credentials; got:\n{toml}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// #257 OAuth-edge: a NON-Anthropic OAuth token (selected via auth_mode =
    /// Setup Token / Browser OAuth) must route to `[provider_credentials.{id}]`
    /// cred_type="oauth" — NOT get silently dropped. Drives all four OAuth-edge
    /// providers (gemini-cli, qwen, minimax, xiaomimimo) through the real
    /// persist path AND the read-side `from_config` resolve, proving end-to-end.
    ///
    /// The bug this guards: the old persist trigger was `key.starts_with(
    /// "sk-ant-oat")` — Anthropic-specific. Non-Anthropic OAuth tokens failed
    /// that test → fell into the `[credentials]` api-key branch keyed by
    /// env_key(). For OAuth-only providers with an empty env_key this silently
    /// dropped the token; for API-key providers it wrote the token to the wrong
    /// auth path.
    #[test]
    fn non_anthropic_oauth_routes_to_provider_credentials_not_dropped() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // (provider-screen index, canonical section id, the provider's env_key.
        //  Indices match screens/providers.rs PROVIDERS order: gemini-cli=4,
        //  qwen=7, minimax=8, mimo=9. collect_and_persist canonicalizes the
        //  screen id via from_prefix — gemini-cli must resolve, not default to
        //  Anthropic.)
        // section = the SERDE rename used in [provider_credentials.{section}]
        // (hyphenated for gemini-cli per #[serde(rename="google-gemini-cli")]),
        // NOT the Rust field name.
        let cases = [
            (4usize, "google-gemini-cli", ""),
            (7, "qwen", "QWEN_API_KEY"),
            (8, "minimax", "MINIMAX_API_KEY"),
            (9, "xiaomimimo", "XIAOMIMIMO_API_KEY"),
        ];

        for (provider_idx, section, env_key) in cases {
            let tmp = tempfile::tempdir().unwrap();
            unsafe {
                std::env::set_var("ZEUS_HOME", tmp.path());
            }
            std::fs::write(
                tmp.path().join("config.toml"),
                "model = \"anthropic/claude-opus-4-8\"\n",
            )
            .unwrap();

            let mut app = App::new();
            // Select the provider via the real onboarding field — cfg.model is
            // rebuilt from provider_selected (+model_screen), NOT the file.
            app.provider_selected = provider_idx;
            // Setup-Token mode (1) — explicit OAuth choice. The token here does
            // NOT start with sk-ant-oat, proving routing is auth_mode-driven.
            app.auth_mode = 1;
            let token = format!("oauth-token-for-{section}-xyz");
            app.auth_api_key = token.clone();
            app.collect_and_persist().expect("persist should succeed");

            // ── persist proof: lands in [provider_credentials.{section}] oauth ──
            let toml = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
            assert!(
                toml.contains(&format!("[provider_credentials.{section}]")),
                "provider[{provider_idx}] {section}: OAuth must create [provider_credentials.{section}]; got:\n{toml}"
            );
            assert!(
                toml.contains("cred_type = \"oauth\""),
                "provider[{provider_idx}] {section}: must set cred_type=oauth; got:\n{toml}"
            );
            assert!(
                toml.contains(&token),
                "provider[{provider_idx}] {section}: OAuth token must be persisted (NOT dropped); got:\n{toml}"
            );
            // It must NOT land in [credentials] under the (possibly empty)
            // env_key — that path sends x-api-key, wrong for an OAuth token.
            if !env_key.is_empty() {
                assert!(
                    !toml.contains(&format!("{env_key} =")),
                    "provider[{provider_idx}] {section}: OAuth token must NOT go into [credentials]; got:\n{toml}"
                );
            }

            // ── read proof: from_config resolves it to AuthMethod::OAuth ──
            // from_config(&Config) derives the provider internally from
            // config.model, then runs the same branch-4 read the gateway uses.
            let cfg = zeus_core::Config::load_from(tmp.path().join("config.toml"))
                .expect("reload persisted config");
            let client =
                zeus_llm::LlmClient::from_config(&cfg).expect("from_config should build a client");
            assert!(
                matches!(client.auth_method(), zeus_llm::AuthMethod::OAuth(t) if *t == token),
                "provider[{provider_idx}] {section}: read-side must resolve [provider_credentials.{section}] \
                 cred_type=oauth → AuthMethod::OAuth (Bearer), got {:?}",
                client.auth_method()
            );

            unsafe {
                std::env::remove_var("ZEUS_HOME");
            }
        }
    }

    /// An empty API key must NOT write an empty key line to `.env` (no
    /// `ANTHROPIC_API_KEY=` clobber when the user skipped auth entry).
    #[test]
    fn empty_api_key_does_not_touch_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }

        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\n",
        )
        .unwrap();

        let app = App::new(); // auth_api_key defaults to empty
        app.collect_and_persist().expect("persist should succeed");

        // No .env should be created (or if one exists, no empty key line).
        let env_path = tmp.path().join(".env");
        if env_path.exists() {
            let env = std::fs::read_to_string(&env_path).unwrap();
            assert!(
                !env.contains("ANTHROPIC_API_KEY="),
                "empty key must not write a key line; got:\n{env}"
            );
        }

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// Completion sites surface a persist error instead of silently
    /// transitioning: when `set_persist_error` is called, the Complete screen
    /// records it and `onboarding_complete` stays false.
    #[test]
    fn persist_error_blocks_completion() {
        // `App::new()` is PURE (no disk read) → onboarding_complete starts false
        // unconditionally, with no ZEUS_HOME isolation needed. (The startup
        // disk-read lives in `new_from_disk()`, covered by onb_startup_routing.)
        let mut app = App::new();
        assert!(!app.onboarding_complete);
        app.complete_screen
            .set_persist_error("disk full".to_string());
        assert_eq!(
            app.complete_screen.persist_error.as_deref(),
            Some("disk full")
        );
        assert!(
            !app.onboarding_complete,
            "a persist error must not flip onboarding_complete"
        );
    }

    /// The animation clock starts at 0, advances monotonically per `tick()`,
    /// and drives a 2-phase cursor blink. Pins the tick → blink contract that
    /// `run_loop` and every cursor renderer depend on.
    #[test]
    fn anim_tick_advances_and_drives_blink() {
        let mut app = App::new();
        assert_eq!(app.anim_tick, 0, "clock starts at 0");
        assert!(app.cursor_visible(), "phase 0 → caret visible");

        app.tick();
        assert_eq!(app.anim_tick, 1);
        assert!(!app.cursor_visible(), "phase 1 → caret hidden");

        app.tick();
        assert_eq!(app.anim_tick, 2);
        assert!(
            app.cursor_visible(),
            "phase 2 → caret visible again (2Hz blink)"
        );
    }

    /// `tick()` must never panic at the u64 boundary — the loop runs unattended
    /// for long sessions, so the clock wraps instead of overflowing.
    #[test]
    fn anim_tick_wraps_without_panic() {
        let mut app = App::new();
        app.anim_tick = u64::MAX;
        app.tick();
        assert_eq!(app.anim_tick, 0, "wraps cleanly at the u64 boundary");
    }

    /// AWAKEN handoff contract: entering `launching` shows the splash (NOT the
    /// prod UI yet), and the tick clock completes the transition only after the
    /// dwell — so pressing AWAKEN visibly DOES something before prod takes over.
    #[test]
    fn awaken_launching_handoff_dwells_then_completes() {
        let mut app = App::new();
        // Simulate AWAKEN firing post-persist.
        app.launching = true;
        app.launch_started_tick = app.anim_tick;
        assert!(app.is_launching(), "splash shows immediately on AWAKEN");
        assert!(
            !app.onboarding_complete,
            "prod UI must NOT take over during the handoff dwell"
        );

        // Tick up to (but not past) the dwell — still launching.
        for _ in 0..(LAUNCH_DWELL_TICKS - 1) {
            app.tick();
        }
        assert!(app.is_launching(), "still on splash before dwell elapses");
        assert!(!app.onboarding_complete);

        // The dwell-crossing tick completes the handoff.
        app.tick();
        assert!(!app.is_launching(), "splash clears after dwell");
        assert!(
            app.onboarding_complete,
            "prod UI takes over once the dwell elapses"
        );
    }

    /// The handoff dwell math must survive the u64 wrap — `launch_started_tick`
    /// captured near u64::MAX must still complete via `wrapping_sub`.
    #[test]
    fn awaken_handoff_survives_tick_wrap() {
        let mut app = App::new();
        app.anim_tick = u64::MAX - 2;
        app.launching = true;
        app.launch_started_tick = app.anim_tick;
        // Tick across the wrap boundary through the full dwell.
        for _ in 0..LAUNCH_DWELL_TICKS {
            app.tick();
        }
        assert!(
            app.onboarding_complete,
            "handoff completes correctly even across the u64 wrap"
        );
    }

    // AWAKEN-B interaction seam: a no-launch probe the test swaps in for the
    // real detached spawn, so we can assert the dwell-flip path actually
    // invokes the gateway spawn without forking a real `zeus gateway`.
    static AWAKEN_PROBE_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    fn awaken_probe() {
        AWAKEN_PROBE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// AWAKEN-B (the bug that slipped: no test asserted the flip fired the
    /// spawn). The tick-driven dwell-flip must invoke the gateway-spawn seam
    /// exactly once as it transitions `launching → onboarding_complete` — that
    /// is what brings the titan live for the in-process prod UI. Drives the
    /// real `tick()` flip path with a no-launch probe swapped into the seam.
    #[test]
    fn awaken_dwell_flip_invokes_gateway_spawn_once() {
        AWAKEN_PROBE_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        let mut app = App::new();
        app.awaken_spawn = awaken_probe;
        app.launching = true;
        app.launch_started_tick = app.anim_tick;
        // Before the dwell elapses, the spawn must NOT have fired.
        app.tick();
        assert_eq!(
            AWAKEN_PROBE_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "spawn must not fire before the dwell completes"
        );
        // Tick through the remaining dwell to cross the flip.
        for _ in 0..LAUNCH_DWELL_TICKS {
            app.tick();
        }
        assert!(app.onboarding_complete, "flip must complete the handoff");
        assert_eq!(
            AWAKEN_PROBE_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the dwell-flip must invoke the gateway spawn exactly once"
        );
        // Further ticks must not re-fire (the once-latch holds).
        for _ in 0..LAUNCH_DWELL_TICKS {
            app.tick();
        }
        assert_eq!(
            AWAKEN_PROBE_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "spawn must fire exactly once, never re-fire on subsequent ticks"
        );
    }

    /// #240: the Auth step must block advance on an invalid key format. A bare
    /// `"asdf"` (no `sk-ant-` prefix) advanced today; it must not. Drives the
    /// universal `advance_step()` path (Ctrl+N / footer NEXT) which mirrors the
    /// Enter arm — both honor `auth_key_valid()`.
    #[test]
    fn auth_invalid_key_blocks_advance() {
        let mut app = App::new();
        app.provider_selected = 0; // Anthropic → key_fmt `sk-ant-...`
        app.current_step = Step::Auth as usize;

        // Garbage key → invalid → advance is a no-op.
        app.auth_api_key = "asdf".to_string();
        assert!(!app.auth_key_valid(), "`asdf` must not validate");
        app.advance_step();
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "invalid key must NOT advance past Auth"
        );

        // Empty key → invalid → still blocked.
        app.auth_api_key.clear();
        app.advance_step();
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "empty key must NOT advance past Auth"
        );

        // Valid prefix → advances.
        app.auth_api_key = "sk-ant-realkey-123".to_string();
        assert!(app.auth_key_valid(), "valid prefix must validate");
        app.advance_step();
        assert_eq!(
            app.current_step,
            Step::Auth as usize + 1,
            "valid key must advance past Auth"
        );
    }

    // ---- #239/#240: fetch-IS-the-validation advance gate ----
    //
    // With a live `fetch_tx` wired (the integrated path), advancing on Auth
    // fires the fetch and WAITS — it does NOT advance until the worker writes
    // `Done`. These tests pin that state machine. (The None/standalone path is
    // covered by `auth_invalid_key_blocks_advance` above — it falls through to
    // the static-list fallback advance.)

    use crate::model_fetch::ModelFetchState;

    /// Wire a dummy fetch_tx and park the App on a valid Auth key.
    fn app_on_auth_with_fetch() -> (App, tokio::sync::mpsc::UnboundedReceiver<(String, String)>) {
        let mut app = App::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        app.fetch_tx = Some(tx);
        app.current_step = Step::Auth as usize;
        app.set_auth_api_key("sk-ant-realkey-123");
        (app, rx)
    }

    #[test]
    fn auth_advance_fires_fetch_without_advancing() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        assert_eq!(app.model_fetch_state, ModelFetchState::Idle);
        app.advance_step();
        // Fired the fetch (provider_id, key on the channel) …
        let sent = rx.try_recv().expect("advance must send a fetch request");
        assert_eq!(sent.1, "sk-ant-realkey-123");
        // … set Fetching …
        assert_eq!(app.model_fetch_state, ModelFetchState::Fetching);
        // … and did NOT advance (waiting on the result).
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "firing the fetch must not advance past Auth"
        );
    }

    #[test]
    fn auth_advance_on_done_advances() {
        let (mut app, _rx) = app_on_auth_with_fetch();
        app.model_fetch_state = ModelFetchState::Done(vec!["m1".into()]);
        app.advance_step();
        assert_eq!(
            app.current_step,
            Step::Auth as usize + 1,
            "Done must advance past Auth"
        );
    }

    #[test]
    fn auth_enter_fires_fetch_without_advancing() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        app.handle_key(KeyCode::Enter);
        let sent = rx.try_recv().expect("Auth Enter must send a fetch request");
        assert_eq!(sent.1, "sk-ant-realkey-123");
        assert_eq!(app.model_fetch_state, ModelFetchState::Fetching);
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "Auth Enter must wait for the probe result before leaving Auth"
        );
    }

    #[test]
    fn auth_right_arrow_fires_fetch_without_advancing() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        app.handle_key(KeyCode::Right);
        let sent = rx
            .try_recv()
            .expect("Auth Right-arrow fallback must send a fetch request");
        assert_eq!(sent.1, "sk-ant-realkey-123");
        assert_eq!(app.model_fetch_state, ModelFetchState::Fetching);
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "Auth Right-arrow must wait for the probe result before leaving Auth"
        );
    }

    #[test]
    fn auth_advance_while_fetching_is_noop() {
        let (mut app, _rx) = app_on_auth_with_fetch();
        app.model_fetch_state = ModelFetchState::Fetching;
        app.advance_step();
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "advance while in-flight must be a no-op"
        );
    }

    #[test]
    fn auth_failed_blocks_but_allows_retry() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        app.model_fetch_state = ModelFetchState::Failed("401".into());
        // Failed must not advance …
        // (retry re-fires: Failed is handled like Idle → fires + Fetching).
        app.advance_step();
        let sent = rx
            .try_recv()
            .expect("Failed advance must re-fire the fetch (retry)");
        assert_eq!(sent.1, "sk-ant-realkey-123");
        assert_eq!(app.model_fetch_state, ModelFetchState::Fetching);
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "Failed must block advance (retry, not skip)"
        );
    }

    #[test]
    fn auth_format_precheck_no_fire_on_invalid() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        app.set_auth_api_key("asdf"); // fails the format pre-check
        app.advance_step();
        assert!(
            rx.try_recv().is_err(),
            "invalid key must NOT fire a doomed fetch"
        );
        assert_eq!(app.model_fetch_state, ModelFetchState::Idle);
        assert_eq!(
            app.current_step,
            Step::Auth as usize,
            "invalid key must block advance"
        );
    }

    /// 🔴 #286 regression: MiniMax keys start with `sk-api-` prefix.
    /// Validates that a key with the correct prefix passes the format precheck.
    #[test]
    fn minimax_skapi_key_passes() {
        let (mut app, mut rx) = app_on_auth_with_fetch();
        app.provider_selected = 8; // MiniMax (key_fmt now `sk-api-...`)
        // Fake sample key with the correct `sk-api-` prefix (NOT a real key).
        app.set_auth_api_key("sk-api-TESTtoken123");
        app.advance_step();
        assert!(
            rx.try_recv().is_ok(),
            "valid MiniMax `sk-api-` key must PASS the format pre-check and fire the fetch"
        );
    }

    /// 🔴 chat-input key-capture guard (#TUI key bug, merakizzz live-found):
    /// On the chat tab (tab 0, no advanced overlay) every printable char —
    /// INCLUDING the bare-letter shortcuts q/c/f — must insert into
    /// `prod_chat_input`, not fire its shortcut. Regression for the
    /// `discord`→`disord` swallow + the worse `q`→quit-from-chat.

    #[test]
    fn prod_office_tab_exits_to_next_primary_tab_on_first_tab() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = PRIMARY_TABS
            .iter()
            .position(|tab| tab.id == "office")
            .expect("office tab exists");
        app.prod_active_adv = None;
        app.prod_office_focused = None;

        app.handle_key_prod(KeyCode::Tab);

        assert_eq!(app.prod_office_focused, None);
        assert_eq!(PRIMARY_TABS[app.prod_active_tab].id, "pantheon");
    }

    #[test]
    fn prod_office_tab_from_internal_focus_advances_to_next_primary_tab() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = PRIMARY_TABS
            .iter()
            .position(|tab| tab.id == "office")
            .expect("office tab exists");
        app.prod_active_adv = None;
        app.prod_office_focused = Some(0);

        app.handle_key_prod(KeyCode::Tab);

        assert_eq!(app.prod_office_focused, None);
        assert_eq!(PRIMARY_TABS[app.prod_active_tab].id, "pantheon");
    }

    #[test]
    fn prod_primary_tabs_follow_left_right_outside_advanced_overlay() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = PRIMARY_TABS
            .iter()
            .position(|tab| tab.id == "office")
            .expect("office tab exists");
        app.prod_active_adv = None;
        app.prod_adv_detail = None;

        app.handle_key_prod(KeyCode::Right);
        assert_eq!(PRIMARY_TABS[app.prod_active_tab].id, "pantheon");

        app.handle_key_prod(KeyCode::Left);
        assert_eq!(PRIMARY_TABS[app.prod_active_tab].id, "office");
    }

    #[test]
    fn chat_tab_captures_shortcut_letters_into_input() {
        let mut app = App::new();
        // Production interface, chat tab focused (the default new() state, made
        // explicit): onboarding done, tab 0, no advanced overlay.
        app.onboarding_complete = true;
        app.prod_active_tab = 0;
        app.prod_active_adv = None;

        // "discord" exercises the 'c' swallow (390) mid-word; the trailing
        // c/f/q are the three previously-unconditional bare-letter arms.
        for ch in "discord".chars() {
            app.handle_key_prod(KeyCode::Char(ch));
        }
        app.handle_key_prod(KeyCode::Char('c'));
        app.handle_key_prod(KeyCode::Char('f'));
        app.handle_key_prod(KeyCode::Char('q'));

        assert_eq!(
            app.prod_chat_input, "discordcfq",
            "every printable char (incl. q/c/f) must land in the chat input on tab 0"
        );
        assert!(
            !app.should_quit,
            "typing 'q' in the chat box must NOT quit the app"
        );
    }

    #[test]
    fn chat_scroll_page_keys_and_escape_restore_bottom() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = 0;
        app.prod_chat_messages.push(ChatMessage {
            role: Role::Assistant,
            text: (0..80).map(|i| format!("line{i}\n")).collect::<String>(),
            tool_name: None,
        });

        app.handle_key_prod(KeyCode::PageUp);
        assert_eq!(app.prod_chat_scroll, 10);

        app.handle_key_prod(KeyCode::PageDown);
        assert_eq!(app.prod_chat_scroll, 0);

        app.handle_key_prod(KeyCode::PageUp);
        app.handle_key_prod(KeyCode::Esc);
        assert_eq!(
            app.prod_chat_scroll, 0,
            "Esc on the scrolled chat tab should jump back to the live bottom"
        );
    }

    #[test]
    fn chat_mouse_wheel_scrolls_chat_only() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = 0;
        app.prod_active_adv = None;
        app.prod_chat_messages.push(ChatMessage {
            role: Role::Assistant,
            text: (0..80).map(|i| format!("line{i}\n")).collect::<String>(),
            tool_name: None,
        });

        app.handle_mouse_prod(MouseEventKind::ScrollUp);
        assert_eq!(app.prod_chat_scroll, 3);

        app.handle_mouse_prod(MouseEventKind::ScrollDown);
        assert_eq!(app.prod_chat_scroll, 0);

        app.prod_active_tab = 1;
        app.handle_mouse_prod(MouseEventKind::ScrollUp);
        assert_eq!(
            app.prod_chat_scroll, 0,
            "wheel scrolling is chat-tab only for now"
        );
    }

    /// The over-shadow guard's INVERSE: off the chat tab (tab != 0), the bare
    /// shortcuts must STILL fire — gating them must not break the shortcuts
    /// where they're meant to work. 'q' quits from a non-chat tab.
    #[test]
    fn chat_enter_immediately_echoes_user_and_shows_pending_feedback() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = 0; // chat
        app.prod_chat_input = "ship it".to_string();

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.prod_chat_input, "", "Enter clears the composer");
        assert_eq!(app.prod_chat_scroll, 0, "send resumes bottom-follow");
        assert_eq!(app.prod_stream_state, StreamState::Idle);
        assert_eq!(app.prod_chat_messages.len(), 2);
        assert_eq!(app.prod_chat_messages[0].role, Role::User);
        assert_eq!(app.prod_chat_messages[0].text, "ship it");
        assert!(
            app.prod_chat_messages[1].text.contains("not connected"),
            "standalone fallback still replies after the immediate user echo"
        );
    }

    #[test]
    fn chat_send_feedback_helper_echoes_user_before_async_reply() {
        let mut app = App::new();
        app.prod_chat_scroll = 7;
        app.prod_iter_count = 3;
        app.prod_stream_state = StreamState::Streaming;

        app.push_user_send_feedback("second fast send".to_string());

        assert_eq!(app.prod_chat_messages.len(), 1);
        assert_eq!(app.prod_chat_messages[0].role, Role::User);
        assert_eq!(app.prod_chat_messages[0].text, "second fast send");
        assert_eq!(app.prod_chat_scroll, 0);
        assert_eq!(app.prod_stream_state, StreamState::Queued);
        assert_eq!(app.prod_iter_count, 0);
        assert!(app.prod_tool_feed.is_empty());
    }

    #[test]
    fn chat_stream_empty_final_reply_does_not_append_model_only_assistant_row() {
        let mut app = App::new();

        app.push_user_send_feedback("why silent?".to_string());
        app.push_stream_thinking("checking tools".to_string());
        app.push_assistant_reply(String::new());

        assert_eq!(app.prod_chat_messages.len(), 1);
        assert_eq!(app.prod_chat_messages[0].role, Role::User);
        assert_eq!(app.prod_stream_state, StreamState::Idle);
        assert!(app.prod_tool_feed.is_empty());
    }

    #[test]
    fn chat_stream_tokens_paint_live_and_coalesce_with_final_reply() {
        let mut app = App::new();

        app.push_user_send_feedback("status".to_string());
        app.push_stream_token("working".to_string());
        app.push_stream_token(" live".to_string());

        assert_eq!(app.prod_stream_state, StreamState::Streaming);
        assert_eq!(app.prod_chat_messages.len(), 2);
        assert_eq!(app.prod_chat_messages[1].role, Role::Assistant);
        assert_eq!(app.prod_chat_messages[1].text, "working live");

        app.push_assistant_reply("working live".to_string());

        assert_eq!(app.prod_stream_state, StreamState::Idle);
        assert_eq!(app.prod_chat_messages.len(), 2);
        assert_eq!(app.prod_chat_messages[1].text, "working live");
    }

    #[test]
    fn chat_stream_continues_same_assistant_draft_after_queued_user_send() {
        let mut app = App::new();

        app.push_user_send_feedback("show table".to_string());
        app.push_stream_token("| Field | Value |\n".to_string());
        app.push_user_send_feedback("next question".to_string());
        app.push_stream_token("| Binary | 0.1.2 |".to_string());

        assert_eq!(app.prod_chat_messages.len(), 3);
        assert_eq!(app.prod_chat_messages[1].role, Role::Assistant);
        assert_eq!(
            app.prod_chat_messages[1].text,
            "| Field | Value |\n| Binary | 0.1.2 |"
        );
        assert_eq!(app.prod_chat_messages[2].role, Role::User);

        app.push_assistant_reply("| Field | Value |\n| Binary | 0.1.2 |".to_string());

        assert_eq!(app.prod_stream_state, StreamState::Idle);
        assert_eq!(app.prod_chat_messages.len(), 3);
        assert_eq!(app.prod_chat_messages[1].role, Role::Assistant);
        assert_eq!(
            app.prod_chat_messages[1].text,
            "| Field | Value |\n| Binary | 0.1.2 |"
        );
        assert_eq!(app.prod_chat_messages[2].role, Role::User);
    }

    #[test]
    fn queued_user_send_preserves_streaming_table_rendering() {
        use ratatui::{Terminal, backend::TestBackend, style::Modifier};

        let mut app = App::new();
        app.push_user_send_feedback("show table".to_string());
        app.push_stream_token("| Field | Value |\n".to_string());
        app.push_user_send_feedback("next question".to_string());
        app.push_stream_token("| Binary | 0.1.2 |".to_string());

        let messages = app.prod_chat_messages.clone();
        let mut term = Terminal::new(TestBackend::new(96, 28)).unwrap();
        term.draw(|f| {
            let chat = ChatTab {
                messages: &messages,
                input: "",
                stream_state: app.prod_stream_state,
                scroll_offset: 0,
                slash_open: false,
                cursor_on: false,
                anim_tick: 0,
                tool_feed: &[],
                iter_count: 0,
                active_tasks: None,
                model_badge: Some("test-model"),
                ctx_percent: 0,
            };
            f.render_widget(chat, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();

        let mut text = String::new();
        let mut field_style = None;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                let symbol = cell.symbol();
                if symbol.starts_with('F') {
                    field_style = Some(cell.style());
                }
                text.push_str(symbol);
            }
            text.push('\n');
        }

        assert!(text.contains("Field"), "table header missing:\n{text}");
        assert!(text.contains("Binary"), "table data missing:\n{text}");
        assert!(
            !text.contains("| Field") && !text.contains("| Binary"),
            "table fell back to raw pipe rows after queued send:\n{text}"
        );
        let style = field_style.expect("table header cell style captured");
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "table header lost markdown/table styling: {style:?}"
        );
    }

    #[test]
    fn chat_stream_tool_and_iter_events_leave_queued_state_so_tool_feed_can_render() {
        let mut app = App::new();

        app.push_user_send_feedback("run tests".to_string());
        assert_eq!(app.prod_stream_state, StreamState::Queued);

        app.push_tool_start("shell".to_string(), "cargo test -p zeus-tui".to_string());

        assert_eq!(app.prod_stream_state, StreamState::Streaming);
        assert_eq!(app.prod_tool_feed.len(), 1);
        assert_eq!(app.prod_tool_feed[0].name, "shell");
        assert!(!app.prod_tool_feed[0].done);
        assert!(app.prod_tool_feed[0].input_summary.contains("cargo test"));

        app.push_iter(3);
        assert_eq!(app.prod_stream_state, StreamState::Streaming);
        assert_eq!(app.prod_iter_count, 3);

        app.push_tool_end("shell".to_string(), "ok".to_string());
        assert_eq!(app.prod_stream_state, StreamState::Streaming);
        assert!(app.prod_tool_feed[0].done);
        assert_eq!(app.prod_tool_feed[0].output_summary, "ok");
    }

    #[test]
    fn non_chat_tab_still_honors_quit_shortcut() {
        let mut app = App::new();
        app.onboarding_complete = true;
        app.prod_active_tab = 1; // any non-chat primary tab
        app.prod_active_adv = None;

        app.handle_key_prod(KeyCode::Char('q'));

        assert!(
            app.should_quit,
            "'q' must still quit when NOT focused in the chat input"
        );
        assert!(
            app.prod_chat_input.is_empty(),
            "no char should leak into chat input off the chat tab"
        );
    }

    /// Onboarding text-field key capture: typing `q` in an onboarding text
    /// input (Auth API key) must APPEND to the field, not quit the app. This is
    /// the onboarding sibling of `chat_tab_captures_shortcut_letters_into_input`
    /// — the bare `Char('q')` quit arm in `handle_key` used to fire first and
    /// swallow `q` in every onboarding text field. Quit stays on Ctrl+C/Ctrl+Q
    /// (event loop) + Esc (step back), so removing the bare-q arm loses nothing.
    #[test]
    fn onboarding_text_field_captures_q_into_input() {
        let mut app = App::new();
        app.onboarding_complete = false;
        // Step 3 = Auth — a text-input step whose catch-all pushes into
        // `auth_api_key`.
        app.current_step = Step::Auth as usize;

        // "sq-key-q" exercises `q` mid-word and trailing; every char (incl. q)
        // must land in the API-key field.
        for ch in "sq-key-q".chars() {
            app.handle_key(KeyCode::Char(ch));
        }

        assert_eq!(
            app.auth_api_key, "sq-key-q",
            "every printable char (incl. 'q') must type into the Auth API-key field"
        );
        assert!(
            !app.should_quit,
            "typing 'q' in an onboarding text field must NOT quit the app"
        );
    }

    /// #270 — dispatcher-level bleed-through guard. Banked gotcha:
    /// `Terminal::draw()` resets its back-buffer every frame, so driving
    /// `frame()` twice through a `TestBackend` CANNOT catch bleed (each draw
    /// starts clean). Real-terminal bleed comes from onboarding screens NOT
    /// filling their whole `body[1]` — they paint cards/panels + `set_string`
    /// individual lines, leaving gap cells — so a prior screen's glyphs survive
    /// where the next screen never writes, and the top-of-`frame()` `bg` paint
    /// only rewrites cell *style*, not the symbol.
    ///
    /// So this test renders the screen widgets directly into ONE persistent
    /// `Buffer` (no `Terminal` reset), mirroring the dispatcher path: render the
    /// long Providers screen, apply the dispatcher's `clear_body_region`, then
    /// render the sparse Welcome screen. The result must equal a fresh Welcome
    /// render. Remove the `clear_body_region` call and Providers' lower-row
    /// glyphs survive Welcome's gaps → buffers diverge. Pins the dispatcher
    /// Clear (dropped silently once in 501c3350).
    #[test]
    fn onboarding_dispatcher_clears_bleed_between_steps() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        fn buffer_text(buf: &Buffer) -> String {
            let area = buf.area();
            let mut out = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    out.push_str(buf[(x, y)].symbol());
                }
                out.push('\n');
            }
            out
        }

        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 36,
        };

        // DIRTY: Providers (long list) -> clear_body_region -> Welcome, all into
        // one persistent buffer (replicating the dispatcher's body[1] path).
        let mut dirty = Buffer::empty(area);
        ProviderScreen { selected: 0 }.render(area, &mut dirty);
        clear_body_region(area, &mut dirty);
        WelcomeScreen {
            existing_config: false,
            anim_tick: 0,
        }
        .render(area, &mut dirty);

        // FRESH: Welcome into a clean buffer.
        let mut fresh = Buffer::empty(area);
        WelcomeScreen {
            existing_config: false,
            anim_tick: 0,
        }
        .render(area, &mut fresh);

        assert_eq!(
            buffer_text(&dirty),
            buffer_text(&fresh),
            "Providers glyphs bled through into the Welcome frame — the \
             dispatcher's clear_body_region on body[1] is missing or was \
             dropped (#270 regression)"
        );
    }

    /// #273: IRC + Matrix relay creds collected in onboarding persist into
    /// `[channels.irc]` / `[channels.matrix]`. The arms were previously deferred
    /// (#247) despite the onboarding fields mapping cleanly onto the structs.
    /// Proves field→struct flow end-to-end through the persist round-trip.
    #[test]
    fn irc_and_matrix_channels_persist_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }
        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = false\n",
        )
        .unwrap();

        let mut app = App::new();
        app.chanconfig_screen.toggled = vec!["irc".to_string(), "matrix".to_string()];
        let cv = &mut app.chanconfig_screen.config_values;
        cv.insert("irc.server".into(), "irc.libera.chat".into());
        cv.insert("irc.port".into(), "6697".into());
        cv.insert("irc.nick".into(), "zeusbot".into());
        cv.insert("irc.channels".into(), "#zeus, #ops".into());
        cv.insert("irc.password".into(), "nickserv-secret".into());
        cv.insert("matrix.homeserver".into(), "https://matrix.org".into());
        cv.insert("matrix.username".into(), "@zeus:matrix.org".into());
        cv.insert("matrix.password".into(), "matrix-secret".into());

        let cfg = app.collect_and_persist().expect("persist should succeed");

        let ch = cfg.channels.as_ref().expect("[channels] must exist");
        let irc = ch.irc.as_ref().expect("[channels.irc] must persist");
        assert_eq!(
            irc.server, "irc.libera.chat",
            "irc.server must flow through"
        );
        assert_eq!(irc.port, 6697, "irc.port must parse + flow through");
        assert_eq!(irc.nick, "zeusbot", "irc.nick must flow through");
        assert_eq!(
            irc.channels,
            vec!["#zeus".to_string(), "#ops".to_string()],
            "irc.channels must split the comma list, trimmed"
        );
        assert!(irc.use_tls, "port 6697 must enable TLS");
        assert_eq!(
            irc.nickserv_password.as_deref(),
            Some("nickserv-secret"),
            "irc.password must map to nickserv_password"
        );

        let matrix = ch.matrix.as_ref().expect("[channels.matrix] must persist");
        assert_eq!(matrix.homeserver, "https://matrix.org");
        assert_eq!(matrix.username.as_deref(), Some("@zeus:matrix.org"));
        assert_eq!(matrix.password.as_deref(), Some("matrix-secret"));

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// Bot-message policy (`allow_bots`) selected in the chanconfig step must be
    /// persisted verbatim into the channel's config section as
    /// `allow_bots = "<choice>"`. Covers both the in-memory struct value and the
    /// serialized `config.toml` text for a bot-capable channel (discord).
    #[test]
    fn allow_bots_policy_persists_verbatim() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }
        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = false\n",
        )
        .unwrap();

        let mut app = App::new();
        app.chanconfig_screen.toggled = vec!["discord".to_string()];
        app.chanconfig_screen
            .config_values
            .insert("discord.token".into(), "tok".into());
        // User cycled the selector from the default ("mentions") to "on".
        app.chanconfig_screen
            .bot_policies
            .insert("discord".into(), "on".into());

        let cfg = app.collect_and_persist().expect("persist should succeed");

        // (1) struct value carries the chosen policy verbatim.
        let ch = cfg.channels.as_ref().expect("[channels] must exist");
        let discord = ch
            .discord
            .as_ref()
            .expect("[channels.discord] must persist");
        assert_eq!(
            discord.allow_bots.as_deref(),
            Some("on"),
            "selected allow_bots policy must persist verbatim onto the struct"
        );

        // (2) serialized config.toml emits `allow_bots = "on"`.
        let written = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            written.contains("allow_bots = \"on\""),
            "config-gen must write `allow_bots = \"on\"` for a bot channel; got:\n{written}"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// The default bot-message policy (when the user never touches the selector)
    /// must be `mentions` — matching the old TUI behavior the rebuild dropped.
    #[test]
    fn allow_bots_policy_defaults_to_mentions() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }
        std::fs::write(
            tmp.path().join("config.toml"),
            "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = false\n",
        )
        .unwrap();

        let mut app = App::new();
        app.chanconfig_screen.toggled = vec!["discord".to_string()];
        app.chanconfig_screen
            .config_values
            .insert("discord.token".into(), "tok".into());
        // Note: no bot_policies entry — the user left the selector untouched.

        let cfg = app.collect_and_persist().expect("persist should succeed");
        let ch = cfg.channels.as_ref().expect("[channels] must exist");
        let discord = ch
            .discord
            .as_ref()
            .expect("[channels.discord] must persist");
        assert_eq!(
            discord.allow_bots.as_deref(),
            Some("mentions"),
            "default allow_bots policy must be `mentions`"
        );

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// X/Twitter onboarding must persist the official X credential names instead
    /// of silently dropping the channel due to the old api_key/api_secret naming.
    #[test]
    fn x_twitter_official_credentials_persist() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }
        std::fs::write(
            tmp.path().join("config.toml"),
            r#"model = "anthropic/claude-sonnet-4"
onboarding_complete = false
"#,
        )
        .unwrap();

        let mut app = App::new();
        app.chanconfig_screen.toggled = vec!["x_twitter".to_string()];
        app.chanconfig_screen
            .config_values
            .insert("x_twitter.bearer_token".into(), "bearer".into());
        app.chanconfig_screen
            .config_values
            .insert("x_twitter.consumer_key".into(), "consumer".into());
        app.chanconfig_screen.config_values.insert(
            "x_twitter.consumer_key_secret".into(),
            "consumer-secret".into(),
        );
        app.chanconfig_screen
            .config_values
            .insert("x_twitter.access_token".into(), "access".into());
        app.chanconfig_screen.config_values.insert(
            "x_twitter.access_token_secret".into(),
            "access-secret".into(),
        );
        app.chanconfig_screen
            .config_values
            .insert("x_twitter.client_id".into(), "client-id".into());
        app.chanconfig_screen
            .config_values
            .insert("x_twitter.client_secret".into(), "client-secret".into());

        let cfg = app.collect_and_persist().expect("persist should succeed");
        let ch = cfg.channels.as_ref().expect("[channels] must exist");
        let x = ch
            .x_twitter
            .as_ref()
            .expect("[channels.x_twitter] must persist");
        assert_eq!(x.bearer_token, "bearer");
        assert_eq!(x.consumer_key, "consumer");
        assert_eq!(x.consumer_key_secret, "consumer-secret");
        assert_eq!(x.access_token, "access");
        assert_eq!(x.access_token_secret, "access-secret");
        assert_eq!(x.client_id, "client-id");
        assert_eq!(x.client_secret, "client-secret");

        let saved = zeus_core::Config::load_from(&tmp.path().join("config.toml"))
            .expect("saved config.toml should reload");
        let saved_x = saved
            .channels
            .as_ref()
            .and_then(|channels| channels.x_twitter.as_ref())
            .expect("saved [channels.x_twitter] must survive disk round-trip");
        assert_eq!(saved_x.bearer_token, "bearer");
        assert_eq!(saved_x.consumer_key, "consumer");
        assert_eq!(saved_x.consumer_key_secret, "consumer-secret");
        assert_eq!(saved_x.access_token, "access");
        assert_eq!(saved_x.access_token_secret, "access-secret");
        assert_eq!(saved_x.client_id, "client-id");
        assert_eq!(saved_x.client_secret, "client-secret");

        unsafe {
            std::env::remove_var("ZEUS_HOME");
        }
    }

    /// #273 registry-completeness guard: EVERY channel id in the onboarding
    /// registry (`chanconfig::channel_registry_ids`) must be CLASSIFIED — either
    /// in the persisted set (has a `collect_and_persist` arm) or in the
    /// explicitly-deferred set (field↔struct mismatch / partial creds). Adding a
    /// new `ChannelDef` without wiring its persist (or deferring it on purpose)
    /// FAILS here — closing the silent-drop gap that left irc/matrix unpersisted
    /// for the whole #247 window. Update the matching set below WITH the arm.
    #[test]
    fn every_registry_channel_is_classified_for_persist() {
        use std::collections::HashSet;

        // Channels with a live persist arm in `collect_and_persist`.
        let persisted: HashSet<&str> =
            ["telegram", "discord", "slack", "irc", "matrix", "x_twitter"]
                .into_iter()
                .collect();
        // Channels intentionally NOT persisted (see the DEFERRED comment block):
        //   email     — partial creds (no imap_*); whatsapp — field↔struct mismatch;
        //   signal/imessage — no struct-matching creds.
        let deferred: HashSet<&str> = ["email", "imessage", "whatsapp", "signal"]
            .into_iter()
            .collect();

        for id in screens::chanconfig::channel_registry_ids() {
            let classified = persisted.contains(id) || deferred.contains(id);
            assert!(
                classified,
                "channel `{id}` is in the onboarding registry but neither \
                 persisted nor explicitly deferred in collect_and_persist — \
                 wire its persist arm or add it to the deferred set (this guard \
                 exists so a new channel can't silently drop, as irc/matrix did \
                 through the whole #247 window)"
            );
        }
    }
}
