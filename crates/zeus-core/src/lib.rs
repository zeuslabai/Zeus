//! Zeus Core - Shared types, errors, and configuration
//!
//! Target: ~300 lines

pub mod cook_state;
pub mod inbox;
pub mod team_memory;
pub mod migration;
pub mod persona;
pub mod sanitize;
pub mod session_lane;
pub mod soul;
pub mod turn_boundary;
pub mod validator;
pub use cook_state::{ActiveCookType, CookFlight, CookGuard, CookState};
pub use session_lane::SessionLaneManager;
pub use soul::{render_soul_md, soul_content_is_stub, soul_is_stub_or_missing, write_onboarding_soul};
pub use persona::{Persona, PersonaRegistry, RouteMatch, RunProfile};
pub use turn_boundary::{
    segment_pending_call_ids, segment_satisfied_call_ids, turn_segment_for_index,
};
pub use validator::{ConfigValidator, Severity, ValidationFinding, ValidationReport};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use thiserror::Error;

// ============================================================================
// TriggerExecutor trait — implemented by zeus-prometheus, consumed by zeus-agent
// ============================================================================

/// Trait for executing trigger tool calls against a live scheduler.
///
/// This trait breaks the dependency cycle between `zeus-agent` and
/// `zeus-prometheus`. The agent holds an `Option<Arc<dyn TriggerExecutor>>`
/// and delegates `create_trigger`, `list_triggers`, and `remove_trigger`
/// calls to it when available.
#[async_trait::async_trait]
pub trait TriggerExecutor: Send + Sync {
    /// Execute a trigger tool by name with the given JSON input.
    async fn execute(&self, tool_name: &str, input: &serde_json::Value) -> Result<String>;
}

/// Structured result from a single agent turn.
/// Replaces raw String returns — gives callers access to tool calls,
/// token usage, iteration count, and stop reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    /// Final assistant response text
    pub content: String,
    /// Tool calls executed during this turn
    pub tool_calls: Vec<ToolCallRecord>,
    /// Total input tokens consumed
    pub input_tokens: usize,
    /// Total output tokens generated
    pub output_tokens: usize,
    /// Number of agent loop iterations
    pub iterations: usize,
    /// How the turn ended
    pub stop_reason: TurnStopReason,
}

/// Record of a single tool call + result within a turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub output: String,
}

/// Why a turn ended
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TurnStopReason {
    /// LLM returned EndTurn stop reason
    EndTurn,
    /// Hit max_iterations limit
    MaxIterations,
    /// Skipped by hook
    Skipped,
    /// Error occurred
    Error(String),
}

impl TurnResult {
    /// Quick access to response text (backward compat)
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Total tokens (input + output)
    pub fn total_tokens(&self) -> usize {
        self.input_tokens + self.output_tokens
    }

    /// Number of tool calls executed
    pub fn tool_count(&self) -> usize {
        self.tool_calls.len()
    }
}

impl From<TurnResult> for String {
    fn from(r: TurnResult) -> String {
        r.content
    }
}

// ============================================================================
// String Utilities
// ============================================================================

/// Find the largest valid UTF-8 char boundary at or before `index`.
/// Polyfill for `str::floor_char_boundary` (unstable before Rust 1.93).
#[inline]
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        let mut i = index;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Strip markdown code block fences from LLM JSON responses.
/// LLMs sometimes wrap JSON in ```json ... ``` blocks which break parsing.
/// Inspired by MiroFish's llm_client.py chat_json cleanup pattern (S59-P7).
pub fn strip_json_markdown(input: &str) -> String {
    let trimmed = input.trim();
    // Remove leading ```json or ``` and trailing ```
    let stripped = if trimmed.starts_with("```json") {
        &trimmed[7..]
    } else if trimmed.starts_with("```") {
        &trimmed[3..]
    } else {
        trimmed
    };
    let stripped = stripped.trim();
    let stripped = if stripped.ends_with("```") {
        &stripped[..stripped.len() - 3]
    } else {
        stripped
    };
    stripped.trim().to_string()
}

// ============================================================================
// Named Constants — eliminates magic numbers across the codebase
// ============================================================================

/// Maximum items per page for API pagination (used across all handlers).
pub const MAX_PAGE_LIMIT: usize = 200;

/// Smaller page limit for high-cardinality endpoints (search, marketplace).
pub const MAX_PAGE_LIMIT_SMALL: usize = 100;

/// Maximum bytes before file/response content is truncated in tool results.
pub const MAX_CONTENT_BYTES: usize = 100_000;

/// Maximum characters for tool result output before capping.
pub const TOOL_RESULT_CAP_CHARS: usize = 50_000;

/// Default max context tokens for overflow management.
pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 200_000;

/// Maximum webhook payload size in bytes (256 KB).
pub const MAX_WEBHOOK_PAYLOAD_BYTES: usize = 262_144;

/// Maximum webhook message length in bytes (50 KB).
pub const MAX_WEBHOOK_MESSAGE_BYTES: usize = 51_200;

/// Maximum WebSocket message size in bytes (1 MB).
pub const MAX_WS_MESSAGE_BYTES: usize = 1_048_576;

/// Maximum inbound API message length in bytes (50 KB).
pub const MAX_INBOUND_MESSAGE_BYTES: usize = 50_000;

// ============================================================================
// Errors (~50 lines)
// ============================================================================

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Skill error: {0}")]
    Skill(String),

    #[error("Security error: {0}")]
    Security(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Structured error macro for tool implementations.
///
/// Routes error messages to the appropriate [`Error`] variant based on category.
/// Replaces raw `Error::Tool(format!(...))` with semantically meaningful variants
/// so callers (retry logic, user-facing surfaces) can discriminate failure types.
///
/// # Categories
/// - `not_found` → `Error::NotFound`   (missing file, missing record)
/// - `validation` → `Error::Validation` (bad input, unsupported arg)
/// - `security` → `Error::Security`    (path traversal, forbidden op)
/// - `timeout` → `Error::Timeout`      (exceeded deadline)
/// - `network` → `Error::Network`      (HTTP, DNS, connect failures)
/// - `io` → `Error::Tool`              (file read/write failure — kept as Tool for now)
/// - `tool` → `Error::Tool`            (fallback / generic tool failure)
///
/// # Examples
/// ```ignore
/// use zeus_core::tool_err;
///
/// return Err(tool_err!(not_found, "file {}", path));
/// return Err(tool_err!(validation, "Unknown tool: {}", name));
/// return Err(tool_err!(timeout, "Command timed out after {}s", secs));
/// return Err(tool_err!(network, "HTTP {} - {}", status, body));
/// return Err(tool_err!(tool, "unexpected: {}", e));
/// ```
#[macro_export]
macro_rules! tool_err {
    (not_found, $($arg:tt)*) => {
        $crate::Error::NotFound(format!($($arg)*))
    };
    (validation, $($arg:tt)*) => {
        $crate::Error::Validation(format!($($arg)*))
    };
    (security, $($arg:tt)*) => {
        $crate::Error::Security(format!($($arg)*))
    };
    (timeout, $($arg:tt)*) => {
        $crate::Error::Timeout(format!($($arg)*))
    };
    (network, $($arg:tt)*) => {
        $crate::Error::Network(format!($($arg)*))
    };
    (io, $($arg:tt)*) => {
        $crate::Error::Tool(format!($($arg)*))
    };
    (tool, $($arg:tt)*) => {
        $crate::Error::Tool(format!($($arg)*))
    };
}

impl Error {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn channel(msg: impl Into<String>) -> Self {
        Self::Channel(msg.into())
    }

    pub fn llm(msg: impl Into<String>) -> Self {
        Self::Llm(msg.into())
    }

    pub fn tool(msg: impl Into<String>) -> Self {
        Self::Tool(msg.into())
    }

    pub fn skill(msg: impl Into<String>) -> Self {
        Self::Skill(msg.into())
    }

    pub fn memory(msg: impl Into<String>) -> Self {
        Self::Memory(msg.into())
    }

    pub fn security(msg: impl Into<String>) -> Self {
        Self::Security(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network(_) | Self::Timeout(_) | Self::RateLimited(_)
        )
    }
}

// ============================================================================
// String Utilities
// ============================================================================

/// Truncate a string at a byte limit, respecting UTF-8 character boundaries.
/// Returns a slice that is at most `max_bytes` long and always valid UTF-8.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ============================================================================
// Configuration (~80 lines)
// ============================================================================

/// Agent verbosity level — controls how chatty the agent is in group channels.
/// Set via `verbosity = "normal"` in config.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    /// Silent mode: only respond when directly @mentioned or reporting commits.
    /// Like a monk who took a vow of silence — speaks only when spoken to.
    Silent,
    /// Normal mode: respond to relevant messages, help when useful, stay on-topic.
    /// Like a professional colleague — engaged but not overbearing.
    #[default]
    Normal,
    /// Barfly mode: chatty, opinionated, interjects freely, full personality.
    /// Like someone three drinks deep at the pub — has thoughts on everything.
    Barfly,
}

impl Verbosity {
    /// Returns system prompt instructions for this verbosity level.
    pub fn system_prompt_instructions(&self) -> &'static str {
        match self {
            Self::Silent => concat!(
                "[Verbosity: SILENT]\n",
                "You are in silent mode. Your default state is QUIET.\n",
                "- Only respond when a human directly @mentions you or addresses you by name\n",
                "- Only respond when reporting completed work (commits, artifacts)\n",
                "- Reply with exactly NO_REPLY to all other messages\n",
                "- Never discuss architecture, plans, or opinions — only ship code\n",
                "- Never respond to other bots under any circumstances\n",
            ),
            Self::Normal => concat!(
                "[Verbosity: NORMAL]\n",
                "You are in normal mode. Be helpful and professional.\n",
                "- Respond to messages directed at you or relevant to your assigned tasks\n",
                "- Keep responses concise and action-oriented\n",
                "- You may offer brief help to teammates if you have useful context\n",
                "- Don't start lengthy debates or architecture discussions unprompted\n",
                "- Don't respond to other bots unless they @mention you with a question\n",
                "- When reporting work, include commit hashes and brief descriptions\n",
            ),
            Self::Barfly => concat!(
                "[Verbosity: BARFLY]\n",
                "You are in barfly mode. Full personality, maximum engagement.\n",
                "- Feel free to chime in on conversations that interest you\n",
                "- Share opinions, crack jokes, and show your personality\n",
                "- Still prioritize shipping code over chatting\n",
                "- Respond to other agents if you have something genuinely useful to add\n",
                "- Use your personality and flair — you're at the pub with friends\n",
                "- Keep it fun but don't derail active work threads\n",
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Silent => "silent",
            Self::Normal => "normal",
            Self::Barfly => "barfly",
        }
    }

    pub fn is_default(&self) -> bool {
        matches!(self, Self::Normal)
    }
}

impl std::fmt::Display for Verbosity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classifies the sender of an inbound channel message.
/// Used to gate bot-to-bot loops at the classification level rather than
/// relying on LLM judgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SenderType {
    /// Message from a verified human user
    Human,
    /// Message from a bot (Discord bot flag, Telegram is_bot, etc.)
    Bot,
    /// System-generated message (webhooks, service notifications)
    System,
    /// Could not determine sender type — treat as human for safety
    #[default]
    Unknown,
}

impl SenderType {
    /// Returns true if the sender is a bot
    pub fn is_bot(&self) -> bool {
        matches!(self, Self::Bot)
    }

    /// Returns true if the sender is a human
    pub fn is_human(&self) -> bool {
        matches!(self, Self::Human)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Bot => "bot",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for SenderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Nested `[agent]` section in config.toml — written by onboarding.
/// Serde maps the TOML table `[agent]` to this struct via the `agent` field on Config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentSection {
    /// Agent display name (e.g. "zeus107", "zeusmarketing")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Selected persona (e.g. "The Builder")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// Agent role (e.g. "Backend developer") — collected at onboarding (#213)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Coordinator agent ID (e.g. "Zeus100") — None for standalone deploys (#213)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Model string: "provider/model" or just "model"
    /// Examples: "ollama/llama3.2", "anthropic/claude-sonnet-4-20250514", "openai/gpt-4o"
    #[serde(default = "default_model", skip_serializing_if = "String::is_empty")]
    pub model: String,

    /// Ordered fallback model list for automatic failover.
    /// Each entry is a "provider/model" string. On primary model failure,
    /// tries these in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_models: Option<Vec<String>>,

    /// Workspace directory for memory files
    #[serde(default = "default_workspace", skip_serializing_if = "is_default_workspace")]
    pub workspace: PathBuf,

    /// Sessions directory for JSONL files
    #[serde(default = "default_sessions", skip_serializing_if = "is_default_sessions")]
    pub sessions: PathBuf,

    /// TUI settings
    #[serde(default, skip_serializing_if = "is_default_tui")]
    pub tui: TuiConfig,

    /// Authentication settings
    #[serde(default, skip_serializing_if = "is_default_auth")]
    pub auth: AuthConfig,

    /// OAuth token storage — legacy single-provider (backward compat)
    #[serde(default, skip_serializing_if = "is_default_oauth")]
    pub oauth: OAuthConfig,

    /// Per-provider credential storage (new format)
    #[serde(default, rename = "provider_credentials", skip_serializing_if = "is_default_credentials")]
    pub provider_credentials: CredentialsConfig,

    /// Ollama settings
    #[serde(default, skip_serializing_if = "is_default_ollama")]
    pub ollama: OllamaConfig,

    /// Maximum agent iterations
    #[serde(default = "default_max_iterations", skip_serializing_if = "is_default_max_iter")]
    pub max_iterations: usize,

    /// Maximum subagent iterations
    #[serde(default = "default_max_subagent_iterations", skip_serializing_if = "is_default_max_subagent_iter")]
    pub max_subagent_iterations: usize,

    /// Suppress detailed tool error messages (show generic error instead)
    #[serde(default, skip_serializing_if = "is_false")]
    pub suppress_tool_errors: bool,

    /// Mnemosyne (advanced memory) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemosyne: Option<MnemosyneConfig>,

    /// Athena (documentation engine) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub athena: Option<AthenaConfig>,

    /// Aegis (security) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aegis: Option<AegisConfig>,

    /// Hermes (notifications) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hermes: Option<HermesConfig>,

    /// Prometheus (orchestration) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prometheus: Option<PrometheusConfig>,

    /// Nous (cognitive engine) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nous: Option<NousConfig>,

    /// Talos (automation tools) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talos: Option<TalosConfig>,

    /// Channels (messaging) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<ChannelsConfig>,

    /// Hooks configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<HooksConfig>,

    /// LLM Council — multi-model deliberation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council: Option<CouncilCoreConfig>,

    /// Search configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchConfig>,

    /// Gateway configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<GatewayConfig>,

    /// Logging configuration (levels, file sink, retention)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingConfig>,

    /// Session compaction configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_compaction: Option<SessionCompactionConfig>,

    /// Session pruning configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruning: Option<PruningConfig>,

    /// Session maintenance configuration (runs on every save)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_maintenance: Option<SessionMaintenanceConfig>,

    /// Thinking level for extended thinking models (low/medium/high/xhigh)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,

    /// Whether onboarding wizard has been completed
    #[serde(default, skip_serializing_if = "is_false")]
    pub onboarding_complete: bool,

    /// Onboarding: subsystem feature toggles
    /// Keys: "nous", "mnemosyne", "prometheus", "browser", "voice",
    ///       "macos", "aegis", "mcp", "skills", "agora"
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub features: std::collections::HashMap<String, bool>,

    /// Onboarding: enabled skill names
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_skills: Vec<String>,

    /// Skill matcher similarity threshold (0.0-1.0, default 0.40)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_matcher_threshold: Option<f32>,

    /// Skill matcher top-K results to return (default 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_matcher_top_k: Option<usize>,

    /// Onboarding: selected persona (top-level key — legacy, prefer `[agent].persona`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,

    /// Agent display name (top-level key — legacy, prefer `[agent].name`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Nested `[agent]` section written by onboarding — canonical name/persona source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentSection>,

    /// Image generation backend configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_gen: Option<ImageGenConfig>,
    /// Video generation backend (ComfyUI + AnimateDiff)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_gen: Option<VideoGenConfig>,

    /// Voice (TTS / STT) backend configuration. Onboarding step 14 writes here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceConfig>,

    /// Context overflow recovery configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<OverflowRecoveryConfig>,

    /// WebSocket Ed25519 authentication configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ws_auth: Option<WsAuthConfig>,

    /// Telegram relay configuration (Bot API polling, simpler than MTProto channels)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_relay: Option<TelegramRelayConfigCore>,

    /// Slack relay configuration (Socket Mode WebSocket, forwards to tmux)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slack_relay: Option<SlackRelayConfigCore>,

    /// Matrix relay configuration (matrix-sdk, dedicated relay)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix_relay: Option<MatrixRelayConfig>,

    /// Signal relay configuration (signal-cli JSON-RPC subprocess)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_relay: Option<SignalRelayConfig>,

    /// Email relay configuration (IMAP polling → agent.run(), dedicated relay)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_relay: Option<EmailRelayConfig>,

    /// MQTT relay configuration (rumqttc subscribe → agent.run(), dedicated relay)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt_relay: Option<MqttRelayConfig>,

    /// WhatsApp relay configuration (Bridge or Cloud API)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whatsapp_relay: Option<WhatsAppRelayConfig>,

    /// Mattermost relay configuration (WebSocket → agent.run(), dedicated relay)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mattermost_relay: Option<MattermostRelayConfig>,

    /// MCP server behavior configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<McpServerConfig>,

    /// Wallet (Ed25519 keypair + x402 payments) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet: Option<WalletCoreConfig>,

    /// Model routing configuration for per-task model selection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_routing: Option<ModelRoutingCoreConfig>,

    /// Agent pool configuration for parallel sub-agent execution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_pool: Option<AgentPoolCoreConfig>,

    /// Network configuration for agent fleet communication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkConfig>,

    /// Deployment configuration for service URLs and external integrations
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<DeploymentConfig>,

    /// Pantheon IRC client configuration — connects to standalone Pantheon server
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pantheon: Option<PantheonClientConfig>,

    /// Star Office Pantheon room configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star_office: Option<StarOfficeConfig>,

    /// Economy configuration for agent earning formula
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub economy: Option<EconomyConfig>,

    /// Agent verbosity level in group channels (silent/normal/barfly).
    /// Default: normal.
    #[serde(default, skip_serializing_if = "Verbosity::is_default")]
    pub verbosity: Verbosity,

    /// Guard flag: true if this config was loaded from Config::default() rather
    /// than from a real config file. Prevents save() from overwriting a real config
    /// with defaults. Not serialized — only exists at runtime.
    #[serde(skip)]
    pub loaded_from_default: bool,

    /// Canonical credential store — single source of truth for all secrets.
    ///
    /// All entries are injected as process env vars at startup, so existing
    /// `std::env::var("KEY")` callsites pick them up automatically.
    /// `.env` file is still loaded for dev overrides but is NOT required.
    ///
    /// ```toml
    /// [credentials]
    /// ANTHROPIC_API_KEY = "sk-ant-api-..."
    /// DISCORD_BOT_TOKEN = "MTQ3..."
    /// OLLAMA_HOST = "https://ollama.example.com"
    /// OPENAI_API_KEY = "sk-..."
    /// ```
    ///
    /// For OAuth tokens, use `credentials.json` (CredentialStore format)
    /// alongside `[auth] use_oauth = true` — the startup code injects the
    /// OAuth token as ANTHROPIC_API_KEY automatically.
    #[serde(default)]
    pub credentials: std::collections::HashMap<String, String>,

    /// Named agent configurations for multi-agent gateway deployments.
    ///
    /// Each entry defines an isolated agent with its own workspace, sessions,
    /// and channel account bindings. The gateway will instantiate one agent
    /// per entry and route inbound messages by `account_id`.
    ///
    /// ```toml
    /// [[agents]]
    /// id = "main"
    ///
    /// [[agents]]
    /// id = "z"
    /// name = "Z"
    /// workspace = "~/.zeus/agents/z/workspace"
    /// sessions = "~/.zeus/agents/z/sessions"
    /// discord_account = "z"
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentConfig>,

    /// Agent routing bindings for fine-grained per-channel/guild/account dispatch.
    ///
    /// Bindings are evaluated before account-based routing. First matching
    /// binding wins. Unset fields are wildcards (match anything).
    ///
    /// ```toml
    /// [[bindings]]
    /// agent_id = "z"
    /// account_id = "z"
    ///
    /// [[bindings]]
    /// agent_id = "support"
    /// channel_id = "1234567890123456789"
    /// guild_id = "9876543210987654321"
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ChannelBinding>,
}

/// Named agent configuration for multi-agent gateway deployments.
///
/// Each entry in `Config.agents` defines an isolated agent.
/// The `id` must be unique and is used to key sessions and route
/// inbound channel messages via `account_id` matching.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Unique agent identifier (e.g. "main", "z").
    /// Used as session key prefix and account_id routing key.
    pub id: String,

    /// Human-readable display name (optional, falls back to id).
    /// Used in identity prefixes and UI labels.
    pub name: Option<String>,

    /// Workspace directory override (overrides global `workspace`).
    /// Defaults to `<global_workspace>/agents/<id>/` when not set.
    pub workspace: Option<PathBuf>,

    /// Sessions directory override (overrides global `sessions`).
    /// Defaults to `<global_sessions>/agents/<id>/` when not set.
    pub sessions: Option<PathBuf>,

    /// Model override for this agent (overrides global `model`).
    /// Format: "provider/model" e.g. "anthropic/claude-sonnet-4-6".
    pub model: Option<String>,

    /// Discord `accounts` key this agent listens on.
    /// Must match a key in `channels.discord.accounts`.
    /// Example: "z" matches `[channels.discord.accounts.z]`.
    pub discord_account: Option<String>,

    /// Mnemosyne database path override (overrides global `mnemosyne.db_path`).
    /// Defaults to `<agent_workspace>/mnemosyne.db` when not set.
    pub mnemosyne_db: Option<PathBuf>,

    /// Per-agent heartbeat interval override in seconds.
    /// Overrides global `heartbeat_interval_secs`. Default: 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u64>,

    /// Active hours window for this agent (e.g. "09:00-22:00").
    /// Outside these hours the heartbeat is suppressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_hours: Option<String>,

    /// Timezone for `active_hours` (IANA, e.g. "America/New_York"). Default: UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_timezone: Option<String>,

    /// Runtime: timestamp of last heartbeat run (not persisted to config).
    #[serde(skip)]
    pub last_heartbeat_run: Option<std::time::Instant>,

    /// Runtime: when the next heartbeat is due (not persisted to config).
    #[serde(skip)]
    pub next_heartbeat_due: Option<std::time::Instant>,
}

impl AgentConfig {
    /// Resolve the effective workspace directory for this agent,
    /// falling back to `<base_workspace>/agents/<id>/` if not explicitly set.
    pub fn resolve_workspace(&self, base_workspace: &std::path::Path) -> PathBuf {
        self.workspace
            .clone()
            .unwrap_or_else(|| base_workspace.join("agents").join(&self.id))
    }

    /// Resolve the effective sessions directory for this agent,
    /// falling back to `<base_sessions>/agents/<id>/` if not explicitly set.
    pub fn resolve_sessions(&self, base_sessions: &std::path::Path) -> PathBuf {
        self.sessions
            .clone()
            .unwrap_or_else(|| base_sessions.join("agents").join(&self.id))
    }

    /// Returns the display name: explicit `name` if set, otherwise `id`.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Resolve the effective Mnemosyne database path for this agent.
    /// Priority: explicit `mnemosyne_db` > `<agent_workspace>/mnemosyne.db`.
    pub fn resolve_mnemosyne_db(&self, base_workspace: &std::path::Path) -> PathBuf {
        self.mnemosyne_db
            .clone()
            .unwrap_or_else(|| self.resolve_workspace(base_workspace).join("mnemosyne.db"))
    }
}

/// Channel-source routing binding that maps account/channel/guild to a named agent.
///
/// Evaluated before account-based routing. First match wins.
/// Unset fields are wildcards — they match any value.
///
/// Priority order (OpenClaw pattern):
/// 1. Exact binding match (this struct, highest priority)
/// 2. Account-only match (via `DiscordAccountConfig.agent_id`)
/// 3. Default agent (first entry in `Config.agents`, or the built-in agent)
///
/// Note: distinct from `AgentBinding` (which controls per-agent tool policy).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelBinding {
    /// Agent to route to when this binding matches.
    /// Must match an `id` in `Config.agents`.
    pub agent_id: String,

    /// Discord account key to match (must match a key in `channels.discord.accounts`).
    /// Unset = matches any account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

    /// Discord channel ID to match (Snowflake, as string).
    /// Unset = matches any channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,

    /// Discord guild (server) ID to match (Snowflake, as string).
    /// Unset = matches any guild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
}

impl ChannelBinding {
    /// Returns `true` if this binding matches the given routing parameters.
    /// Unset fields on the binding are treated as wildcards.
    pub fn matches(
        &self,
        account_id: Option<&str>,
        channel_id: Option<&str>,
        guild_id: Option<&str>,
    ) -> bool {
        self.account_id.as_deref().is_none_or(|a| Some(a) == account_id)
            && self.channel_id.as_deref().is_none_or(|c| Some(c) == channel_id)
            && self.guild_id.as_deref().is_none_or(|g| Some(g) == guild_id)
    }
}

/// Network configuration for agent fleet communication (hub-spoke WebSocket)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// This node's agent identity (e.g. "@zeus112")
    pub agent_name: Option<String>,

    /// Hub WebSocket URL to connect to (only for fleet agents, not the coordinator)
    /// e.g. "ws://192.168.1.112:8080/v1/ws/nodes"
    pub hub_url: Option<String>,

    /// Target tmux session for delivering incoming messages
    pub tmux_target: Option<String>,
}

/// Model routing configuration for intelligent per-task model selection.
///
/// ```toml
/// [model_routing]
/// enabled = true
/// reasoning = "anthropic/claude-opus-4-20250514"
/// code = "anthropic/claude-sonnet-4-20250514"
/// research = "google/gemini-2.0-flash"
/// speed = "groq/llama-3.3-70b-versatile"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRoutingCoreConfig {
    #[serde(default)]
    pub enabled: bool,
    pub reasoning: Option<String>,
    pub code: Option<String>,
    pub research: Option<String>,
    pub speed: Option<String>,
    pub creative: Option<String>,
    pub review: Option<String>,
    pub general: Option<String>,
}

/// Agent pool configuration for parallel sub-agent execution.
///
/// ```toml
/// [agent_pool]
/// max_concurrent = 4
/// task_timeout_secs = 300
/// rate_limit_ms = 100
/// continue_on_failure = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPoolCoreConfig {
    #[serde(default = "default_pool_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_pool_timeout")]
    pub task_timeout_secs: u64,
    #[serde(default = "default_pool_rate_limit")]
    pub rate_limit_ms: u64,
    #[serde(default = "default_pool_continue")]
    pub continue_on_failure: bool,
}

fn default_pool_max_concurrent() -> usize {
    4
}
fn default_pool_timeout() -> u64 {
    300
}
fn default_pool_rate_limit() -> u64 {
    100
}
fn default_pool_continue() -> bool {
    true
}

impl Default for AgentPoolCoreConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_pool_max_concurrent(),
            task_timeout_secs: default_pool_timeout(),
            rate_limit_ms: default_pool_rate_limit(),
            continue_on_failure: default_pool_continue(),
        }
    }
}

/// Wallet configuration (defined in zeus-core to avoid circular deps)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletCoreConfig {
    /// Directory to store wallet keys (default: ~/.zeus/wallet/)
    #[serde(default = "default_wallet_dir")]
    pub wallet_dir: PathBuf,

    /// Enable x402 automatic payment protocol
    #[serde(default)]
    pub enable_x402: bool,

    /// Maximum amount (in micro-units, 1 token = 1_000_000) per single x402 payment
    #[serde(default = "default_max_payment")]
    pub max_payment_amount: u64,

    /// x402 payment network: "solana-devnet" or "solana-mainnet"
    #[serde(default = "default_wallet_network")]
    pub network: String,

    /// Token symbol → mint address mapping (e.g. "ZEUS" → "<mint>").
    /// Replace placeholder addresses with real mint addresses after token launch.
    #[serde(default = "default_token_mints")]
    pub token_mints: HashMap<String, String>,
}

fn default_wallet_dir() -> PathBuf {
    default_config_dir().join("wallet")
}
fn default_max_payment() -> u64 {
    1_000_000
}
fn default_wallet_network() -> String {
    "solana-devnet".to_string()
}
fn default_token_mints() -> HashMap<String, String> {
    // TODO: Replace with real ZEUS token mint address after token launch
    HashMap::from([("ZEUS".to_string(), "ZEUS_MINT_PLACEHOLDER".to_string())])
}

impl Default for WalletCoreConfig {
    fn default() -> Self {
        Self {
            wallet_dir: default_wallet_dir(),
            enable_x402: false,
            max_payment_amount: default_max_payment(),
            network: default_wallet_network(),
            token_mints: default_token_mints(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default)]
    pub vim_mode: bool,

    /// Automatically restore the most recent session on TUI startup
    #[serde(default)]
    pub resume_last_session: bool,

    /// Tools disabled by the user via the TUI tool visibility toggle.
    /// These tools are hidden from the LLM but remain available in code.
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            vim_mode: false,
            resume_last_session: false,
            disabled_tools: Vec::new(),
        }
    }
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    /// Use OAuth instead of API keys for Anthropic
    #[serde(default)]
    pub use_oauth: bool,

    /// OAuth access token (stored encrypted in keychain on macOS)
    #[serde(skip)]
    pub access_token: Option<String>,

    /// OAuth refresh token (stored encrypted in keychain on macOS)
    #[serde(skip)]
    pub refresh_token: Option<String>,

    /// Token expiry time
    #[serde(skip)]
    pub expires_at: Option<DateTime<Utc>>,

    /// Anthropic OAuth client ID (defaults to built-in)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_client_id: Option<String>,

    /// Anthropic OAuth redirect URI (defaults to http://127.0.0.1:{port}/v1/auth/anthropic/callback)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_redirect_uri: Option<String>,
}

/// OAuth token storage — provider-keyed.
/// Written by onboarding, read by gateway at startup.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    /// The provider this token belongs to (e.g. "openai", "anthropic", "google")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The access token
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Optional refresh token for auto-renewal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// Per-provider credential storage.
/// Stored as `[credentials.openai]`, `[credentials.google]` etc. in config.toml.
/// Supports multiple providers simultaneously (primary + fallback).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<ProviderCredential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<ProviderCredential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google: Option<ProviderCredential>,
    #[serde(default, rename = "google-gemini-cli")]
    pub google_gemini_cli: Option<ProviderCredential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qwen: Option<ProviderCredential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimax: Option<ProviderCredential>,
    #[serde(default, rename = "xiaomimimo", skip_serializing_if = "Option::is_none")]
    pub xiaomimimo: Option<ProviderCredential>,
}

/// A single provider's credential — either API key or OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    /// "api_key" or "oauth"
    #[serde(default = "default_cred_type")]
    pub cred_type: String,
    /// The token/key value
    #[serde(default)]
    pub token: String,
    /// Optional refresh token (OAuth only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

fn default_cred_type() -> String { "api_key".to_string() }

impl CredentialsConfig {
    /// Route an OAuth token into the correct per-provider slot
    /// (`[provider_credentials.{provider}]` with `cred_type="oauth"`).
    ///
    /// This is the store the gateway's auth read-path actually consults
    /// (zeus-llm `from_config`, branch-4: `cred_type=="oauth" → AuthMethod::OAuth`).
    /// Only the providers the read-side enumerates are routable here; others
    /// return `false` so the caller can fall back to `[credentials]`.
    pub fn set_oauth(&mut self, provider: Provider, token: &str) -> bool {
        let cred = ProviderCredential {
            cred_type: "oauth".to_string(),
            token: token.to_string(),
            refresh_token: None,
        };
        match provider {
            Provider::OpenAI => self.openai = Some(cred),
            Provider::Anthropic => self.anthropic = Some(cred),
            Provider::Google => self.google = Some(cred),
            Provider::GoogleGeminiCli => self.google_gemini_cli = Some(cred),
            Provider::Qwen => self.qwen = Some(cred),
            Provider::Minimax => self.minimax = Some(cred),
            Provider::XiaomiMimo => self.xiaomimimo = Some(cred),
            _ => return false,
        }
        true
    }
}

/// Ollama-specific configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OllamaConfig {
    /// Ollama server URL (default: http://localhost:11434)
    #[serde(default = "default_ollama_url")]
    pub url: String,

    /// Preferred model (auto-detected if not set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_model: Option<String>,

    /// Connection timeout in seconds for remote endpoints (default: 30).
    /// Local endpoints (localhost/127.0.0.1) use a shorter 10s timeout.
    #[serde(default = "default_remote_timeout")]
    pub remote_timeout_secs: u64,

    /// Accept self-signed or invalid TLS certificates for remote endpoints.
    /// Only enable for internal/dev Ollama servers behind self-signed certs.
    #[serde(default)]
    pub accept_invalid_certs: bool,

    /// Retry backoff multiplier for remote endpoints in seconds (default: 3).
    /// Remote endpoints use `multiplier * 2^attempt` instead of the standard
    /// `2^attempt` to give slow remote servers more recovery time.
    #[serde(default = "default_remote_backoff_multiplier")]
    pub remote_backoff_multiplier: u64,

    /// Custom temperature for Ollama requests (default: 0.3)
    #[serde(default = "default_ollama_temperature")]
    pub temperature: f64,

    /// Custom num_predict for non-tool requests (default: 1024)
    #[serde(default = "default_ollama_num_predict")]
    pub num_predict: u32,

    /// Custom num_predict for tool-use requests (default: 4096)
    #[serde(default = "default_ollama_num_predict_tools")]
    pub num_predict_tools: u32,

    /// Keep-alive duration string (default: "30m")
    #[serde(default = "default_ollama_keep_alive")]
    pub keep_alive: String,

    /// Custom top_p sampling parameter (optional, Ollama default if unset)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Custom top_k sampling parameter (optional, Ollama default if unset)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Custom repeat_penalty (optional, Ollama default if unset)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f64>,

    /// Maximum number of tools to send to Ollama (default: 30).
    /// Higher values give the model more capabilities but increase prompt overhead.
    /// Set to 0 to disable the cap entirely.
    /// Systems with 128GB+ RAM can safely use 60-100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<usize>,
}

fn default_remote_timeout() -> u64 { 30 }
fn default_remote_backoff_multiplier() -> u64 { 3 }
fn default_ollama_temperature() -> f64 { 0.3 }
fn default_ollama_num_predict() -> u32 { 1024 }
fn default_ollama_num_predict_tools() -> u32 { 8192 } // 4096 was too low for large file generation
fn default_ollama_keep_alive() -> String { "30m".to_string() }

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: default_ollama_url(),
            preferred_model: None,
            remote_timeout_secs: default_remote_timeout(),
            accept_invalid_certs: false,
            remote_backoff_multiplier: default_remote_backoff_multiplier(),
            temperature: default_ollama_temperature(),
            num_predict: default_ollama_num_predict(),
            num_predict_tools: default_ollama_num_predict_tools(),
            keep_alive: default_ollama_keep_alive(),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
            max_tools: None, // Default 30, configurable
        }
    }
}

fn default_ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

// ── Deserializer: accept string OR integer for fields like chat_id ──────────
fn string_or_int<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    struct StringOrInt;
    impl<'de> de::Visitor<'de> for StringOrInt {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or integer")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<String, E> { Ok(v.to_string()) }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<String, E> { Ok(v) }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<String, E> { Ok(v.to_string()) }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<String, E> { Ok(v.to_string()) }
    }
    deserializer.deserialize_any(StringOrInt)
}

// ── Skip-serialization helpers (S22 config save bloat fix) ──────────────────
fn is_false(v: &bool) -> bool { !v }
fn is_default_workspace(p: &PathBuf) -> bool { *p == default_workspace() }
fn is_default_sessions(p: &PathBuf) -> bool { *p == default_sessions() }
fn is_default_max_iter(v: &usize) -> bool { *v == default_max_iterations() }
fn is_default_max_subagent_iter(v: &usize) -> bool { *v == default_max_subagent_iterations() }
fn is_default_tui(v: &TuiConfig) -> bool {
    v.theme == "dark" && !v.vim_mode && !v.resume_last_session && v.disabled_tools.is_empty()
}
fn is_default_auth(v: &AuthConfig) -> bool { !v.use_oauth }
fn is_default_oauth(v: &OAuthConfig) -> bool {
    v.provider.is_none() && v.token.is_none() && v.refresh_token.is_none()
}
fn is_default_credentials(v: &CredentialsConfig) -> bool {
    // MUST enumerate EVERY provider field — a field omitted here means a config
    // where ONLY that provider is set serializes as "default" → the whole
    // [provider_credentials] section is skipped → the credential is silently
    // dropped on write. (#257: xiaomimimo was missing → its OAuth token never
    // persisted even though set_oauth wrote it into the in-memory struct.)
    v.openai.is_none() && v.anthropic.is_none() && v.google.is_none()
        && v.google_gemini_cli.is_none() && v.qwen.is_none() && v.minimax.is_none()
        && v.xiaomimimo.is_none()
}
fn is_default_ollama(v: &OllamaConfig) -> bool { *v == OllamaConfig::default() }

fn default_model() -> String {
    String::new() // No default — user must select during onboarding
}

/// Default Zeus config directory (~/.zeus)
pub fn default_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
}

fn default_workspace() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("workspace")
}

fn default_sessions() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("sessions")
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_max_iterations() -> usize {
    20
}

fn default_max_subagent_iterations() -> usize {
    15
}

// ============================================================================
// Advanced Crate Configs (defined in zeus-core to avoid circular deps)
// ============================================================================

/// Supported embedding provider types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProvider {
    /// Ollama local embedding (nomic-embed-text, etc.)
    Ollama,
    /// OpenAI text-embedding-3-small / text-embedding-3-large
    #[serde(rename = "openai")]
    OpenAI,
    /// Google Gemini embedding-001
    Gemini,
    /// Voyage AI voyage-3 / voyage-code-3
    Voyage,
}

impl std::fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingProvider::Ollama => write!(f, "ollama"),
            EmbeddingProvider::OpenAI => write!(f, "openai"),
            EmbeddingProvider::Gemini => write!(f, "gemini"),
            EmbeddingProvider::Voyage => write!(f, "voyage"),
        }
    }
}

/// Mnemosyne (advanced memory) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MnemosyneConfig {
    /// Database file path
    #[serde(default = "default_mnemosyne_db")]
    pub db_path: PathBuf,
    /// Enable FTS5 full-text search
    #[serde(default = "default_true")]
    pub enable_fts: bool,
    /// Maximum messages to keep per session
    #[serde(default = "default_max_messages")]
    pub max_messages_per_session: usize,
    /// Enable vector embeddings storage
    #[serde(default)]
    pub enable_embeddings: bool,
    /// Embedding dimensions (768 for nomic-embed-text, 1536 for OpenAI)
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
    /// Ollama server URL for generating embeddings
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    /// Embedding model name (e.g., "nomic-embed-text")
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Weight for vector (cosine similarity) score in hybrid search (0.0–1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// Weight for text (BM25/FTS5) score in hybrid search (0.0–1.0)
    #[serde(default = "default_text_weight")]
    pub text_weight: f64,
    /// Multiplier for candidate retrieval in hybrid search (candidates = multiplier * limit)
    #[serde(default = "default_candidate_multiplier")]
    pub candidate_multiplier: usize,
    /// Ordered list of embedding providers to try (fallback chain)
    #[serde(default = "default_embedding_providers")]
    pub embedding_providers: Vec<EmbeddingProvider>,
    /// Number of consecutive failures before switching to next provider
    #[serde(default = "default_fallback_threshold")]
    pub fallback_threshold: usize,
    /// Enable session transcript indexing
    #[serde(default = "default_true")]
    pub enable_session_indexing: bool,
    /// Byte delta threshold before re-indexing a session file
    #[serde(default = "default_session_delta_bytes")]
    pub session_delta_bytes: usize,
    /// Message count delta threshold before re-indexing a session file
    #[serde(default = "default_session_delta_messages")]
    pub session_delta_messages: usize,
    /// Enable file watcher for auto-sync on changes
    #[serde(default)]
    pub enable_file_watcher: bool,
    /// Extra paths to watch (in addition to workspace root)
    #[serde(default)]
    pub watch_paths: Vec<PathBuf>,
    /// Extra markdown directories to index (in addition to workspace memory/)
    #[serde(default)]
    pub extra_memory_paths: Vec<PathBuf>,
    /// Number of approximate tokens to overlap between adjacent chunks (default 80)
    #[serde(default = "default_chunk_overlap_tokens")]
    pub chunk_overlap_tokens: usize,
    /// Number of texts to send per batch embedding API call (default 100)
    #[serde(default = "default_embed_batch_size")]
    pub embed_batch_size: usize,
    /// Enable QMD (BM25+vector+reranking) sidecar for search
    #[serde(default)]
    pub enable_qmd: bool,
    /// QMD sidecar HTTP URL
    #[serde(default = "default_qmd_url")]
    pub qmd_url: String,
    /// QMD request timeout in milliseconds
    #[serde(default = "default_qmd_timeout_ms")]
    pub qmd_timeout_ms: u64,
    /// URL for cross-encoder reranking model (e.g. sentence-transformers served via HTTP)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qmd_reranker_url: Option<String>,
    /// Cross-encoder model name (sent in request body)
    #[serde(default = "default_reranker_model")]
    pub qmd_reranker_model: String,
    /// Weight for BM25 score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_bm25_weight")]
    pub qmd_bm25_weight: f64,
    /// Weight for vector score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_vector_weight")]
    pub qmd_vector_weight: f64,
    /// Weight for cross-encoder score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_reranker_weight")]
    pub qmd_reranker_weight: f64,
    /// Number of over-fetch candidates for reranking (multiplier on limit)
    #[serde(default = "default_qmd_candidate_multiplier")]
    pub qmd_candidate_multiplier: usize,
    /// Dedicated host URL for embedding API calls (overrides ollama_url for embeddings only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_host: Option<String>,
    /// Enable fact-checking during memory compaction.
    /// When true, key facts (entities, dates, decisions) are extracted before
    /// compaction and validated against the summary — missing facts are appended.
    #[serde(default)]
    pub compaction_fact_check: bool,
    /// Maximum total memories before triggering consolidation (0 = unlimited)
    #[serde(default = "default_max_memories")]
    pub max_memories: usize,
    /// FTS5 similarity threshold for dedup (0.0–1.0, higher = stricter). 0 = disabled.
    #[serde(default = "default_dedup_threshold")]
    pub dedup_threshold: f64,
    /// Maximum messages per session before summary consolidation kicks in
    #[serde(default = "default_consolidation_session_limit")]
    pub consolidation_session_limit: usize,
}

fn default_qmd_url() -> String {
    std::env::var("ZEUS_QMD_URL").unwrap_or_else(|_| "http://localhost:7720".to_string())
}
fn default_qmd_timeout_ms() -> u64 {
    3000
}
fn default_reranker_model() -> String {
    "cross-encoder/ms-marco-MiniLM-L-6-v2".to_string()
}
fn default_qmd_bm25_weight() -> f64 {
    0.3
}
fn default_qmd_vector_weight() -> f64 {
    0.3
}
fn default_qmd_reranker_weight() -> f64 {
    0.4
}
fn default_qmd_candidate_multiplier() -> usize {
    4
}
fn default_max_memories() -> usize {
    50_000
}
fn default_dedup_threshold() -> f64 {
    0.85
}
fn default_consolidation_session_limit() -> usize {
    200
}
fn default_chunk_overlap_tokens() -> usize {
    80
}
fn default_embed_batch_size() -> usize {
    100
}
fn default_session_delta_bytes() -> usize {
    100_000
}
fn default_session_delta_messages() -> usize {
    50
}
fn default_embedding_providers() -> Vec<EmbeddingProvider> {
    // S70: Auto-detect embedding provider from available API keys.
    // Priority: OpenAI (best quality) → Ollama (local) → none (FTS-only)
    if std::env::var("OPENAI_API_KEY").map_or(false, |k| !k.is_empty()) {
        vec![EmbeddingProvider::OpenAI]
    } else if std::env::var("OLLAMA_HOST").map_or(false, |_| true) {
        vec![EmbeddingProvider::Ollama]
    } else {
        // No embedding provider available — FTS-only recall
        vec![]
    }
}
fn default_fallback_threshold() -> usize {
    3
}
fn default_vector_weight() -> f64 {
    0.7
}
fn default_text_weight() -> f64 {
    0.3
}
fn default_candidate_multiplier() -> usize {
    4
}
fn default_mnemosyne_db() -> PathBuf {
    default_config_dir().join("memory.db")
}
fn default_true() -> bool {
    true
}
fn default_max_messages() -> usize {
    10000
}
fn default_embedding_dim() -> usize {
    768
}
fn default_embedding_model() -> String {
    "nomic-embed-text".to_string()
}

impl Default for MnemosyneConfig {
    fn default() -> Self {
        Self {
            db_path: default_mnemosyne_db(),
            enable_fts: true,
            max_messages_per_session: 10000,
            enable_embeddings: false,
            embedding_dim: 768,
            ollama_url: default_ollama_url(),
            embedding_model: default_embedding_model(),
            vector_weight: default_vector_weight(),
            text_weight: default_text_weight(),
            candidate_multiplier: default_candidate_multiplier(),
            embedding_providers: default_embedding_providers(),
            fallback_threshold: default_fallback_threshold(),
            enable_session_indexing: true,
            session_delta_bytes: default_session_delta_bytes(),
            session_delta_messages: default_session_delta_messages(),
            enable_file_watcher: false,
            watch_paths: Vec::new(),
            extra_memory_paths: Vec::new(),
            chunk_overlap_tokens: default_chunk_overlap_tokens(),
            embed_batch_size: default_embed_batch_size(),
            enable_qmd: false,
            qmd_url: default_qmd_url(),
            qmd_timeout_ms: default_qmd_timeout_ms(),
            qmd_reranker_url: None,
            qmd_reranker_model: default_reranker_model(),
            qmd_bm25_weight: default_qmd_bm25_weight(),
            qmd_vector_weight: default_qmd_vector_weight(),
            qmd_reranker_weight: default_qmd_reranker_weight(),
            qmd_candidate_multiplier: default_qmd_candidate_multiplier(),
            embedding_host: None,
            compaction_fact_check: false,
            max_memories: default_max_memories(),
            dedup_threshold: default_dedup_threshold(),
            consolidation_session_limit: default_consolidation_session_limit(),
        }
    }
}

/// Athena (documentation engine) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AthenaConfig {
    /// Path to Obsidian vault
    #[serde(default = "default_vault_path")]
    pub vault_path: PathBuf,
}

fn default_vault_path() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("Zeus")
}

impl Default for AthenaConfig {
    fn default() -> Self {
        Self {
            vault_path: default_vault_path(),
        }
    }
}

/// Aegis (security) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AegisConfig {
    /// Keychain service name
    #[serde(default = "default_keychain_service")]
    pub keychain_service: String,
    /// Sandbox level: "none", "basic", "standard", "strict", "paranoid"
    #[serde(default = "default_sandbox_level")]
    pub sandbox_level: String,
    /// Audit log path
    #[serde(default = "default_audit_path")]
    pub audit_path: PathBuf,
    /// Allowed operations
    #[serde(default = "default_permissions")]
    pub permissions: Vec<String>,
    /// Network allowlist
    #[serde(default = "default_network_allowlist")]
    pub network_allowlist: Vec<String>,
    /// Tools that require approval before execution
    #[serde(default)]
    pub require_confirmation_for: Vec<String>,
    /// Approval timeout in seconds (default 1800 = 30 minutes)
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    /// Additional filesystem paths that Zeus may write to, beyond the workspace.
    /// Each entry is treated as a prefix — subdirectories are included.
    #[serde(default)]
    pub allowed_write_paths: Vec<String>,
    /// Grant autonomous agents write access to standard system paths
    /// (/usr/local/, /etc/, etc.). OS-level permissions still apply.
    #[serde(default)]
    pub allow_system_paths: bool,
}

fn default_keychain_service() -> String {
    "zeus".to_string()
}
fn default_sandbox_level() -> String {
    "none".to_string()
}
fn default_audit_path() -> PathBuf {
    default_config_dir().join("audit.log")
}
fn default_permissions() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_network_allowlist() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_approval_timeout_secs() -> u64 {
    1800
}

impl Default for AegisConfig {
    fn default() -> Self {
        Self {
            keychain_service: default_keychain_service(),
            sandbox_level: default_sandbox_level(),
            audit_path: default_audit_path(),
            permissions: default_permissions(),
            network_allowlist: default_network_allowlist(),
            require_confirmation_for: Vec::new(),
            approval_timeout_secs: default_approval_timeout_secs(),
            allowed_write_paths: Vec::new(),
            allow_system_paths: false,
        }
    }
}

/// Hermes (notifications) configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HermesConfig {
    /// Default notification channels
    #[serde(default)]
    pub default_channels: Vec<String>,
    /// Whether to batch low priority notifications
    #[serde(default)]
    pub batch_low_priority: bool,
}

/// Prometheus (orchestration) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    /// Enable heartbeat tasks
    #[serde(default)]
    pub enable_heartbeat: bool,
    /// Heartbeat check interval in seconds
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    /// Enable cognitive (Nous) integration
    #[serde(default)]
    pub enable_cognitive: bool,
    /// Maximum iterations per request
    #[serde(default = "default_prometheus_max_iterations")]
    pub max_iterations: usize,
    /// Cron-based scheduler configuration. Typed in zeus-core; rich engine-side
    /// `SchedulerConfig` lives in zeus-prometheus and round-trips via serde_json.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<PrometheusSchedulerConfig>,
    /// Autonomy level for decision-making. Typed mirror of zeus-prometheus
    /// `AutonomyConfig`; serde-shape-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<PrometheusAutonomyConfig>,
    /// Learning engine configuration. Typed mirror of zeus-prometheus
    /// `LearningConfig`; serde-shape-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning: Option<PrometheusLearningConfig>,
    /// Monitor configuration. Typed mirror of zeus-prometheus
    /// `MonitorConfig`; serde-shape-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<PrometheusMonitorConfig>,
    /// Heartbeat runtime configuration (`[prometheus.heartbeat]`).
    /// Typed mirror of zeus-prometheus `HeartbeatConfig`; serde-shape-compatible.
    /// Lets ops flip `event_driven_only`, tune intervals, and override quiet hours
    /// without a binary patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<PrometheusHeartbeatConfig>,
    /// Backlog sync (autonomous backlog-pull) configuration. Sprint #84.
    /// Typed mirror of zeus-prometheus `BacklogSyncConfig`; serde-shape-compatible.
    /// Opt-in: disabled by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog_sync: Option<PrometheusBacklogSyncConfig>,
    /// Context journal configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_journal: Option<ContextJournalConfig>,
    /// Maximum cooking backoff in milliseconds (default: 100000 = 100s)
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,
    /// Overall cooking-loop wall-clock timeout in seconds.
    /// When absent, falls back to `gateway.timeout_secs` (default 1800s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooking_loop_timeout_secs: Option<u64>,
    /// Human-readable cooking-loop wall-clock timeout (e.g. "2h", "30 hours", "45m").
    /// When present and parseable, overrides `cooking_loop_timeout_secs`.
    /// Falls back to `cooking_loop_timeout_secs`, then gateway default (1800s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooking_loop_timeout: Option<String>,
    /// #176/#284B: Failsafe ceiling for task-derived cooking idle windows.
    /// Caps explicit H1 task-derived idle windows so a misparse cannot create an
    /// unbounded idle wait. Active cook lifetime is governed by the idle watchdog,
    /// not this ceiling. Human-readable (e.g. "24h"). When absent, defaults to 24 hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooking_loop_max: Option<String>,
    /// Legacy #176-b H2 recency-window knob retained for config compatibility.
    /// The #284B dispatch watchdog now uses `cooking_loop_timeout` / gateway timeout
    /// as the idle window and activity comes from model text or completed tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooking_recency_window: Option<String>,
    /// Legacy #176-b H2 extension-quantum knob retained for config compatibility.
    /// The #284B dispatch watchdog re-arms to `last_activity_at + idle_window` instead
    /// of extending a static deadline by fixed quanta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooking_extension_quantum: Option<String>,
    /// Number of recent messages to inject from OTHER channel sessions before each cook.
    /// Number of recent messages from *other* sessions to prepend as a
    /// "## Cross-channel session tail" block. 0 = feature off.
    /// Default 5 (#192): same-human session continuity across surfaces —
    /// the fleet session resolver is keyed on `(agent, human)`, so this is
    /// same-human carry-over, never cross-human bleed. Set
    /// `cross_channel_session_tail_n = 0` under `[prometheus]` in config.toml
    /// to opt out. Mirrors #86 Mnemosyne memory-injection pattern.
    #[serde(default = "default_cross_channel_session_tail_n")]
    pub cross_channel_session_tail_n: usize,
    /// Max number of cooking-checkpoint rows to retain (count-cap). Keeps the N
    /// most-recently-updated; deletes the rest on each retention sweep. 0 disables.
    /// Guards against the unbounded DB growth that ballooned cooking_checkpoints.db.
    #[serde(default = "default_checkpoint_max_rows")]
    pub checkpoint_max_rows: usize,
    /// Age cap for completed checkpoints, in days. Completed rows older than this
    /// are pruned on each sweep. 0 disables the age cap.
    #[serde(default = "default_checkpoint_max_age_days")]
    pub checkpoint_max_age_days: u64,
    /// How often (seconds) to run the checkpoint retention sweep (age cap + count
    /// cap + VACUUM). 0 = boot-only (sweep once at startup, never re-run).
    #[serde(default = "default_checkpoint_sweep_interval_secs")]
    pub checkpoint_sweep_interval_secs: u64,
}

fn default_cross_channel_session_tail_n() -> usize {
    // #192: default-on at 5. Safe because the fleet session resolver is keyed
    // on (agent, human) — same-human continuity across surfaces, not
    // cross-human bleed. Dial back to 0 via config to opt out.
    5
}

fn default_checkpoint_max_rows() -> usize {
    500 // generous active-resume window; far below the count that bloated the DB to 2 GB
}

fn default_checkpoint_max_age_days() -> u64 {
    7 // completed checkpoints older than a week have no resume value
}

fn default_checkpoint_sweep_interval_secs() -> u64 {
    3600 // hourly — a long-lived gateway re-sweeps instead of boot-only
}

fn default_heartbeat_interval() -> u64 {
    300 // 5 minutes — active autonomy, cook-priority prevents message starvation
}

fn default_prometheus_max_iterations() -> usize {
    20
}

fn default_max_backoff_ms() -> u64 {
    100_000
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enable_heartbeat: true,
            heartbeat_interval_secs: 300,
            enable_cognitive: true,
            max_iterations: 20,
            scheduler: None,
            autonomy: None,
            learning: None,
            monitor: None,
            heartbeat: None,
            backlog_sync: None,
            context_journal: None,
            max_backoff_ms: default_max_backoff_ms(),
            cooking_loop_timeout_secs: None,
            cooking_loop_timeout: None,
            cooking_loop_max: None,
            cooking_recency_window: None,
            cooking_extension_quantum: None,
            cross_channel_session_tail_n: default_cross_channel_session_tail_n(),
            checkpoint_max_rows: default_checkpoint_max_rows(),
            checkpoint_max_age_days: default_checkpoint_max_age_days(),
            checkpoint_sweep_interval_secs: default_checkpoint_sweep_interval_secs(),
        }
    }
}

/// Resolve the effective cooking-loop wall-clock timeout.
/// Priority: NL string > secs override > gateway_default_secs (fallback 1800s).
pub fn resolve_cooking_loop_timeout(config: &PrometheusConfig, gateway_default_secs: u64) -> std::time::Duration {
    if let Some(ref raw) = config.cooking_loop_timeout {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
        }
    }
    if let Some(secs) = config.cooking_loop_timeout_secs {
        if secs > 0 {
            return std::time::Duration::from_secs(secs);
        }
    }
    let effective = if gateway_default_secs > 0 { gateway_default_secs } else { 1800 };
    std::time::Duration::from_secs(effective)
}

/// Parse a per-goal front-matter timeout string (e.g. `timeout: "30 hours"`).
/// Returns None if input is None/empty/unparseable.
pub fn parse_goal_timeout(raw: Option<&str>) -> Option<std::time::Duration> {
    let s = raw?.trim();
    if s.is_empty() { return None; }
    humantime::parse_duration(s).ok()
}

/// #176/#284B: Resolve the failsafe ceiling for task-derived idle windows.
///
/// This no longer caps active cook lifetime; #284B's watchdog treats the resolved
/// cooking timeout as an idle window. The cap only prevents absurd explicit-timeout
/// parses from turning into unbounded idle waits. Human-readable (`cooking_loop_max`);
/// falls back to 24 hours when absent/unparseable.
pub fn resolve_cooking_loop_max(config: &PrometheusConfig) -> std::time::Duration {
    const DEFAULT_MAX: u64 = 24 * 60 * 60; // 24h failsafe
    if let Some(ref raw) = config.cooking_loop_max {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
        }
    }
    std::time::Duration::from_secs(DEFAULT_MAX)
}

/// #176-b H2: Resolve the progress-recency window. A tool-call completed within this
/// window of the deadline means the cook is actively working → extend instead of kill.
/// Human-readable (`cooking_recency_window`); falls back to 120s when absent/unparseable.
pub fn resolve_cooking_recency_window(config: &PrometheusConfig) -> std::time::Duration {
    const DEFAULT_RECENCY: u64 = 120; // 120s
    if let Some(ref raw) = config.cooking_recency_window {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
        }
    }
    std::time::Duration::from_secs(DEFAULT_RECENCY)
}

/// Legacy #176-b H2 extension-quantum resolver retained for config compatibility.
/// #284B no longer extends a static deadline by fixed quanta; the dispatch
/// watchdog re-arms to `last_activity_at + idle_window`.
pub fn resolve_cooking_extension_quantum(config: &PrometheusConfig) -> std::time::Duration {
    const DEFAULT_QUANTUM: u64 = 600; // 600s
    if let Some(ref raw) = config.cooking_extension_quantum {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
        }
    }
    std::time::Duration::from_secs(DEFAULT_QUANTUM)
}

/// #284B: Idle/no-progress watchdog decision for dispatch cooking.
///
/// `last_activity_at` is unix-seconds for the most recent observed cook activity
/// (model text response or completed tool call). `idle_window` is the configured
/// gateway cooking timeout interpreted as an idle window, not an absolute cook
/// lifetime. Kill only when activity is strictly older than the idle window.
pub fn should_abort_for_idle(
    last_activity_at: u64,
    now: u64,
    idle_window: std::time::Duration,
) -> bool {
    if last_activity_at == 0 {
        return true;
    }
    now.saturating_sub(last_activity_at) > idle_window.as_secs()
}

/// #176 H1: Conservatively extract an explicit cooking budget from dispatch task text.
///
/// Matches ONLY explicit intent patterns — "for Xh", "for X hours", "for the next Xh/X
/// hours/X minutes" — NOT any bare duration mentioned in the body (false-positive risk:
/// "the 2h timeout bug" must NOT set a 2h budget). Returns the parsed duration capped at
/// `ceiling`; `None` when no explicit pattern is present (caller keeps the config default).
///
/// The ceiling guarantees a misparse can never exceed `cooking_loop_max`.
pub fn extract_task_timeout(
    text: &str,
    ceiling: std::time::Duration,
) -> Option<std::time::Duration> {
    let lower = text.to_lowercase();
    // Scan for "for [the next ]<num> <unit>" where unit ∈ {h, hour(s), m, min(s), minute(s)}.
    // Conservative: requires the literal "for " lead-in to signal intent.
    let bytes = lower.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("for ") {
        let start = search_from + rel + 4; // past "for "
        search_from = start;
        let mut i = start;
        // optional "the next "
        if lower[i..].starts_with("the next ") {
            i += "the next ".len();
        }
        // parse leading number (integer or simple decimal)
        let num_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i == num_start {
            continue; // no number after "for " — not our pattern
        }
        let num: f64 = match lower[num_start..i].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // optional single space before unit
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let rest = &lower[i..];
        let secs: Option<u64> = if rest.starts_with("hours")
            || rest.starts_with("hour")
            || rest.starts_with("hrs")
            || rest.starts_with("hr")
            || rest.starts_with('h')
        {
            Some((num * 3600.0) as u64)
        } else if rest.starts_with("minutes")
            || rest.starts_with("minute")
            || rest.starts_with("mins")
            || rest.starts_with("min")
            || rest.starts_with('m')
        {
            Some((num * 60.0) as u64)
        } else {
            None
        };
        if let Some(s) = secs {
            if s == 0 {
                continue;
            }
            let d = std::time::Duration::from_secs(s);
            return Some(d.min(ceiling)); // ceiling-capped: misparse can't exceed max
        }
    }
    None
}

// ── Phase 3.5: Typed mirrors of zeus-prometheus engine configs ──────────────
//
// These structs sit in zeus-core because:
//   1. zeus-prometheus depends on zeus-core (not the reverse), so the
//      canonical `*Config` types in zeus-prometheus can't be imported here.
//   2. Onboarding (zeus-tui) needs typed write-access to these subsections
//      to stamp `[prometheus.scheduler]` / `[prometheus.autonomy]` / etc.
//      via `Config::save()` instead of leaving them as opaque
//      `Option<serde_json::Value>` blobs.
//
// Each mirror is **serde-shape-compatible** with its zeus-prometheus twin —
// zeus-prometheus continues to call `serde_json::from_value::<*Config>(v.clone())`
// on the field, which round-trips cleanly through these typed structs.
// When the zeus-prometheus shape evolves, fields here must be kept in sync
// (or made permissive via `#[serde(default)]` + extra-field tolerance).

/// Typed mirror of zeus-prometheus `SchedulerConfig`. See
/// `crates/zeus-prometheus/src/scheduler.rs` for the canonical engine struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrometheusSchedulerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_scheduler_max_concurrent_jobs")]
    pub max_concurrent_jobs: u32,
    /// Engine-side `Vec<TaskConfig>`; preserved as raw JSON in zeus-core to
    /// avoid duplicating the `TaskConfig` enum hierarchy. Round-trips via
    /// `serde_json::from_value::<SchedulerConfig>` in zeus-prometheus.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<serde_json::Value>,
}

fn default_scheduler_max_concurrent_jobs() -> u32 {
    4
}

/// Typed mirror of zeus-prometheus `AutonomyConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusAutonomyConfig {
    /// "full" | "supervised" | "restricted"
    #[serde(default = "default_autonomy_level")]
    pub level: String,
    #[serde(default = "default_autonomy_confidence")]
    pub confidence_threshold: f32,
    #[serde(default = "default_autonomy_max_tools")]
    pub max_autonomous_tools: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_confirmation_for: Vec<String>,
    #[serde(default = "default_autonomy_error_threshold")]
    pub error_threshold: usize,
}

impl Default for PrometheusAutonomyConfig {
    fn default() -> Self {
        Self {
            level: default_autonomy_level(),
            confidence_threshold: default_autonomy_confidence(),
            max_autonomous_tools: default_autonomy_max_tools(),
            require_confirmation_for: Vec::new(),
            error_threshold: default_autonomy_error_threshold(),
        }
    }
}

fn default_autonomy_level() -> String {
    "supervised".to_string()
}
fn default_autonomy_confidence() -> f32 {
    0.7
}
fn default_autonomy_max_tools() -> usize {
    20
}
fn default_autonomy_error_threshold() -> usize {
    3
}

/// Typed mirror of zeus-prometheus `LearningConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrometheusLearningConfig {
    /// SQLite DB path. Engine fills with `~/.zeus/learning.db` if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_observations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_threshold: Option<f32>,
    /// Permissive bag for fields zeus-prometheus may add later.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Typed mirror of zeus-prometheus `MonitorConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrometheusMonitorConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_interval_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate_threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_threshold_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_success_rate: Option<f32>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Typed mirror of zeus-prometheus `HeartbeatConfig`.
/// Onboarding step 16 (Orchestration) writes the user-facing knobs here;
/// zeus-prometheus deserializes into its richer engine struct at boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusHeartbeatConfig {
    #[serde(default = "default_quiet_hours_start")]
    pub quiet_hours_start: u8,
    #[serde(default = "default_quiet_hours_end")]
    pub quiet_hours_end: u8,
    #[serde(default = "default_true")]
    pub enable_quiet_hours: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default = "default_heartbeat_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_heartbeat_dedup_window_secs")]
    pub dedup_window_secs: u64,
    #[serde(default = "default_heartbeat_active_interval_secs")]
    pub active_interval_secs: u64,
    #[serde(default = "default_event_driven_only")]
    pub event_driven_only: bool,
    #[serde(default = "default_safety_net_interval_secs")]
    pub safety_net_interval_secs: u64,
    #[serde(default = "default_plan_resume_interval_secs")]
    pub plan_resume_interval_secs: u64,
    /// Permissive bag for fields zeus-prometheus may add later.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for PrometheusHeartbeatConfig {
    fn default() -> Self {
        Self {
            quiet_hours_start: default_quiet_hours_start(),
            quiet_hours_end: default_quiet_hours_end(),
            enable_quiet_hours: true,
            timezone: None,
            timeout_secs: default_heartbeat_timeout_secs(),
            dedup_window_secs: default_heartbeat_dedup_window_secs(),
            active_interval_secs: default_heartbeat_active_interval_secs(),
            event_driven_only: default_event_driven_only(),
            safety_net_interval_secs: default_safety_net_interval_secs(),
            plan_resume_interval_secs: default_plan_resume_interval_secs(),
            extra: Default::default(),
        }
    }
}

fn default_quiet_hours_start() -> u8 {
    23
}
fn default_quiet_hours_end() -> u8 {
    8
}
fn default_heartbeat_timeout_secs() -> u64 {
    30
}
fn default_heartbeat_dedup_window_secs() -> u64 {
    86_400
}
fn default_heartbeat_active_interval_secs() -> u64 {
    120
}
fn default_event_driven_only() -> bool {
    true
}
fn default_safety_net_interval_secs() -> u64 {
    3_600
}
fn default_plan_resume_interval_secs() -> u64 {
    3_600
}

/// Typed mirror of zeus-prometheus `BacklogSyncConfig` (Sprint #84).
///
/// Schema for the `[prometheus.backlog_sync]` section in `config.toml`.
/// V1 is opt-in (`enabled = false`); when enabled, the gateway spawns a
/// background loop in `zeus-prometheus::backlog_sync::sync_loop` that
/// periodically pulls backlog items and stages them as goal files in
/// `~/.zeus/workspace/goals/` for the existing autonomous_loop hot-loader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusBacklogSyncConfig {
    /// Master switch. Default `false` (opt-in for V1).
    #[serde(default)]
    pub enabled: bool,

    /// Source kind: "local_file" | "github_issues" | "hybrid" (V1: only
    /// "local_file" is wired; the others are stubs that return empty).
    #[serde(default = "default_backlog_source")]
    pub source: String,

    /// Poll interval in seconds (default 60).
    #[serde(default = "default_backlog_poll_interval_secs")]
    pub poll_interval_secs: u64,

    /// Cap on pending goal files in `goals/` before staging halts (default 20).
    #[serde(default = "default_backlog_max_pending")]
    pub max_pending: usize,

    /// Path to local backlog markdown file (used when `source = "local_file"`).
    /// Defaults to `~/.zeus/workspace/BACKLOG.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,

    /// GitHub repository, format `"owner/repo"` (used when `source = "github_issues"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repo: Option<String>,

    /// GitHub issue labels to filter on (e.g. `["backlog", "agent-task"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub github_labels: Vec<String>,

    /// GitHub token env var name (default `GITHUB_TOKEN`). The value is
    /// resolved from the env at boot, not stored in config.toml.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_token_env: Option<String>,

    /// Default titan role tag stamped into rendered goal files
    /// (default "implementer"). Future use for role-based dispatch.
    #[serde(default = "default_backlog_titan_role")]
    pub titan_role: String,

    /// Forward-compat catch-all for unknown keys.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for PrometheusBacklogSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: default_backlog_source(),
            poll_interval_secs: default_backlog_poll_interval_secs(),
            max_pending: default_backlog_max_pending(),
            local_path: None,
            github_repo: None,
            github_labels: Vec::new(),
            github_token_env: None,
            titan_role: default_backlog_titan_role(),
            extra: Default::default(),
        }
    }
}

fn default_backlog_source() -> String {
    "local_file".to_string()
}
fn default_backlog_poll_interval_secs() -> u64 {
    60
}
fn default_backlog_max_pending() -> usize {
    20
}
fn default_backlog_titan_role() -> String {
    "implementer".to_string()
}

/// Voice (TTS / STT) backend configuration. Onboarding step 14 stamps the
/// selected provider here; downstream voice integrations read it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Provider key: "elevenlabs" | "openai_tts" | "local_coqui" | "system_tts" | "none".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Optional model/voice ID for providers that accept one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether voice output is globally enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Permissive bag for provider-specific knobs (sample rate, voice id, etc.).
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Nous (cognitive engine) configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NousConfig {
    /// Enable intent recognition
    #[serde(default = "default_true")]
    pub enable_intent: bool,
    /// Enable learning from interactions
    #[serde(default)]
    pub enable_learning: bool,
}

/// Talos (automation tools) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalosConfig {
    /// Enable calendar tools
    #[serde(default = "default_true")]
    pub calendar: bool,
    /// Enable notes tools
    #[serde(default = "default_true")]
    pub notes: bool,
    /// Enable reminders tools
    #[serde(default = "default_true")]
    pub reminders: bool,
    /// Enable contacts tools
    #[serde(default = "default_true")]
    pub contacts: bool,
    /// Enable browser tools
    #[serde(default = "default_true")]
    pub browser: bool,
    /// Enable system tools
    #[serde(default = "default_true")]
    pub system: bool,
    /// Enable network tools
    #[serde(default = "default_true")]
    pub network: bool,
}

impl Default for TalosConfig {
    fn default() -> Self {
        Self {
            calendar: true,
            notes: true,
            reminders: true,
            contacts: true,
            browser: true,
            system: true,
            network: true,
        }
    }
}

/// Search configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Search provider: "brave" or "duckduckgo"
    #[serde(default = "default_search_provider")]
    pub provider: String,
    /// API key for Brave Search (or from BRAVE_API_KEY env)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Maximum number of results to return
    #[serde(default = "default_search_max_results")]
    pub max_results: usize,
}

fn default_search_provider() -> String {
    "duckduckgo".to_string()
}
fn default_search_max_results() -> usize {
    5
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            provider: default_search_provider(),
            api_key: None,
            max_results: default_search_max_results(),
        }
    }
}

/// Rate limiting configuration for the HTTP API gateway.
///
/// Applied per-IP using a token bucket algorithm with two tiers:
/// global (all endpoints) and LLM (expensive endpoints only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayRateLimitConfig {
    /// Enable per-IP rate limiting (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Max requests per minute per IP for all endpoints (default: 120)
    #[serde(default = "default_rl_global_rpm")]
    pub global_rpm: u32,
    /// Max requests per minute per IP for LLM-invoking endpoints (default: 20)
    #[serde(default = "default_rl_llm_rpm")]
    pub llm_rpm: u32,
    /// Extra burst tokens above the sustained rate (default: 10)
    #[serde(default = "default_rl_burst_size")]
    pub burst_size: u32,
}

impl Default for GatewayRateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            global_rpm: default_rl_global_rpm(),
            llm_rpm: default_rl_llm_rpm(),
            burst_size: default_rl_burst_size(),
        }
    }
}

fn default_rl_global_rpm() -> u32 {
    120
}
fn default_rl_llm_rpm() -> u32 {
    20
}
fn default_rl_burst_size() -> u32 {
    10
}

/// Logging configuration: `[logging]` section.
///
/// Controls the default tracing level for all Zeus workspace crates, the
/// durable rotating file sink under `{zeus_home}/logs/`, and its retention.
/// `RUST_LOG` always wins over these settings when set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Default level for stderr/console output: trace|debug|info|warn|error.
    /// Overridden by `--verbose` (forces debug) and by `RUST_LOG`.
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Level for the rotating file sink (defaults to `level` when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_level: Option<String>,

    /// Enable the rotating file sink under `{zeus_home}/logs/gateway.log` (default: true).
    #[serde(default = "default_true")]
    pub file_enabled: bool,

    /// How many days of rotated daily log files to keep (default: 7).
    #[serde(default = "default_log_retention_days")]
    pub retention_days: u32,

    /// #332 ⑤ — per-subsystem level overrides, applied on top of the base
    /// level. Keys are tracing targets: workspace crates (`zeus_channels`,
    /// `zeus_prometheus`, …), bare event targets (`boot`, `adapter`, `cook`,
    /// `msg`), or external SDK crates (`serenity`, `matrix_sdk`, …).
    ///
    /// ```toml
    /// [logging.targets]
    /// zeus_channels = "debug"   # one chatty subsystem up
    /// serenity = "info"         # one SDK deeper than the warn default
    /// zeus_api = "warn"         # one noisy subsystem down
    /// ```
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub targets: std::collections::HashMap<String, String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file_level: None,
            file_enabled: true,
            retention_days: default_log_retention_days(),
            targets: std::collections::HashMap::new(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_retention_days() -> u32 {
    7
}

/// Gateway configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Host to bind to
    #[serde(default = "default_gateway_host")]
    pub host: String,
    /// Port to listen on
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    /// Public URL for remote frontend connections (set during onboarding)
    #[serde(default = "default_gateway_public_url")]
    pub public_url: String,
    /// Enable channel adapters
    #[serde(default = "default_true")]
    pub enable_channels: bool,
    /// Enable cron scheduler
    #[serde(default = "default_true")]
    pub enable_cron: bool,
    /// Enable heartbeat
    #[serde(default = "default_true")]
    pub enable_heartbeat: bool,
    /// Enable API server
    #[serde(default = "default_true")]
    pub enable_api: bool,
    /// Enable MCP server
    #[serde(default = "default_true")]
    pub enable_mcp: bool,
    /// MCP server port
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// Path to web frontend dist directory
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_dist: Option<String>,
    /// Web frontend port (default: 8081)
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    /// Gateway request timeout in seconds (default: 1800 = 30 min)
    #[serde(default = "default_gateway_timeout_secs")]
    pub timeout_secs: u64,
    /// Reconnect delay for WebSocket/node clients in seconds (default: 5)
    #[serde(default = "default_reconnect_delay_secs")]
    pub reconnect_delay_secs: u64,
    /// #329: hard shutdown deadline in seconds (default: 60). After SIGTERM/
    /// Ctrl+C, a runtime-INDEPENDENT OS thread force-exits the process if
    /// graceful teardown hasn't completed within this window. Exists because
    /// a wedged tokio time driver can starve async timeouts (minibsd hung
    /// 8 days in the drain despite an existing 5s tokio timeout).
    #[serde(default = "default_shutdown_hard_deadline_secs")]
    pub shutdown_hard_deadline_secs: u64,
    /// #331: hold a macOS IOPM `PreventSystemSleep` assertion for the
    /// gateway's lifetime so Maintenance-Sleep/DarkWake cycles can't freeze
    /// it (default: true). macOS-only effect; no-op on other platforms.
    /// Boundary: does not survive lid-close on laptops and may be ignored
    /// on battery — desktop seats get full coverage. Set false if the
    /// operator wants the machine to sleep normally.
    #[serde(default = "default_prevent_sleep")]
    pub prevent_sleep: bool,
    /// Maximum WebSocket message size in bytes (default: 1MB)
    #[serde(default = "default_max_ws_message_bytes")]
    pub max_ws_message_bytes: usize,
    /// Maximum webhook payload size in bytes (default: 256KB)
    #[serde(default = "default_max_webhook_payload_bytes")]
    pub max_webhook_payload_bytes: usize,
    /// Maximum webhook message body size in bytes (default: 50KB)
    #[serde(default = "default_max_webhook_message_bytes")]
    pub max_webhook_message_bytes: usize,
    /// Maximum inbound message length in characters (default: 50000)
    #[serde(default = "default_max_inbound_message_len")]
    pub max_inbound_message_len: usize,
    /// HTTP rate limiting configuration
    #[serde(default)]
    pub rate_limit: GatewayRateLimitConfig,
    /// Enable agent processing for inbound channel messages (default: true).
    /// When false, the gateway receives and relays messages but does NOT
    /// run them through the agent loop (useful when Claude Code handles messages).
    #[serde(default = "default_true")]
    pub enable_agent_processing: bool,

    /// Prefix prepended to outbound channel messages (e.g. "[Zeus100]").
    /// Helps identify which agent replied in shared fleet channels.
    /// Uses `{agent_name}` template to auto-insert from `[network].agent_name`.
    /// Set to empty string to disable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_prefix: Option<String>,

    /// Custom channel system prompt injected before each inbound message.
    /// Overrides the hardcoded default. Set to "" to disable all injection.
    /// If not set, uses a minimal default: "You are part of a team. Respond naturally."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_prompt: Option<String>,

    /// Only process messages that @mention this agent (default: false).
    /// When true, ALL inbound messages (bot AND human) are filtered:
    /// only messages containing @agent_name or <@BOT_SNOWFLAKE> trigger cooking.
    /// Non-mentioned messages are still saved to session for context.
    #[serde(default)]
    pub mentions_only: bool,

    /// Discord role IDs that this agent belongs to.
    /// When a message contains `<@&ROLE_ID>` and the ID is in this list,
    /// the agent treats it as directly addressed (same as @mention).
    /// Empty = role mentions are ignored (safe default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discord_role_ids: Vec<String>,

    /// Peer agent names (including self) that share our role mentions / broadcasts.
    /// Used for responder election on role pings and `@everyone` / `@here`:
    /// only the first agent alphabetically (including self) cooks; others skip.
    /// Empty = election disabled (backward-compatible: everyone cooks, race wins).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_agent_names: Vec<String>,

    /// Discord fleet channel ID for agent notifications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_channel_id: Option<String>,

    /// API authentication token (replaces ZEUS_API_TOKEN env var)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,

    /// Comma-separated CORS allowed origins (replaces ZEUS_CORS_ORIGINS env var)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors_origins: Option<String>,

    /// DM session scope — controls how inbound channel messages are routed to sessions.
    ///
    /// - `"main"` (default): all channels share a single `primary` session.
    ///   Useful when you want one unified context across Discord, Telegram, Slack, etc.
    /// - `"per-channel"`: each channel type gets its own session, keyed by
    ///   `<channel_type>-<chat_id>`. Preserves separate conversation contexts.
    ///
    /// Config: `[gateway] dm_scope = "main"`
    #[serde(default = "default_dm_scope")]
    pub dm_scope: String,

    /// #89.4: Allow non-coordinator agents to tag/mention peer bots in responses.
    /// When true, the system prompt restriction on peer-tagging is relaxed.
    /// Safety: max 1 peer-tag per response is enforced at the prompt level.
    /// Default: false (backward-compatible — only coordinator can delegate).
    #[serde(default)]
    pub allow_peer_tagging: bool,
}

fn default_dm_scope() -> String {
    "main".to_string()
}

fn default_web_port() -> u16 {
    8081
}
fn default_gateway_timeout_secs() -> u64 {
    3600 // 60 minutes — long tasks (repo analysis, fleet coordination) need time
}
fn default_reconnect_delay_secs() -> u64 {
    5
}
fn default_prevent_sleep() -> bool {
    true
}

fn default_shutdown_hard_deadline_secs() -> u64 {
    60
}
fn default_max_ws_message_bytes() -> usize {
    1_048_576 // 1 MB
}
fn default_max_webhook_payload_bytes() -> usize {
    262_144 // 256 KB
}
fn default_max_webhook_message_bytes() -> usize {
    51_200 // 50 KB
}
fn default_max_inbound_message_len() -> usize {
    50_000
}

fn default_gateway_host() -> String {
    "0.0.0.0".to_string()
}
fn default_gateway_port() -> u16 {
    8080
}
fn default_gateway_public_url() -> String {
    String::new()
}
fn default_mcp_port() -> u16 {
    3002
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_gateway_host(),
            port: default_gateway_port(),
            public_url: default_gateway_public_url(),
            enable_channels: true,
            enable_cron: true,
            enable_heartbeat: true,
            enable_api: true,
            enable_mcp: true,
            mcp_port: default_mcp_port(),
            web_dist: None,
            web_port: default_web_port(),
            timeout_secs: default_gateway_timeout_secs(),
            reconnect_delay_secs: default_reconnect_delay_secs(),
            shutdown_hard_deadline_secs: default_shutdown_hard_deadline_secs(),
            prevent_sleep: default_prevent_sleep(),
            max_ws_message_bytes: default_max_ws_message_bytes(),
            max_webhook_payload_bytes: default_max_webhook_payload_bytes(),
            max_webhook_message_bytes: default_max_webhook_message_bytes(),
            max_inbound_message_len: default_max_inbound_message_len(),
            rate_limit: GatewayRateLimitConfig::default(),
            enable_agent_processing: true,
            mentions_only: false,
            discord_role_ids: Vec::new(),
            peer_agent_names: Vec::new(),
            response_prefix: None,
            channel_prompt: None,
            fleet_channel_id: None,
            api_token: None,
            cors_origins: None,
            dm_scope: default_dm_scope(),
            allow_peer_tagging: false,
        }
    }
}

/// Session compaction configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCompactionConfig {
    /// Maximum context tokens before compaction
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Compact at this fraction of max (e.g., 0.8 = compact at 80%)
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f32,
    /// Override model for summarization
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<String>,
    /// Timeout in seconds for the compaction LLM call. On timeout, compaction
    /// is skipped for this cycle (agent loop continues oversized). Default 120s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_timeout_secs: Option<u64>,
    /// Compaction fill threshold for the Ollama inline path (0.0–1.0).
    /// Defaults to 0.7 to preserve existing behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_compaction_threshold: Option<f64>,
    /// Pre-compaction flush timeout in seconds. Defaults to 30s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_timeout_secs: Option<u64>,
}

fn default_max_context_tokens() -> usize {
    180000
}
fn default_compaction_threshold() -> f32 {
    0.8
}

impl Default for SessionCompactionConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: default_max_context_tokens(),
            compaction_threshold: default_compaction_threshold(),
            summary_model: None,
            compaction_timeout_secs: None,
            ollama_compaction_threshold: None,
            flush_timeout_secs: None,
        }
    }
}

/// Session pruning configuration — auto-deletes old/excess session files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningConfig {
    /// Whether automatic pruning is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Maximum age of sessions in days before they are pruned
    #[serde(default = "default_pruning_max_age_days")]
    pub max_age_days: u32,
    /// Maximum number of sessions to keep
    #[serde(default = "default_pruning_max_sessions")]
    pub max_sessions: usize,
    /// Maximum total size in MB for all session files
    #[serde(default = "default_pruning_max_total_size_mb")]
    pub max_total_size_mb: u64,
    /// Interval in seconds between pruning checks
    #[serde(default = "default_pruning_check_interval_secs")]
    pub check_interval_secs: u64,
    /// Dry-run mode: log what would be pruned without deleting
    #[serde(default)]
    pub dry_run: bool,
}

fn default_pruning_max_age_days() -> u32 {
    7 // keep sessions for 7 days; prevents unbounded session accumulation
}
fn default_pruning_max_sessions() -> usize {
    50 // cap at 50 sessions; prevents context bloat causing over-cooking
}
fn default_pruning_max_total_size_mb() -> u64 {
    500
}
fn default_pruning_check_interval_secs() -> u64 {
    3600
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            enabled: true, // enabled by default — session bloat causes 17+ iteration over-cooking
            max_age_days: default_pruning_max_age_days(),
            max_sessions: default_pruning_max_sessions(),
            max_total_size_mb: default_pruning_max_total_size_mb(),
            check_interval_secs: default_pruning_check_interval_secs(),
            dry_run: false,
        }
    }
}

/// Session maintenance mode: enforce deletions or just warn
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceMode {
    /// Delete stale sessions and rotate oversized files
    #[default]
    Enforce,
    /// Log warnings but don't modify anything
    WarnOnly,
}

/// Session maintenance configuration — auto-prunes stale sessions on save
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMaintenanceConfig {
    /// Whether maintenance is enabled at all
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Mode: enforce (delete/rotate) or warn_only (log only)
    #[serde(default)]
    pub mode: MaintenanceMode,
    /// Maximum age of sessions in days (default: 30)
    #[serde(default = "default_pruning_max_age_days")]
    pub max_age_days: u32,
    /// Maximum number of sessions to keep (default: 500)
    #[serde(default = "default_maintenance_max_sessions")]
    pub max_sessions: usize,
    /// Maximum size in MB for a single session file before rotation (default: 10)
    #[serde(default = "default_maintenance_max_file_size_mb")]
    pub max_file_size_mb: u64,
}

fn default_maintenance_max_sessions() -> usize {
    500
}
fn default_maintenance_max_file_size_mb() -> u64 {
    10
}

impl Default for SessionMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: MaintenanceMode::default(),
            max_age_days: default_pruning_max_age_days(),
            max_sessions: default_maintenance_max_sessions(),
            max_file_size_mb: default_maintenance_max_file_size_mb(),
        }
    }
}

/// Image generation provider type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageGenProviderType {
    /// Fooocus API (SDXL Turbo) — `/v1/generation/text-to-image`
    Fooocus,
    /// OpenAI DALL-E API — `POST /v1/images/generations`
    OpenAi,
    /// Automatic1111 Stable Diffusion WebUI — `POST /sdapi/v1/txt2img`
    Automatic1111,
    /// ComfyUI workflow API — `POST /api/prompt`
    ComfyUi,
    /// Any OpenAI-compatible image API (custom URL + optional key)
    OpenAiCompatible,
}

impl Default for ImageGenProviderType {
    fn default() -> Self {
        Self::from_env()
    }
}

impl ImageGenProviderType {
    /// Resolve provider from `ZEUS_IMAGE_GEN_PROVIDER` env var, defaulting to Fooocus
    fn from_env() -> Self {
        match std::env::var("ZEUS_IMAGE_GEN_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "openai" | "dall-e" | "dalle" => Self::OpenAi,
            "automatic1111" | "a1111" | "sd-webui" => Self::Automatic1111,
            "comfyui" | "comfy" => Self::ComfyUi,
            "openai_compatible" | "openai-compatible" | "generic" => Self::OpenAiCompatible,
            _ => Self::Fooocus,
        }
    }
}

/// Image generation backend configuration
///
/// Supports pluggable providers: Fooocus, OpenAI DALL-E, Automatic1111,
/// ComfyUI, and any OpenAI-compatible image API. Configure via config.toml
/// or environment variables (`ZEUS_IMAGE_GEN_*`).
///
/// ```toml
/// [image_gen]
/// provider = "openai"
/// url = "https://api.openai.com"
/// api_key = "sk-..."
/// model = "dall-e-3"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenConfig {
    /// Provider type — determines API format and endpoints
    #[serde(default)]
    pub provider: ImageGenProviderType,
    /// Backend API URL — overridden by `ZEUS_IMAGE_GEN_URL` env var
    #[serde(default = "default_image_gen_url")]
    pub url: String,
    /// API key for cloud providers — overridden by `ZEUS_IMAGE_GEN_API_KEY` env var
    #[serde(default = "default_image_gen_api_key")]
    pub api_key: Option<String>,
    /// Model identifier (e.g., "dall-e-3", "sd-xl-turbo") — overridden by `ZEUS_IMAGE_GEN_MODEL`
    #[serde(default = "default_image_gen_model")]
    pub model: Option<String>,
    /// Default width in pixels
    #[serde(default = "default_image_width")]
    pub default_width: u32,
    /// Default height in pixels
    #[serde(default = "default_image_height")]
    pub default_height: u32,
    /// Directory to store generated images
    #[serde(default = "default_image_store_path")]
    pub store_path: PathBuf,
}

fn default_image_gen_url() -> String {
    std::env::var("ZEUS_IMAGE_GEN_URL")
        .or_else(|_| std::env::var("ZEUS_FOOOCUS_URL"))
        .unwrap_or_else(|_| "http://localhost:8888".to_string())
}
fn default_image_gen_api_key() -> Option<String> {
    std::env::var("ZEUS_IMAGE_GEN_API_KEY").ok()
}
fn default_image_gen_model() -> Option<String> {
    std::env::var("ZEUS_IMAGE_GEN_MODEL").ok()
}
fn default_image_width() -> u32 {
    1024
}
fn default_image_height() -> u32 {
    1024
}
fn default_image_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("images")
}

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            provider: ImageGenProviderType::default(),
            url: default_image_gen_url(),
            api_key: default_image_gen_api_key(),
            model: default_image_gen_model(),
            default_width: default_image_width(),
            default_height: default_image_height(),
            store_path: default_image_store_path(),
        }
    }
}

impl ImageGenConfig {
    pub fn default_store_path() -> PathBuf {
        default_image_store_path()
    }
}

/// Video generation backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenConfig {
    /// Backend API URL (ComfyUI + AnimateDiff)
    #[serde(default = "default_video_gen_url")]
    pub url: String,
    /// Default video duration in seconds
    #[serde(default = "default_video_duration")]
    pub default_duration: u32,
    /// Default frames per second
    #[serde(default = "default_video_fps")]
    pub default_fps: u32,
    /// Directory to store generated videos
    #[serde(default = "default_video_store_path")]
    pub store_path: PathBuf,
}

fn default_video_gen_url() -> String {
    std::env::var("ZEUS_COMFYUI_URL").unwrap_or_else(|_| "http://localhost:8188".to_string())
}
fn default_video_duration() -> u32 {
    4
}
fn default_video_fps() -> u32 {
    24
}
fn default_video_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("videos")
}

impl Default for VideoGenConfig {
    fn default() -> Self {
        Self {
            url: default_video_gen_url(),
            default_duration: default_video_duration(),
            default_fps: default_video_fps(),
            store_path: default_video_store_path(),
        }
    }
}

/// Deployment configuration — service URLs that vary per environment.
///
/// Centralizes URLs for external services so deployments don't need env vars
/// or hardcoded addresses. All values have sane localhost defaults.
///
/// ```toml
/// [deployment]
/// piper_tts_url = "http://localhost:8104"
/// kokoro_tts_url = "http://localhost:8880"
/// chrome_cdp_url = "http://localhost:9222"
/// whisper_stt_url = "http://localhost:8080/inference"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// Piper TTS server URL
    #[serde(default = "default_piper_tts_url")]
    pub piper_tts_url: String,
    /// Kokoro TTS server URL (OpenAI-compatible)
    #[serde(default = "default_kokoro_tts_url")]
    pub kokoro_tts_url: String,
    /// Chrome DevTools Protocol debug URL
    #[serde(default = "default_chrome_cdp_url")]
    pub chrome_cdp_url: String,
    /// Whisper STT inference URL (optional — used by ZeusWeb studio)
    #[serde(default = "default_whisper_stt_url")]
    pub whisper_stt_url: Option<String>,
    /// Voice webhook base URL (Twilio/inbound calls)
    #[serde(default = "default_webhook_url")]
    pub webhook_url: Option<String>,
    /// OTLP telemetry endpoint
    #[serde(default = "default_otlp_url")]
    pub otlp_url: Option<String>,
    /// ngrok API URL for tunnel management
    #[serde(default = "default_ngrok_url")]
    pub ngrok_url: Option<String>,
}

fn default_piper_tts_url() -> String {
    "http://localhost:8104".to_string()
}
fn default_kokoro_tts_url() -> String {
    "http://localhost:8880".to_string()
}
fn default_chrome_cdp_url() -> String {
    "http://localhost:9222".to_string()
}
fn default_whisper_stt_url() -> Option<String> {
    None
}
fn default_webhook_url() -> Option<String> {
    None
}
fn default_otlp_url() -> Option<String> {
    None
}
fn default_ngrok_url() -> Option<String> {
    None
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            piper_tts_url: default_piper_tts_url(),
            kokoro_tts_url: default_kokoro_tts_url(),
            chrome_cdp_url: default_chrome_cdp_url(),
            whisper_stt_url: default_whisper_stt_url(),
            webhook_url: default_webhook_url(),
            otlp_url: default_otlp_url(),
            ngrok_url: default_ngrok_url(),
        }
    }
}

impl DeploymentConfig {
    /// Export all resolved service URLs as `ZEUS_*` environment variables.
    ///
    /// Called at gateway startup so downstream crates and child processes
    /// inherit the correct URLs without reading config.toml themselves.
    /// # Safety
    ///
    /// This is called once at gateway startup before any multithreaded work begins.
    /// `set_var` is unsafe because it's not thread-safe, but at boot time only the
    /// main thread is running.
    pub fn export_env_vars(&self) {
        // SAFETY: Called at single-threaded gateway startup before spawning tasks.
        unsafe {
            std::env::set_var("ZEUS_PIPER_URL", &self.piper_tts_url);
            std::env::set_var("ZEUS_KOKORO_URL", &self.kokoro_tts_url);
            std::env::set_var("ZEUS_CDP_URL", &self.chrome_cdp_url);
            if let Some(ref url) = self.whisper_stt_url {
                std::env::set_var("ZEUS_WHISPER_URL", url);
            }
            if let Some(ref url) = self.webhook_url {
                std::env::set_var("ZEUS_WEBHOOK_URL", url);
            }
            if let Some(ref url) = self.otlp_url {
                std::env::set_var("ZEUS_OTLP_URL", url);
            }
            if let Some(ref url) = self.ngrok_url {
                std::env::set_var("ZEUS_NGROK_URL", url);
            }
        }
    }
}

/// Star Office Pantheon room configuration.
///
/// The gateway reads this on startup and auto-joins the Star Office Pantheon room.
///
/// ```toml
/// [star_office]
/// room_id = "!abc123:pantheon.zeus.local"
/// auto_idle_secs = 300
/// ```
// ── Pantheon IRC Client Config ──────────────────────────────────────────────

/// Configuration for connecting to a standalone Pantheon IRC server.
///
/// ```toml
/// [pantheon]
/// server = "192.168.1.100:7777"
/// nick = "zeus100"
/// channel_key = "fleet-key-here"
/// auto_join = ["#general", "#ops", "#builds"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PantheonClientConfig {
    /// Pantheon server address (host:port)
    pub server: String,
    /// This agent's nick/user_id
    #[serde(default = "default_pantheon_nick")]
    pub nick: String,
    /// Shared channel key for authentication
    #[serde(default)]
    pub channel_key: String,
    /// Channels to auto-join after auth
    #[serde(default = "default_pantheon_auto_join")]
    pub auto_join: Vec<String>,
    /// Whether this is an AI agent (sets agent=true in AUTH)
    #[serde(default = "default_true")]
    pub is_agent: bool,
    /// Reconnect delay in seconds on disconnect
    #[serde(default = "default_pantheon_reconnect_secs")]
    pub reconnect_secs: u64,
    /// Forward Discord/Telegram messages to this Pantheon channel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_channel: Option<String>,
}

fn default_pantheon_nick() -> String {
    "zeus-agent".into()
}
fn default_pantheon_auto_join() -> Vec<String> {
    vec!["#general".into(), "#ops".into()]
}
fn default_pantheon_reconnect_secs() -> u64 { 5 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarOfficeConfig {
    /// Pantheon room ID for the Star Office (e.g. "!abc123:pantheon.zeus.local")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,

    /// Seconds of heartbeat silence before an agent is considered idle (default: 300)
    #[serde(default = "default_auto_idle_secs")]
    pub auto_idle_secs: u64,
}

fn default_auto_idle_secs() -> u64 {
    300
}

impl Default for StarOfficeConfig {
    fn default() -> Self {
        Self {
            room_id: None,
            auto_idle_secs: default_auto_idle_secs(),
        }
    }
}

/// Economy configuration — agent earning formula parameters.
///
/// Controls the credit reward formula:
/// `earning = base + (tools_used × tool_bonus) + complexity_bonus`
///
/// ```toml
/// [economy]
/// earning_base = 10
/// earning_tool_bonus = 2
/// earning_moderate_bonus = 10
/// earning_complex_bonus = 25
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomyConfig {
    /// Base credits earned per task completion
    #[serde(default = "default_earning_base")]
    pub earning_base: u64,
    /// Bonus credits per tool used during task
    #[serde(default = "default_earning_tool_bonus")]
    pub earning_tool_bonus: u64,
    /// Bonus credits for moderate-complexity tasks
    #[serde(default = "default_earning_moderate_bonus")]
    pub earning_moderate_bonus: u64,
    /// Bonus credits for complex tasks
    #[serde(default = "default_earning_complex_bonus")]
    pub earning_complex_bonus: u64,
}

fn default_earning_base() -> u64 {
    10
}
fn default_earning_tool_bonus() -> u64 {
    2
}
fn default_earning_moderate_bonus() -> u64 {
    10
}
fn default_earning_complex_bonus() -> u64 {
    25
}

impl Default for EconomyConfig {
    fn default() -> Self {
        Self {
            earning_base: default_earning_base(),
            earning_tool_bonus: default_earning_tool_bonus(),
            earning_moderate_bonus: default_earning_moderate_bonus(),
            earning_complex_bonus: default_earning_complex_bonus(),
        }
    }
}

/// Context overflow recovery configuration.
///
/// Controls pre-emptive tool result capping, progressive summarization,
/// and forced session rotation to prevent hitting LLM context limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverflowRecoveryConfig {
    /// Maximum context window in tokens.
    #[serde(default = "default_overflow_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Maximum characters per single tool result before truncation.
    #[serde(default = "default_overflow_tool_result_cap_chars")]
    pub tool_result_cap_chars: usize,
    /// Percentage of capacity at which to start warning / capping tool results.
    #[serde(default = "default_overflow_warning_threshold_pct")]
    pub warning_threshold_pct: u8,
    /// Percentage of capacity at which to start summarizing older messages.
    #[serde(default = "default_overflow_summarize_threshold_pct")]
    pub summarize_threshold_pct: u8,
    /// Percentage of capacity at which to force session rotation.
    #[serde(default = "default_overflow_critical_threshold_pct")]
    pub critical_threshold_pct: u8,
    /// Number of recent messages to always preserve during summarization.
    #[serde(default = "default_overflow_keep_recent_messages")]
    pub keep_recent_messages: usize,
    /// Maximum characters for a summary block.
    #[serde(default = "default_overflow_summary_max_chars")]
    pub summary_max_chars: usize,
}

fn default_overflow_max_context_tokens() -> usize {
    100_000
}
fn default_overflow_tool_result_cap_chars() -> usize {
    50_000
}
fn default_overflow_warning_threshold_pct() -> u8 {
    80
}
fn default_overflow_summarize_threshold_pct() -> u8 {
    85
}
fn default_overflow_critical_threshold_pct() -> u8 {
    95
}
fn default_overflow_keep_recent_messages() -> usize {
    10
}
fn default_overflow_summary_max_chars() -> usize {
    2_000
}

impl Default for OverflowRecoveryConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: default_overflow_max_context_tokens(),
            tool_result_cap_chars: default_overflow_tool_result_cap_chars(),
            warning_threshold_pct: default_overflow_warning_threshold_pct(),
            summarize_threshold_pct: default_overflow_summarize_threshold_pct(),
            critical_threshold_pct: default_overflow_critical_threshold_pct(),
            keep_recent_messages: default_overflow_keep_recent_messages(),
            summary_max_chars: default_overflow_summary_max_chars(),
        }
    }
}

/// WebSocket Ed25519 authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsAuthConfig {
    /// Enable the v3 Ed25519 challenge-response handshake
    #[serde(default)]
    pub enabled: bool,
    /// Path to the PKCS8 key file (default: ~/.zeus/ws_ed25519.key)
    #[serde(default = "default_ws_auth_key_path")]
    pub key_path: PathBuf,
    /// Allowed clock skew in seconds (default: 30)
    #[serde(default = "default_ws_auth_tolerance")]
    pub timestamp_tolerance_secs: u64,
}

fn default_ws_auth_key_path() -> PathBuf {
    default_config_dir().join("ws_ed25519.key")
}

fn default_ws_auth_tolerance() -> u64 {
    30
}

impl Default for WsAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            key_path: default_ws_auth_key_path(),
            timestamp_tolerance_secs: default_ws_auth_tolerance(),
        }
    }
}

/// MCP server behavior configuration (detailed settings beyond gateway on/off + port)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// CORS allowed origins (empty = default localhost only)
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Maximum concurrent MCP connections
    #[serde(default = "default_mcp_max_connections")]
    pub max_connections: usize,
    /// Expose Talos macOS automation tools via MCP
    #[serde(default = "default_true")]
    pub enable_talos: bool,
    /// Expose agent spawn/list/status tools via MCP
    #[serde(default)]
    pub enable_agents: bool,
    /// Bearer token for MCP server authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Expose graph memory tools (memory_graph, memory_communities, memory_graph_search) via MCP
    #[serde(default = "default_true")]
    pub enable_mnemosyne: bool,
}

fn default_mcp_max_connections() -> usize {
    32
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            max_connections: default_mcp_max_connections(),
            enable_talos: true,
            enable_agents: false,
            auth_token: None,
            enable_mnemosyne: true,
        }
    }
}

/// Context journal configuration — captures structured task state before compaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextJournalConfig {
    /// Whether the context journal is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Remaining context % that triggers journal capture (default 10)
    #[serde(default = "default_journal_threshold_pct")]
    pub threshold_pct: u8,
    /// Directory for journal files, relative to ~/.zeus/ (default "context-journals")
    #[serde(default = "default_journal_path")]
    pub path: String,
}

fn default_journal_threshold_pct() -> u8 {
    10
}
fn default_journal_path() -> String {
    "context-journals".to_string()
}

impl Default for ContextJournalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_pct: default_journal_threshold_pct(),
            path: default_journal_path(),
        }
    }
}

/// DM access policy
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DmPolicy {
    #[default]
    Open,
    Allowlist,
    Pairing,
    Disabled,
}

/// Group access policy
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GroupPolicy {
    /// Respond to all group messages (default — matches OpenClaw/molty behavior).
    /// Set `policy.group = "mentiononly"` in config.toml to restrict to @mentions only.
    #[default]
    Open,
    Allowlist,
    MentionOnly,
    Disabled,
}

fn default_group_policy() -> GroupPolicy {
    match std::env::var("ZEUS_DEFAULT_GROUP_POLICY")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "mentiononly" | "mention_only" => GroupPolicy::MentionOnly,
        "allowlist" => GroupPolicy::Allowlist,
        "disabled" => GroupPolicy::Disabled,
        _ => GroupPolicy::Open,
    }
}

fn default_allow_bots_policy() -> Option<String> {
    std::env::var("ZEUS_DEFAULT_ALLOW_BOTS")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Channel access policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPolicyConfig {
    /// DM access policy
    #[serde(default)]
    pub dm: DmPolicy,
    /// Group access policy.
    /// Defaults to `GroupPolicy::MentionOnly` unless `ZEUS_DEFAULT_GROUP_POLICY=open`
    /// (or another value) is set in the environment, or `policy.group` is set in config.toml.
    #[serde(default = "default_group_policy")]
    pub group: GroupPolicy,
    /// Allowed user IDs / usernames
    #[serde(default)]
    pub allow_from: Vec<String>,
    /// Allowed group/channel IDs
    #[serde(default)]
    pub allow_groups: Vec<String>,
    /// Restrict tools available from this channel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_allowlist: Option<Vec<String>>,
    /// Restrict commands available from this channel.
    /// Commands are slash-style operations like "status", "memory", "config".
    /// None means all commands allowed. Some(vec![]) means no commands allowed.
    /// Supports "*" wildcard for all commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands_allowlist: Option<Vec<String>>,
}

impl Default for ChannelPolicyConfig {
    fn default() -> Self {
        Self {
            dm: DmPolicy::default(),
            group: default_group_policy(),
            allow_from: Vec::new(),
            allow_groups: Vec::new(),
            tools_allowlist: None,
            commands_allowlist: None,
        }
    }
}

// ============================================================================
// Agent Routing & Tool Policy
// ============================================================================

/// Binding rule for matching inbound messages to agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value")]
#[serde(rename_all = "snake_case")]
pub enum BindingRule {
    /// Match by user_id (DM-level routing)
    Peer(String),
    /// Match by chat_id (group-level routing)
    Guild(String),
    /// Match by channel_type (platform-level, e.g. "telegram")
    Team(String),
    /// Match by agent_id (inter-agent routing, not channel routing)
    Account(String),
}

impl BindingRule {
    /// Check if this rule matches the given channel message metadata.
    /// Account rules never match channel messages (they are for inter-agent routing).
    pub fn matches(&self, channel_type: &str, user_id: &str, chat_id: &str) -> bool {
        match self {
            BindingRule::Peer(id) => user_id == id,
            BindingRule::Guild(id) => chat_id == id,
            BindingRule::Team(ct) => channel_type == ct,
            BindingRule::Account(_) => false,
        }
    }
}

/// Per-agent tool access policy.
///
/// Deny takes precedence over allow. Empty `allowed_tools` means all tools are
/// allowed (unless explicitly denied). Supports wildcards: `"*"` matches
/// everything, `"prefix*"` matches any tool starting with `prefix`,
/// `"ns.*"` matches any tool starting with `ns.`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentToolPolicy {
    /// Whitelist — empty means all allowed (subject to deny list).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Blacklist — deny takes precedence over allow.
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl AgentToolPolicy {
    /// Check whether `tool_name` is permitted under this policy.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        // Deny-first: if any deny pattern matches, reject
        if self
            .denied_tools
            .iter()
            .any(|p| Self::pattern_matches(p, tool_name))
        {
            return false;
        }
        // If allow list is empty, everything (not denied) is allowed
        if self.allowed_tools.is_empty() {
            return true;
        }
        // Otherwise, must match at least one allow pattern
        self.allowed_tools
            .iter()
            .any(|p| Self::pattern_matches(p, tool_name))
    }

    /// Simple wildcard pattern matching.
    /// - `"*"` matches everything
    /// - `"prefix*"` matches any string starting with `prefix`
    /// - exact match otherwise
    fn pattern_matches(pattern: &str, name: &str) -> bool {
        if pattern == "*" {
            true
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            pattern == name
        }
    }
}

/// Combines binding rules with a tool policy for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    /// Agent identifier
    pub agent_id: String,
    /// Rules that determine when this agent handles a message
    #[serde(default)]
    pub bindings: Vec<BindingRule>,
    /// Per-agent tool access control
    #[serde(default)]
    pub tool_policy: AgentToolPolicy,
    /// Priority for routing (higher = checked first, default 0)
    #[serde(default)]
    pub priority: i32,
}

/// Per-account WhatsApp configuration for multi-account support.
///
/// Each entry in `WhatsAppChannelConfig.accounts` spawns a dedicated adapter
/// instance with its own bridge/API connection.
///
/// TOML example:
/// ```toml
/// [channels.whatsapp.accounts.support]
/// bridge_url = "ws://localhost:3002"
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WhatsAppAccountConfig {
    /// Operating mode override: "bridge" or "cloud_api"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// WebSocket bridge URL (Bridge mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_url: Option<String>,
    /// Meta Graph API access token (Cloud API mode)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Phone number ID (Cloud API mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number_id: Option<String>,
    /// Phone number associated with this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// WhatsApp channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppChannelConfig {
    /// URL of the Baileys WebSocket bridge. Falls back to WHATSAPP_BRIDGE_URL env var.
    #[serde(default = "default_whatsapp_bridge")]
    pub bridge_url: String,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account WhatsApp configurations.
    /// Each entry spawns a separate adapter instance.
    #[serde(default)]
    pub accounts: HashMap<String, WhatsAppAccountConfig>,
    /// Global bot message policy (overridden per-account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

fn default_whatsapp_bridge() -> String {
    std::env::var("ZEUS_WHATSAPP_BRIDGE_URL")
        .or_else(|_| std::env::var("WHATSAPP_BRIDGE_URL"))
        .unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

/// Per-account Signal configuration for multi-account support.
///
/// Each entry in `SignalChannelConfig.accounts` spawns a dedicated signal-cli
/// subprocess with its own phone number identity.
///
/// TOML example:
/// ```toml
/// [channels.signal.accounts.support]
/// phone = "+15551234567"
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignalAccountConfig {
    /// Path to signal-cli binary (overrides top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_cli_path: Option<String>,
    /// Phone number registered with Signal
    #[serde(default)]
    pub phone: String,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Signal channel configuration
///
/// Falls back to `SIGNAL_CLI_PATH` and `SIGNAL_ACCOUNT` env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalChannelConfig {
    /// Path to signal-cli binary
    #[serde(default = "default_signal_cli_path")]
    pub signal_cli_path: String,
    /// Phone number
    #[serde(default = "default_signal_account")]
    pub phone: String,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account Signal configurations.
    /// Each entry spawns a separate signal-cli subprocess.
    #[serde(default)]
    pub accounts: HashMap<String, SignalAccountConfig>,
    /// Global bot message policy (overridden per-account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

fn default_signal_cli_path() -> String {
    std::env::var("SIGNAL_CLI_PATH").unwrap_or_default()
}

fn default_signal_account() -> String {
    std::env::var("SIGNAL_ACCOUNT").unwrap_or_default()
}

fn default_signal_http_host() -> String {
    "127.0.0.1".to_string()
}

fn default_signal_http_port() -> u16 {
    8088
}

/// Per-account Matrix configuration for multi-account support.
///
/// Each entry in `MatrixChannelConfig.accounts` spawns a dedicated Matrix
/// client with its own credentials and room list.
///
/// TOML example:
/// ```toml
/// [channels.matrix.accounts.support]
/// homeserver = "https://matrix.org"
/// username = "@support:matrix.org"
/// password = "secret"
/// rooms = ["!room1:matrix.org"]
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatrixAccountConfig {
    /// Homeserver URL (overrides top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homeserver: Option<String>,
    /// Username
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Password
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Access token (alternative to password)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// User ID for token-based auth
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Rooms to join/monitor
    #[serde(default)]
    pub rooms: Vec<String>,
    /// Display name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Matrix channel configuration
///
/// Falls back to `MATRIX_HOMESERVER`, `MATRIX_ACCESS_TOKEN`, `MATRIX_USER`,
/// `MATRIX_PASSWORD` env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixChannelConfig {
    /// Homeserver URL (e.g., "https://matrix.org")
    #[serde(default = "default_matrix_homeserver")]
    pub homeserver: String,
    /// Access token (alternative to password login)
    #[serde(default = "default_matrix_access_token", skip_serializing)]
    pub access_token: String,
    /// Username (e.g., "@bot:matrix.org" or just "bot")
    #[serde(default = "default_matrix_user")]
    pub username: Option<String>,
    /// Password for password-based login
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// User ID for token-based auth (e.g., "@bot:matrix.org")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Rooms to join/monitor (room IDs or aliases)
    #[serde(default)]
    pub rooms: Vec<String>,
    /// Display name to set after login
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account Matrix configurations.
    /// Each entry spawns a separate Matrix client.
    #[serde(default)]
    pub accounts: HashMap<String, MatrixAccountConfig>,
    /// Global bot message policy (overridden per-account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

fn default_matrix_homeserver() -> String {
    std::env::var("MATRIX_HOMESERVER").unwrap_or_default()
}

fn default_matrix_access_token() -> String {
    std::env::var("MATRIX_ACCESS_TOKEN").unwrap_or_default()
}

fn default_matrix_user() -> Option<String> {
    std::env::var("MATRIX_USER").ok().filter(|s| !s.is_empty())
}

/// MQTT channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttChannelConfig {
    /// Broker hostname or IP (e.g., "192.168.1.100" or "mqtt.example.com")
    pub broker_url: String,
    /// Broker port (default: 1883)
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    /// MQTT client ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Topic prefix (e.g., "zeus/")
    #[serde(default)]
    pub topic_prefix: String,
    /// QoS level: 0, 1, or 2
    #[serde(default = "default_mqtt_qos")]
    pub qos: u8,
    /// Topics to subscribe to
    #[serde(default)]
    pub subscribe_topics: Vec<String>,
    /// Broker username
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Broker password
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_mqtt_qos() -> u8 {
    1
}

/// Mattermost channel configuration
///
/// Used for Mattermost integration under `[channels.mattermost]`.
/// Env var fallbacks: `MATTERMOST_URL`, `MATTERMOST_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MattermostChannelConfig {
    /// Mattermost server URL (e.g., "https://mattermost.example.com")
    #[serde(default = "default_mattermost_channel_url")]
    pub server_url: String,
    /// Personal access token or bot token
    #[serde(default = "default_mattermost_channel_token", skip_serializing)]
    pub token: String,
    /// Team ID (optional, for team-specific operations)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_mattermost_channel_url() -> String {
    std::env::var("MATTERMOST_URL").unwrap_or_default()
}

fn default_mattermost_channel_token() -> String {
    std::env::var("MATTERMOST_TOKEN").unwrap_or_default()
}

/// IRC channel configuration
///
/// Used for IRC integration under `[channels.irc]`.
/// Env var fallbacks: `IRC_SERVER`, `IRC_NICK`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrcChannelConfig {
    /// IRC server hostname (e.g. "irc.libera.chat")
    #[serde(default = "default_irc_server")]
    pub server: String,
    /// Port (default: 6667 for plain, 6697 for TLS)
    #[serde(default = "default_irc_port")]
    pub port: u16,
    /// Bot nickname
    #[serde(default = "default_irc_nick")]
    pub nick: String,
    /// Optional username (defaults to nick)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Channels to join on connect (e.g. ["#zeus", "#dev"])
    #[serde(default)]
    pub channels: Vec<String>,
    /// Use TLS (default: false)
    #[serde(default)]
    pub use_tls: bool,
    /// NickServ password (optional, for registered nicks)
    #[serde(default, skip_serializing)]
    pub nickserv_password: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

fn default_irc_server() -> String {
    std::env::var("IRC_SERVER").unwrap_or_default()
}

fn default_irc_port() -> u16 {
    6667
}

fn default_irc_nick() -> String {
    std::env::var("IRC_NICK").unwrap_or_default()
}

/// X (Twitter) channel configuration
///
/// Used for X/Twitter integration under `[channels.x_twitter]`.
/// Env var fallbacks: `X_BEARER_TOKEN`, `X_CONSUMER_KEY`,
/// `X_CONSUMER_KEY_SECRET`, `X_ACCESS_TOKEN`, `X_ACCESS_TOKEN_SECRET`,
/// `X_CLIENT_ID`, `X_CLIENT_SECRET` (legacy `X_API_KEY` / `X_API_SECRET`
/// are still honored as fallbacks for existing installs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XTwitterChannelConfig {
    /// Bearer token (OAuth 2.0 App-Only)
    #[serde(default = "default_x_bearer_token")]
    pub bearer_token: String,
    /// Consumer key (OAuth 1.0a)
    #[serde(default = "default_x_consumer_key", alias = "api_key")]
    pub consumer_key: String,
    /// Consumer key secret (OAuth 1.0a)
    #[serde(default = "default_x_consumer_key_secret", alias = "api_secret")]
    pub consumer_key_secret: String,
    /// Access token (OAuth 1.0a)
    #[serde(default = "default_x_access_token")]
    pub access_token: String,
    /// Access token secret (OAuth 1.0a)
    #[serde(default = "default_x_access_token_secret")]
    pub access_token_secret: String,
    /// OAuth 2.0 Client ID (for PKCE flow)
    #[serde(default = "default_x_client_id")]
    pub client_id: String,
    /// OAuth 2.0 Client Secret (for PKCE flow)
    #[serde(default = "default_x_client_secret")]
    pub client_secret: String,
    /// Polling interval for mentions in seconds (default: 60)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    /// Auto-reply to mentions (default: false)
    #[serde(default)]
    pub auto_reply: bool,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Fan-out targets: repost polled X content to configured TG/Discord
    /// channels. Each target names a destination channel + chat/channel ID.
    /// Empty (default) = no fan-out (pure inbound-relay behavior, unchanged).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fanout: Vec<FanoutTarget>,
}

/// A single fan-out destination for X→channel reposting (#100).
///
/// Routes polled X content to a configured messaging channel. The `channel`
/// field selects the outbound adapter (`"telegram"` or `"discord"`), and
/// `chat_id` is the destination chat/channel ID within that platform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FanoutTarget {
    /// Destination channel type: `"telegram"` or `"discord"`.
    pub channel: String,
    /// Destination chat/channel ID within the target platform.
    pub chat_id: String,
}

impl FanoutTarget {
    /// Normalized channel type (lowercased, trimmed) for matching.
    pub fn channel_kind(&self) -> String {
        self.channel.trim().to_ascii_lowercase()
    }

    /// Whether this target names a supported outbound channel.
    pub fn is_supported(&self) -> bool {
        matches!(self.channel_kind().as_str(), "telegram" | "discord")
            && !self.chat_id.trim().is_empty()
    }
}

fn default_x_bearer_token() -> String {
    std::env::var("X_BEARER_TOKEN").unwrap_or_default()
}

fn default_x_consumer_key() -> String {
    std::env::var("X_CONSUMER_KEY")
        .or_else(|_| std::env::var("X_API_KEY"))
        .unwrap_or_default()
}

fn default_x_consumer_key_secret() -> String {
    std::env::var("X_CONSUMER_KEY_SECRET")
        .or_else(|_| std::env::var("X_API_SECRET"))
        .unwrap_or_default()
}

fn default_x_access_token() -> String {
    std::env::var("X_ACCESS_TOKEN").unwrap_or_default()
}

fn default_x_access_token_secret() -> String {
    std::env::var("X_ACCESS_TOKEN_SECRET").unwrap_or_default()
}

fn default_x_client_id() -> String {
    std::env::var("X_CLIENT_ID").unwrap_or_default()
}

fn default_x_client_secret() -> String {
    std::env::var("X_CLIENT_SECRET").unwrap_or_default()
}

/// Instagram channel configuration
///
/// Used for Instagram Graph API integration under `[channels.instagram]`.
/// Env var fallbacks: `INSTAGRAM_ACCESS_TOKEN`, `INSTAGRAM_ACCOUNT_ID`,
/// `INSTAGRAM_PAGE_ID`, `INSTAGRAM_APP_ID`, `INSTAGRAM_APP_SECRET`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstagramConfig {
    /// Meta Graph API access token (long-lived)
    #[serde(default = "default_instagram_access_token")]
    pub access_token: String,
    /// Instagram Business/Creator Account ID
    #[serde(default = "default_instagram_account_id")]
    pub account_id: String,
    /// Facebook Page ID (required for some operations)
    #[serde(default = "default_instagram_page_id", skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// App ID (for token refresh)
    #[serde(default = "default_instagram_app_id", skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// App secret (for token refresh)
    #[serde(default = "default_instagram_app_secret", skip_serializing_if = "Option::is_none")]
    pub app_secret: Option<String>,
    /// Polling interval for comments in seconds (default: 120)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    /// Auto-reply to comments (default: false)
    #[serde(default)]
    pub auto_reply: bool,
}

fn default_instagram_access_token() -> String {
    std::env::var("INSTAGRAM_ACCESS_TOKEN").unwrap_or_default()
}

fn default_instagram_account_id() -> String {
    std::env::var("INSTAGRAM_ACCOUNT_ID").unwrap_or_default()
}

fn default_instagram_page_id() -> Option<String> {
    std::env::var("INSTAGRAM_PAGE_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

fn default_instagram_app_id() -> Option<String> {
    std::env::var("INSTAGRAM_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

fn default_instagram_app_secret() -> Option<String> {
    std::env::var("INSTAGRAM_APP_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

/// TikTok channel configuration
///
/// Used for TikTok Content Posting API integration under `[channels.tiktok]`.
/// Env var fallback: `TIKTOK_ACCESS_TOKEN`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TikTokConfig {
    /// OAuth2 user access token with `video.publish` scope.
    #[serde(default = "default_tiktok_access_token")]
    pub access_token: String,
}

fn default_tiktok_access_token() -> String {
    std::env::var("TIKTOK_ACCESS_TOKEN").unwrap_or_default()
}

/// Twitch channel configuration
///
/// Used for Twitch chat integration under `[channels.twitch]`.
/// Env var fallbacks: `TWITCH_OAUTH_TOKEN`, `TWITCH_USERNAME`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchChannelConfig {
    /// OAuth token (without the 'oauth:' prefix)
    #[serde(default = "default_twitch_oauth_token", skip_serializing)]
    pub oauth_token: String,
    /// Bot username
    #[serde(default = "default_twitch_username")]
    pub username: String,
    /// Channels to join (without '#' prefix)
    #[serde(default)]
    pub channels: Vec<String>,
    /// Client ID (for Helix API, optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_twitch_oauth_token() -> String {
    std::env::var("TWITCH_OAUTH_TOKEN").unwrap_or_default()
}

fn default_twitch_username() -> String {
    std::env::var("TWITCH_USERNAME").unwrap_or_default()
}

/// Microsoft Teams channel configuration
///
/// Used for Teams integration via Microsoft Graph API under `[channels.teams]`.
/// Env var fallback: `TEAMS_CLIENT_SECRET`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsChannelConfig {
    /// Azure AD tenant ID
    #[serde(default)]
    pub tenant_id: String,
    /// Azure AD application (client) ID
    #[serde(default)]
    pub client_id: String,
    /// Azure AD client secret
    #[serde(default = "default_teams_client_secret", skip_serializing)]
    pub client_secret: String,
    /// Microsoft Teams team ID
    #[serde(default)]
    pub team_id: String,
    /// Teams channel ID within the team
    #[serde(default)]
    pub channel_id: String,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_teams_client_secret() -> String {
    std::env::var("TEAMS_CLIENT_SECRET").unwrap_or_default()
}

/// WebChat channel configuration
///
/// Embedded WebSocket chat served by the gateway under `[channels.webchat]`.
/// Env var fallback: `WEBCHAT_AUTH_TOKEN`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebChatChannelConfig {
    /// WebSocket path (default: /ws/chat)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_path: Option<String>,
    /// Optional authentication token
    #[serde(default = "default_webchat_auth_token", skip_serializing)]
    pub auth_token: Option<String>,
    /// CORS allowed origins
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Maximum message size in bytes (0 = adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_message_size: Option<usize>,
    /// Connection timeout in seconds (0 = adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_timeout_secs: Option<u64>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_webchat_auth_token() -> Option<String> {
    std::env::var("WEBCHAT_AUTH_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Google Chat channel configuration
///
/// Used for Google Chat via service-account auth under `[channels.googlechat]`.
/// Env var fallback: `GOOGLE_CHAT_CREDENTIALS_PATH`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoogleChatChannelConfig {
    /// Service account key JSON (inline)
    #[serde(default, skip_serializing)]
    pub service_account_key: String,
    /// Path to service-account credentials file
    #[serde(default = "default_googlechat_credentials_path")]
    pub credentials_path: Option<String>,
    /// Pre-configured access token (development only)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Webhook path for receiving messages
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Google Cloud project ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_googlechat_credentials_path() -> Option<String> {
    std::env::var("GOOGLE_CHAT_CREDENTIALS_PATH").ok().filter(|s| !s.is_empty())
}

/// Nextcloud Talk channel configuration
///
/// Used for Nextcloud Talk under `[channels.nextcloud]`.
/// Env var fallbacks: `NEXTCLOUD_PASSWORD`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NextcloudChannelConfig {
    /// Server URL (e.g., https://cloud.example.com)
    #[serde(default)]
    pub server_url: String,
    /// Username
    #[serde(default)]
    pub username: String,
    /// Password or app password
    #[serde(default = "default_nextcloud_password", skip_serializing)]
    pub password: String,
    /// Poll interval in seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_nextcloud_password() -> String {
    std::env::var("NEXTCLOUD_PASSWORD").unwrap_or_default()
}

/// Nostr channel configuration
///
/// Used for the decentralized Nostr protocol under `[channels.nostr]`.
/// Env var fallbacks: `NOSTR_PRIVATE_KEY`, `NOSTR_NSEC`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrChannelConfig {
    /// Private key (hex)
    #[serde(default = "default_nostr_private_key", skip_serializing)]
    pub private_key: String,
    /// Private key (nsec bech32 format) — alternative to `private_key`
    #[serde(default = "default_nostr_nsec", skip_serializing)]
    pub nsec: Option<String>,
    /// Public key (hex, derived from private key if omitted)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    /// Relay URLs (wss:// or ws://)
    #[serde(default, alias = "relays")]
    pub relay_urls: Vec<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_nostr_private_key() -> String {
    std::env::var("NOSTR_PRIVATE_KEY").unwrap_or_default()
}

fn default_nostr_nsec() -> Option<String> {
    std::env::var("NOSTR_NSEC").ok().filter(|s| !s.is_empty())
}

/// LINE channel configuration
///
/// Used for the LINE Messaging API under `[channels.line]`.
/// Env var fallbacks: `LINE_CHANNEL_ACCESS_TOKEN`, `LINE_CHANNEL_SECRET`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineChannelConfig {
    /// Channel access token (from LINE Developers console)
    #[serde(default = "default_line_channel_access_token", skip_serializing)]
    pub channel_access_token: String,
    /// Channel secret (for webhook signature validation)
    #[serde(default = "default_line_channel_secret", skip_serializing)]
    pub channel_secret: Option<String>,
    /// Webhook path (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_line_channel_access_token() -> String {
    std::env::var("LINE_CHANNEL_ACCESS_TOKEN").unwrap_or_default()
}

fn default_line_channel_secret() -> Option<String> {
    std::env::var("LINE_CHANNEL_SECRET").ok().filter(|s| !s.is_empty())
}

/// Feishu (Lark) channel configuration
///
/// Used for Feishu/Lark bots under `[channels.feishu]`.
/// Env var fallback: `FEISHU_APP_SECRET`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuChannelConfig {
    /// App ID (from Feishu open platform)
    #[serde(default)]
    pub app_id: String,
    /// App secret
    #[serde(default = "default_feishu_app_secret", skip_serializing)]
    pub app_secret: String,
    /// Encrypt key (for event payload decryption)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypt_key: Option<String>,
    /// Verification token (for event verification)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_token: Option<String>,
    /// Webhook path (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Use Lark (international) API endpoints instead of Feishu (China)
    #[serde(default)]
    pub use_lark: bool,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_feishu_app_secret() -> String {
    std::env::var("FEISHU_APP_SECRET").unwrap_or_default()
}

/// Zalo channel configuration
///
/// Used for Zalo Official Account bots under `[channels.zalo]`.
/// Env var fallbacks: `ZALO_SECRET_KEY`, `ZALO_ACCESS_TOKEN`, `ZALO_REFRESH_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaloChannelConfig {
    /// App ID (from Zalo developer console)
    #[serde(default)]
    pub app_id: String,
    /// App secret key
    #[serde(default = "default_zalo_secret_key", skip_serializing)]
    pub secret_key: String,
    /// OA access token
    #[serde(default = "default_zalo_access_token", skip_serializing)]
    pub access_token: Option<String>,
    /// OA refresh token
    #[serde(default = "default_zalo_refresh_token", skip_serializing)]
    pub refresh_token: Option<String>,
    /// Webhook path (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_zalo_secret_key() -> String {
    std::env::var("ZALO_SECRET_KEY").unwrap_or_default()
}

fn default_zalo_access_token() -> Option<String> {
    std::env::var("ZALO_ACCESS_TOKEN").ok().filter(|s| !s.is_empty())
}

fn default_zalo_refresh_token() -> Option<String> {
    std::env::var("ZALO_REFRESH_TOKEN").ok().filter(|s| !s.is_empty())
}

/// BlueBubbles channel configuration (#316 P3 batch-2b)
///
/// iMessage via a BlueBubbles server (cross-platform alternative to the
/// macOS-only AppleScript bridge). Used under `[channels.bluebubbles]`.
/// Env var fallback: `BLUEBUBBLES_PASSWORD`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueBubblesChannelConfig {
    /// BlueBubbles server URL (e.g. http://localhost:1234)
    #[serde(default)]
    pub server_url: String,
    /// Server password
    #[serde(default = "default_bluebubbles_password", skip_serializing)]
    pub password: String,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_bluebubbles_password() -> String {
    std::env::var("BLUEBUBBLES_PASSWORD").unwrap_or_default()
}

/// SMS channel configuration (Twilio) (#316 P3 batch-2b)
///
/// Used under `[channels.sms]`.
/// Env var fallback: `TWILIO_AUTH_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsChannelConfig {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,
    /// Twilio Auth Token
    #[serde(default = "default_twilio_auth_token", skip_serializing)]
    pub auth_token: String,
    /// Phone number to send from (E.164 format, e.g. "+14155551234")
    #[serde(default)]
    pub from_number: String,
    /// Webhook path for inbound SMS (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_twilio_auth_token() -> String {
    std::env::var("TWILIO_AUTH_TOKEN").unwrap_or_default()
}

/// Twilio WhatsApp channel configuration (#316 P3 batch-2b)
///
/// WhatsApp via Twilio's API (alternative to the Meta Cloud API `whatsapp`
/// channel). Used under `[channels.twilio_whatsapp]`.
/// Env var fallback: `TWILIO_AUTH_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwilioWhatsAppChannelConfig {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,
    /// Twilio Auth Token
    #[serde(default = "default_twilio_auth_token", skip_serializing)]
    pub auth_token: String,
    /// WhatsApp-enabled Twilio number (E.164; sandbox number if sandbox)
    #[serde(default)]
    pub whatsapp_number: String,
    /// Whether using the Twilio WhatsApp sandbox (default: true)
    #[serde(default = "default_twilio_sandbox")]
    pub sandbox: bool,
    /// Webhook path for inbound messages (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    /// Status callback URL for delivery reports (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_callback_url: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_twilio_sandbox() -> bool {
    true
}

/// Voice (phone calls via Twilio) channel configuration (#316 P3 batch-2b)
///
/// Used under `[channels.voice]`.
/// Env var fallback: `TWILIO_AUTH_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceChannelConfigCore {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,
    /// Twilio Auth Token
    #[serde(default = "default_twilio_auth_token", skip_serializing)]
    pub auth_token: String,
    /// Phone number to call from (E.164 format)
    #[serde(default)]
    pub from_number: String,
    /// Base URL for webhooks (must be publicly accessible, e.g. ngrok URL)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_base_url: Option<String>,
    /// Port for the webhook server (default: adapter default, 8090)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_port: Option<u16>,
    /// TTS voice for calls (Twilio voice name, default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_voice: Option<String>,
    /// Greeting message for incoming calls (default: adapter default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming_greeting: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

/// Hooks configuration
/// LLM Council configuration — multi-model deliberation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilCoreConfig {
    /// Enable council mode
    #[serde(default)]
    pub enabled: bool,
    /// Models to query (e.g. ["anthropic/claude-sonnet-4-6", "openai/gpt-4o"])
    #[serde(default = "default_council_models")]
    pub models: Vec<String>,
    /// Chairman model for final synthesis
    #[serde(default = "default_council_chairman")]
    pub chairman: String,
    /// Per-LLM-call timeout in seconds for council pipeline stages
    #[serde(default = "default_council_timeout_secs")]
    pub timeout_secs: u64,
    /// Skip cross-review stage (faster but less accurate)
    #[serde(default)]
    pub skip_review: bool,
}

fn default_council_models() -> Vec<String> {
    vec![
        "anthropic/claude-sonnet-4-6".to_string(),
        "openai/gpt-4o".to_string(),
        "google/gemini-2.0-flash".to_string(),
    ]
}

fn default_council_chairman() -> String {
    "anthropic/claude-sonnet-4-6".to_string()
}

fn default_council_timeout_secs() -> u64 {
    60
}

impl Default for CouncilCoreConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            models: default_council_models(),
            chairman: default_council_chairman(),
            timeout_secs: default_council_timeout_secs(),
            skip_review: false,
        }
    }
}

/// Hooks configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Enable the logging hook (logs all events via tracing)
    #[serde(default = "default_true")]
    pub logging: bool,
    /// Enable the notification hook (sends notifications on errors)
    #[serde(default)]
    pub notifications: bool,
    /// Enable the memory-save hook (persists key events to workspace)
    #[serde(default)]
    pub memory_save: bool,
    /// Custom shell hooks: event_type -> shell command
    #[serde(default)]
    pub shell_hooks: std::collections::HashMap<String, String>,
    /// Before-tool hooks: tool_name_pattern -> shell command
    /// Command runs before the tool executes. Non-zero exit blocks the tool.
    #[serde(default)]
    pub before_tool: std::collections::HashMap<String, String>,
    /// After-tool hooks: tool_name_pattern -> shell command
    /// Command runs after the tool executes with result in env vars.
    #[serde(default)]
    pub after_tool: std::collections::HashMap<String, String>,
    /// Webhook hooks: event_type -> URL to POST
    /// Sends JSON payload with event context to the URL.
    #[serde(default)]
    pub webhooks: std::collections::HashMap<String, String>,
}

/// Channels (messaging) configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsConfig {
    /// Telegram configuration
    pub telegram: Option<TelegramChannelConfig>,
    /// Discord configuration
    pub discord: Option<DiscordChannelConfig>,
    /// Slack configuration
    pub slack: Option<SlackChannelConfig>,
    /// Email configuration
    pub email: Option<EmailChannelConfig>,
    /// WhatsApp configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whatsapp: Option<WhatsAppChannelConfig>,
    /// Signal configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalChannelConfig>,
    /// Matrix configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixChannelConfig>,
    /// MQTT configuration (IoT/home automation)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt: Option<MqttChannelConfig>,
    /// Mattermost configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mattermost: Option<MattermostChannelConfig>,
    /// IRC configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irc: Option<IrcChannelConfig>,
    /// Twitch configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitch: Option<TwitchChannelConfig>,
    /// X (Twitter) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_twitter: Option<XTwitterChannelConfig>,
    /// Instagram configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instagram: Option<InstagramConfig>,
    /// TikTok configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiktok: Option<TikTokConfig>,
    /// Microsoft Teams configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams: Option<TeamsChannelConfig>,
    /// WebChat (embedded WebSocket chat) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webchat: Option<WebChatChannelConfig>,
    /// Google Chat configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub googlechat: Option<GoogleChatChannelConfig>,
    /// Nextcloud Talk configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextcloud: Option<NextcloudChannelConfig>,
    /// Nostr configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nostr: Option<NostrChannelConfig>,
    /// LINE configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<LineChannelConfig>,
    /// Feishu (Lark) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feishu: Option<FeishuChannelConfig>,
    /// Zalo configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zalo: Option<ZaloChannelConfig>,
    /// BlueBubbles (iMessage server) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluebubbles: Option<BlueBubblesChannelConfig>,
    /// SMS (Twilio) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sms: Option<SmsChannelConfig>,
    /// WhatsApp via Twilio configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twilio_whatsapp: Option<TwilioWhatsAppChannelConfig>,
    /// Voice (phone calls via Twilio) configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceChannelConfigCore>,
}

impl ChannelsConfig {
    /// Auto-detect channel configurations from environment variables.
    /// Returns `Some(config)` if at least one channel has valid credentials,
    /// `None` if no channel env vars are set.
    ///
    /// This allows zero-config channel setup: just set `DISCORD_BOT_TOKEN` in
    /// `~/.zeus/.env` and Discord auto-enables.
    pub fn from_env() -> Option<Self> {
        let mut cfg = Self::default();
        let mut any = false;

        // Discord: DISCORD_BOT_TOKEN (primary bot)
        // Named accounts are TOML-only; per-account token env overrides
        // (DISCORD_ACCOUNT_<ID>_TOKEN) are resolved at connection time by Track B.
        let discord_token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
        if !discord_token.is_empty() {
            cfg.discord = Some(DiscordChannelConfig {
                token: discord_token,
                application_id: None,
                policy: None,
                voice: None,
                accounts: HashMap::new(),
                allow_bots: default_allow_bots_policy(),
            });
            any = true;
        }

        // Slack: SLACK_BOT_TOKEN (app_token optional for some modes)
        let slack_bot = std::env::var("SLACK_BOT_TOKEN").unwrap_or_default();
        if !slack_bot.is_empty() {
            cfg.slack = Some(SlackChannelConfig {
                bot_token: slack_bot,
                app_token: std::env::var("SLACK_APP_TOKEN").unwrap_or_default(),
                policy: None,
                accounts: HashMap::new(),
                allow_bots: default_allow_bots_policy(),
            });
            any = true;
        }

        // Matrix: MATRIX_HOMESERVER + (MATRIX_ACCESS_TOKEN or MATRIX_USER + MATRIX_PASSWORD)
        let matrix_hs = std::env::var("MATRIX_HOMESERVER").unwrap_or_default();
        let matrix_token = std::env::var("MATRIX_ACCESS_TOKEN").unwrap_or_default();
        let matrix_user = std::env::var("MATRIX_USER").ok().filter(|s| !s.is_empty());
        let matrix_pass = std::env::var("MATRIX_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());
        let matrix_rooms: Vec<String> = std::env::var("MATRIX_ROOMS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let has_token = !matrix_token.is_empty();
        let has_password = matrix_user.is_some() && matrix_pass.is_some();
        if !matrix_hs.is_empty() && (has_token || has_password) {
            cfg.matrix = Some(MatrixChannelConfig {
                homeserver: matrix_hs,
                access_token: matrix_token,
                username: matrix_user,
                password: matrix_pass,
                user_id: None,
                rooms: matrix_rooms,
                display_name: None,
                policy: None,
                accounts: HashMap::new(),
                allow_bots: default_allow_bots_policy(),
            });
            any = true;
        }

        // Signal: SIGNAL_CLI_PATH + SIGNAL_ACCOUNT
        let signal_cli = std::env::var("SIGNAL_CLI_PATH").unwrap_or_default();
        let signal_phone = std::env::var("SIGNAL_ACCOUNT").unwrap_or_default();
        if !signal_cli.is_empty() && !signal_phone.is_empty() {
            cfg.signal = Some(SignalChannelConfig {
                signal_cli_path: signal_cli,
                phone: signal_phone,
                policy: None,
                accounts: HashMap::new(),
                allow_bots: default_allow_bots_policy(),
            });
            any = true;
        }

        // Teams: TEAMS_TENANT_ID + TEAMS_CLIENT_ID + TEAMS_CLIENT_SECRET
        let teams_tenant = std::env::var("TEAMS_TENANT_ID").unwrap_or_default();
        let teams_client = std::env::var("TEAMS_CLIENT_ID").unwrap_or_default();
        let teams_secret = std::env::var("TEAMS_CLIENT_SECRET").unwrap_or_default();
        if !teams_tenant.is_empty() && !teams_client.is_empty() && !teams_secret.is_empty() {
            cfg.teams = Some(TeamsChannelConfig {
                tenant_id: teams_tenant,
                client_id: teams_client,
                client_secret: teams_secret,
                team_id: std::env::var("TEAMS_TEAM_ID").unwrap_or_default(),
                channel_id: std::env::var("TEAMS_CHANNEL_ID").unwrap_or_default(),
                policy: None,
            });
            any = true;
        }

        // WebChat: WEBCHAT_ENABLED=1 (serves an embedded WS endpoint; no creds required)
        if std::env::var("WEBCHAT_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            cfg.webchat = Some(WebChatChannelConfig {
                auth_token: std::env::var("WEBCHAT_AUTH_TOKEN").ok().filter(|s| !s.is_empty()),
                ..Default::default()
            });
            any = true;
        }

        // Google Chat: GOOGLE_CHAT_CREDENTIALS_PATH (service-account file)
        let gchat_creds = std::env::var("GOOGLE_CHAT_CREDENTIALS_PATH").unwrap_or_default();
        if !gchat_creds.is_empty() {
            cfg.googlechat = Some(GoogleChatChannelConfig {
                credentials_path: Some(gchat_creds),
                ..Default::default()
            });
            any = true;
        }

        // Nextcloud Talk: NEXTCLOUD_SERVER_URL + NEXTCLOUD_USERNAME + NEXTCLOUD_PASSWORD
        let nc_url = std::env::var("NEXTCLOUD_SERVER_URL").unwrap_or_default();
        let nc_user = std::env::var("NEXTCLOUD_USERNAME").unwrap_or_default();
        let nc_pass = std::env::var("NEXTCLOUD_PASSWORD").unwrap_or_default();
        if !nc_url.is_empty() && !nc_user.is_empty() && !nc_pass.is_empty() {
            cfg.nextcloud = Some(NextcloudChannelConfig {
                server_url: nc_url,
                username: nc_user,
                password: nc_pass,
                poll_interval_secs: None,
                policy: None,
            });
            any = true;
        }

        // Nostr: NOSTR_PRIVATE_KEY or NOSTR_NSEC (+ optional NOSTR_RELAYS, comma-separated)
        let nostr_key = std::env::var("NOSTR_PRIVATE_KEY").unwrap_or_default();
        let nostr_nsec = std::env::var("NOSTR_NSEC").unwrap_or_default();
        if !nostr_key.is_empty() || !nostr_nsec.is_empty() {
            let relays: Vec<String> = std::env::var("NOSTR_RELAYS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            cfg.nostr = Some(NostrChannelConfig {
                private_key: nostr_key,
                nsec: if nostr_nsec.is_empty() { None } else { Some(nostr_nsec) },
                public_key: None,
                relay_urls: relays,
                policy: None,
            });
            any = true;
        }

        // LINE: LINE_CHANNEL_ACCESS_TOKEN (+ optional LINE_CHANNEL_SECRET)
        let line_token = std::env::var("LINE_CHANNEL_ACCESS_TOKEN").unwrap_or_default();
        if !line_token.is_empty() {
            cfg.line = Some(LineChannelConfig {
                channel_access_token: line_token,
                channel_secret: std::env::var("LINE_CHANNEL_SECRET").ok().filter(|s| !s.is_empty()),
                webhook_path: None,
                policy: None,
            });
            any = true;
        }

        // Feishu: FEISHU_APP_ID + FEISHU_APP_SECRET
        let feishu_id = std::env::var("FEISHU_APP_ID").unwrap_or_default();
        let feishu_secret = std::env::var("FEISHU_APP_SECRET").unwrap_or_default();
        if !feishu_id.is_empty() && !feishu_secret.is_empty() {
            cfg.feishu = Some(FeishuChannelConfig {
                app_id: feishu_id,
                app_secret: feishu_secret,
                encrypt_key: std::env::var("FEISHU_ENCRYPT_KEY").ok().filter(|s| !s.is_empty()),
                verification_token: std::env::var("FEISHU_VERIFICATION_TOKEN")
                    .ok()
                    .filter(|s| !s.is_empty()),
                webhook_path: None,
                use_lark: std::env::var("FEISHU_USE_LARK")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false),
                policy: None,
            });
            any = true;
        }

        // Zalo: ZALO_APP_ID + ZALO_SECRET_KEY
        let zalo_id = std::env::var("ZALO_APP_ID").unwrap_or_default();
        let zalo_secret = std::env::var("ZALO_SECRET_KEY").unwrap_or_default();
        if !zalo_id.is_empty() && !zalo_secret.is_empty() {
            cfg.zalo = Some(ZaloChannelConfig {
                app_id: zalo_id,
                secret_key: zalo_secret,
                access_token: std::env::var("ZALO_ACCESS_TOKEN").ok().filter(|s| !s.is_empty()),
                refresh_token: std::env::var("ZALO_REFRESH_TOKEN").ok().filter(|s| !s.is_empty()),
                webhook_path: None,
                policy: None,
            });
            any = true;
        }

        // BlueBubbles: BLUEBUBBLES_SERVER_URL + BLUEBUBBLES_PASSWORD
        let bb_url = std::env::var("BLUEBUBBLES_SERVER_URL").unwrap_or_default();
        let bb_pass = std::env::var("BLUEBUBBLES_PASSWORD").unwrap_or_default();
        if !bb_url.is_empty() && !bb_pass.is_empty() {
            cfg.bluebubbles = Some(BlueBubblesChannelConfig {
                server_url: bb_url,
                password: bb_pass,
                policy: None,
            });
            any = true;
        }

        // Twilio-family channels share TWILIO_ACCOUNT_SID + TWILIO_AUTH_TOKEN;
        // the per-channel number env var decides which channel(s) enable.
        let twilio_sid = std::env::var("TWILIO_ACCOUNT_SID").unwrap_or_default();
        let twilio_token = std::env::var("TWILIO_AUTH_TOKEN").unwrap_or_default();
        if !twilio_sid.is_empty() && !twilio_token.is_empty() {
            // SMS: TWILIO_SMS_FROM_NUMBER
            let sms_from = std::env::var("TWILIO_SMS_FROM_NUMBER").unwrap_or_default();
            if !sms_from.is_empty() {
                cfg.sms = Some(SmsChannelConfig {
                    account_sid: twilio_sid.clone(),
                    auth_token: twilio_token.clone(),
                    from_number: sms_from,
                    webhook_path: None,
                    policy: None,
                });
                any = true;
            }

            // Twilio WhatsApp: TWILIO_WHATSAPP_NUMBER
            let wa_number = std::env::var("TWILIO_WHATSAPP_NUMBER").unwrap_or_default();
            if !wa_number.is_empty() {
                cfg.twilio_whatsapp = Some(TwilioWhatsAppChannelConfig {
                    account_sid: twilio_sid.clone(),
                    auth_token: twilio_token.clone(),
                    whatsapp_number: wa_number,
                    sandbox: std::env::var("TWILIO_WHATSAPP_SANDBOX")
                        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                        .unwrap_or(true),
                    webhook_path: None,
                    status_callback_url: None,
                    policy: None,
                });
                any = true;
            }

            // Voice: TWILIO_VOICE_FROM_NUMBER
            let voice_from = std::env::var("TWILIO_VOICE_FROM_NUMBER").unwrap_or_default();
            if !voice_from.is_empty() {
                cfg.voice = Some(VoiceChannelConfigCore {
                    account_sid: twilio_sid,
                    auth_token: twilio_token,
                    from_number: voice_from,
                    webhook_base_url: std::env::var("ZEUS_WEBHOOK_URL")
                        .ok()
                        .filter(|s| !s.is_empty()),
                    webhook_port: None,
                    tts_voice: None,
                    incoming_greeting: None,
                    policy: None,
                });
                any = true;
            }
        }

        if any { Some(cfg) } else { None }
    }

    /// Merge env-detected channels into an existing config.
    /// Config-file values take priority over env vars.
    pub fn merge_env(&mut self) {
        if let Some(env_cfg) = Self::from_env() {
            if self.discord.is_none() {
                self.discord = env_cfg.discord;
            }
            if self.slack.is_none() {
                self.slack = env_cfg.slack;
            }
            if self.matrix.is_none() {
                self.matrix = env_cfg.matrix;
            }
            if self.signal.is_none() {
                self.signal = env_cfg.signal;
            }
        }
    }
}

/// Telegram relay configuration (Bot HTTP API, no MTProto needed)
///
/// All fields are sourced from `config.toml` under `[telegram_relay]`.
/// No environment variable fallbacks — `~/.zeus/config.toml` is the single source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramRelayConfigCore {
    /// Bot token from @BotFather. Must be set in config.toml.
    #[serde(default, skip_serializing)]
    pub bot_token: String,
    /// Default chat ID to send/receive messages. Must be set in config.toml.
    /// Accepts both string ("−100...") and integer (−100...) in TOML.
    #[serde(default, deserialize_with = "string_or_int")]
    pub chat_id: String,
    /// Allowed usernames (comma-separated)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_users: Option<String>,
    /// Require @mention in groups (default: false)
    #[serde(default)]
    pub require_mention_in_groups: bool,
    /// Tmux session to forward messages to (for interactive relay)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session: Option<String>,
    /// Access policy (group mention filtering, DM access)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Per-node toggle to disable the Telegram relay without removing config.
    /// Set to `false` to prevent this node from polling Telegram.
    /// Defaults to `true` (enabled) when the `[telegram_relay]` section exists.
    #[serde(default = "default_true")]
    pub enable_telegram_relay: bool,
    /// Bot message filter mode (`off` | `mentions` | `on`).
    /// Default `mentions` — bot-authored messages only pass through if they
    /// @-mention this bot (or reply to one of our messages, or contain a
    /// structured entity mention). Self-echo is always blocked regardless.
    /// Set to `"on"` to allow all bot messages (legacy behavior, infinite-loop risk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
    /// Fleet bot allowlist — Telegram user IDs (as strings) that bypass the
    /// Layer 2 mention filter. Allows fleet titans to coordinate via reply-chains
    /// without triggering the bot-loop prevention. External bots not in this list
    /// are still gated by `allow_bots` mode.
    ///
    /// Example: `fleet_bot_ids = ["123456789", "987654321"]`
    ///
    /// Empty list (default) = standard `allow_bots` behavior, no allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fleet_bot_ids: Vec<String>,
}

/// Matrix relay configuration (matrix-sdk, dedicated relay — not ChannelManager)
///
/// Like `[telegram_relay]`, this provides a standalone Matrix relay that
/// routes inbound room messages to `agent.run()` and sends replies back.
/// Env var fallbacks: `MATRIX_HOMESERVER`, `MATRIX_USER`, `MATRIX_PASSWORD`,
/// `MATRIX_ACCESS_TOKEN`, `MATRIX_ROOMS`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixRelayConfig {
    /// Homeserver URL (e.g., "https://matrix.org")
    #[serde(default = "default_matrix_homeserver")]
    pub homeserver: String,
    /// Username (e.g., "@bot:matrix.org" or just "bot")
    #[serde(default = "default_matrix_user")]
    pub username: Option<String>,
    /// Password for password-based login
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Access token (alternative to password login)
    #[serde(default = "default_matrix_access_token_opt", skip_serializing)]
    pub access_token: Option<String>,
    /// User ID for token restore (e.g., "@bot:matrix.org")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Rooms to join and monitor (room IDs or aliases)
    #[serde(default = "default_matrix_rooms")]
    pub rooms: Vec<String>,
    /// Display name to set after login
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Access policy (DM/group filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_matrix_access_token_opt() -> Option<String> {
    std::env::var("MATRIX_ACCESS_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

fn default_matrix_rooms() -> Vec<String> {
    std::env::var("MATRIX_ROOMS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Signal relay configuration (signal-cli JSON-RPC subprocess)
///
/// Dedicated `[signal_relay]` config for routing inbound Signal messages
/// to `agent.run()`. Env var fallbacks: `SIGNAL_CLI_PATH`, `SIGNAL_ACCOUNT`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalRelayConfig {
    /// Path to signal-cli binary
    #[serde(default = "default_signal_cli_path")]
    pub signal_cli_path: String,
    /// Phone number registered with Signal
    #[serde(default = "default_signal_account")]
    pub phone: String,
    /// Access policy (DM/group filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// HTTP callback host for signal-cli (default: 127.0.0.1)
    #[serde(default = "default_signal_http_host")]
    pub http_host: String,
    /// HTTP callback port for signal-cli (default: 8088)
    #[serde(default = "default_signal_http_port")]
    pub http_port: u16,
    /// Allowed sender phone numbers. Only messages from these numbers are processed.
    /// Empty = allow all senders (default, backward compatible).
    /// Format: ["+1234567890", "+0987654321"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_senders: Vec<String>,
}

/// WhatsApp relay configuration (Bridge or Cloud API)
///
/// Dedicated `[whatsapp_relay]` config for routing inbound WhatsApp messages
/// to `agent.run()`. Bridge mode uses WebSocket; Cloud API uses webhook.
/// Env var fallbacks: `WHATSAPP_BRIDGE_URL`, `WHATSAPP_ACCESS_TOKEN`,
/// `WHATSAPP_PHONE_NUMBER_ID`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppRelayConfig {
    /// Operating mode: "bridge" (Baileys WS) or "cloud_api" (Meta Graph API)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// WebSocket bridge URL (Bridge mode, e.g., "ws://localhost:3001")
    #[serde(default = "default_whatsapp_bridge")]
    pub bridge_url: String,
    /// Meta Graph API access token (Cloud API mode)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Phone number ID from WhatsApp Business dashboard (Cloud API mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number_id: Option<String>,
    /// Webhook verification token (Cloud API mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_token: Option<String>,
    /// Graph API version (Cloud API mode, default "v21.0")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    /// Phone number associated with this WhatsApp account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// Access policy (DM/group filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Allowed sender phone numbers. Only messages from these numbers are processed.
    /// Empty = allow all senders (default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_senders: Vec<String>,
}

/// Mattermost relay configuration
///
/// Dedicated `[mattermost_relay]` config for routing inbound Mattermost messages
/// to `agent.run()`. Uses REST API + WebSocket for real-time events.
/// Env var fallbacks: `MATTERMOST_URL`, `MATTERMOST_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MattermostRelayConfig {
    /// Mattermost server URL (e.g., "https://mattermost.example.com")
    #[serde(default = "default_mattermost_url")]
    pub server_url: String,
    /// Personal access token or bot token
    #[serde(default = "default_mattermost_token", skip_serializing)]
    pub token: String,
    /// Team ID (optional, for team-specific operations)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// Access policy (DM/channel filtering)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_mattermost_url() -> String {
    std::env::var("MATTERMOST_URL").unwrap_or_default()
}

fn default_mattermost_token() -> String {
    std::env::var("MATTERMOST_TOKEN").unwrap_or_default()
}

/// Telegram channel configuration (MTProto via grammers)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChannelConfig {
    /// MTProto API ID (from my.telegram.org). Not needed for Bot API mode.
    #[serde(default)]
    pub api_id: i32,
    /// MTProto API hash. Not needed for Bot API mode.
    #[serde(default)]
    pub api_hash: String,
    #[serde(default)]
    pub phone: String,
    /// Bot token for bot mode (alternative to phone auth)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    pub session_file: Option<String>,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account Telegram configurations.
    /// Each entry spawns a separate adapter instance.
    #[serde(default)]
    pub accounts: HashMap<String, TelegramAccountConfig>,
    /// Global bot message policy (overridden per-account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Per-account Telegram configuration for multi-account support.
///
/// Each entry in `TelegramChannelConfig.accounts` spawns a dedicated adapter
/// instance with its own bot token / phone identity.
///
/// TOML example:
/// ```toml
/// [channels.telegram.accounts.support]
/// bot_token = "123456:ABC..."
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramAccountConfig {
    /// Bot token for this account
    #[serde(default, skip_serializing)]
    pub bot_token: Option<String>,
    /// API ID override (falls back to top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_id: Option<i32>,
    /// API hash override (falls back to top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_hash: Option<String>,
    /// Session file path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Per-account Discord configuration for multi-bot support.
///
/// Each entry in `DiscordChannelConfig.accounts` spawns a dedicated WebSocket
/// connection with its own bot identity.
///
/// TOML example:
/// ```toml
/// [channels.discord.accounts.z]
/// token = "Bot-token-for-zeusmolty"
/// agent_id = "zeus-native"
///
/// [channels.discord.accounts.relay]
/// token = "Bot-token-for-fleet-relay"
/// ```
///
/// Env-var override: `DISCORD_ACCOUNT_<ID>_TOKEN` (uppercase ID, e.g.
/// `DISCORD_ACCOUNT_Z_TOKEN`) takes precedence over the TOML value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscordAccountConfig {
    /// Bot token for this account.
    /// Override via `DISCORD_ACCOUNT_<ID>_TOKEN` env var.
    #[serde(default)]
    pub token: String,
    /// Application ID (optional, for slash commands)
    pub application_id: Option<u64>,
    /// Webhook URL for per-message identity (username + avatar per send)
    #[serde(default, skip_serializing)]
    pub webhook_url: Option<String>,
    /// Access policy for this account's bot
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages from this account to.
    /// If `None`, falls back to the default agent session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off" (default, skip all bot messages),
    /// "mentions" (allow bot messages that @mention us), "on" (allow all).
    /// OpenClaw parity: `allowBots` config field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Discord channel configuration
///
/// Primary bot token falls back to `DISCORD_BOT_TOKEN` env var if not set in config.
/// Named accounts in `accounts` each spawn a separate WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordChannelConfig {
    #[serde(default = "default_discord_token")]
    pub token: String,
    pub application_id: Option<u64>,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Voice channel configuration (enables STT/TTS in voice channels)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<DiscordVoiceChannelConfig>,
    /// Named per-account Discord configurations.
    /// Each entry spawns a separate bot connection (N WebSocket connections).
    /// Key = account label (e.g. `"z"`, `"relay"`).
    #[serde(default)]
    pub accounts: HashMap<String, DiscordAccountConfig>,
    /// Global bot message policy (overridden per-account via `accounts.*.allow_bots`).
    /// "mentions" (default): allow bot msgs that @mention you. "off": skip all bot messages.
    /// "on": allow all bot messages (for agent-to-agent comms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Discord voice channel configuration
///
/// When present under `[channels.discord.voice]`, enables Zeus to join
/// Discord voice channels for speech-to-text / text-to-speech interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordVoiceChannelConfig {
    /// Enable voice channel support
    #[serde(default)]
    pub enabled: bool,
    /// Auto-join these voice channels on startup (format: "guild_id:channel_id")
    #[serde(default)]
    pub auto_join_channels: Vec<String>,
    /// Minimum speech duration in ms before transcribing
    #[serde(default = "default_voice_min_speech_ms")]
    pub min_speech_ms: u64,
    /// Silence duration in ms to detect end of speech
    #[serde(default = "default_voice_silence_timeout_ms")]
    pub silence_timeout_ms: u64,
    /// Energy threshold for VAD (RMS amplitude, 0.0–1.0)
    #[serde(default = "default_voice_energy_threshold")]
    pub energy_threshold: f64,
    /// TTS voice identifier (e.g. "en_US-amy-medium")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_voice: Option<String>,
    /// TTS provider: "piper", "openai", "elevenlabs"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_provider: Option<String>,
    /// Piper TTS server URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub piper_url: Option<String>,
    /// STT provider: "groq" or "openai"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stt_provider: Option<String>,
    /// Only respond when a wake word is detected
    #[serde(default)]
    pub require_wake_word: bool,
    /// Wake words that trigger processing
    #[serde(default)]
    pub wake_words: Vec<String>,
}

fn default_voice_min_speech_ms() -> u64 {
    500
}
fn default_voice_silence_timeout_ms() -> u64 {
    1500
}
fn default_voice_energy_threshold() -> f64 {
    0.02
}

fn default_discord_token() -> String {
    std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default()
}

/// Resolve Discord bot token from config.toml (SSoT) with env var fallback.
/// Returns the first non-empty token found: per-account → top-level → env var.
pub fn resolve_discord_token() -> Option<String> {
    if let Ok(config) = Config::load() {
        if let Some(ref channels) = config.channels {
            if let Some(ref dc) = channels.discord {
                for acct in dc.accounts.values() {
                    if !acct.token.is_empty() {
                        return Some(acct.token.clone());
                    }
                }
                if !dc.token.is_empty() {
                    return Some(dc.token.clone());
                }
            }
        }
    }
    std::env::var("DISCORD_BOT_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Resolve the canonical on-disk path for an inbound channel attachment.
///
/// Returns `<workspace>/channels/<platform>/received/<YYYY-MM-DD>/<safe_name>`.
/// This is the single source of truth for where titan-inbound files land —
/// every download/persist site (gateway inbound + Telegram relay + Talos
/// relays) calls this so the directory logic can never drift across them.
///
/// Hardening:
/// - `filename` is treated as untrusted input and run through
///   [`sanitize::sanitize_filename`], so a hostile name (`../../etc/passwd`,
///   path separators, control chars) can never escape `received/<date>/`.
/// - An empty/whitespace `platform` falls back to the `unknown` bucket
///   (`channels/unknown/received/...`), never a bare or empty path segment.
///
/// The parent directory is **not** created here — callers create it (they're
/// already doing `create_dir_all` and own the async/sync choice).
pub fn received_file_path(platform: &str, filename: &str) -> Result<PathBuf> {
    let workspace = Config::zeus_home()?.join("workspace");

    let platform_seg = {
        let p = platform.trim();
        let cleaned: String = p
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if cleaned.is_empty() {
            "unknown".to_string()
        } else {
            cleaned
        }
    };

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let safe_name = sanitize::sanitize_filename(filename);

    Ok(workspace
        .join("channels")
        .join(platform_seg)
        .join("received")
        .join(date)
        .join(safe_name))
}

/// Slack channel configuration
///
/// Tokens fall back to `SLACK_BOT_TOKEN` / `SLACK_APP_TOKEN` env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackChannelConfig {
    #[serde(default = "default_slack_bot_token")]
    pub bot_token: String,
    #[serde(default = "default_slack_app_token")]
    pub app_token: String,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account Slack configurations.
    /// Each entry spawns a separate adapter instance.
    #[serde(default)]
    pub accounts: HashMap<String, SlackAccountConfig>,
    /// Global bot message policy (overridden per-account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

/// Per-account Slack configuration for multi-workspace support.
///
/// Each entry in `SlackChannelConfig.accounts` spawns a dedicated adapter
/// with its own bot token and workspace connection.
///
/// TOML example:
/// ```toml
/// [channels.slack.accounts.support]
/// bot_token = "xoxb-..."
/// app_token = "xapp-..."
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlackAccountConfig {
    /// Bot token (xoxb-...)
    #[serde(default, skip_serializing)]
    pub bot_token: String,
    /// App token for socket mode (xapp-...)
    #[serde(default, skip_serializing)]
    pub app_token: Option<String>,
    /// Allowed channel IDs (empty = all)
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<String>,
}

fn default_slack_bot_token() -> String {
    std::env::var("SLACK_BOT_TOKEN").unwrap_or_default()
}

fn default_slack_app_token() -> String {
    std::env::var("SLACK_APP_TOKEN").unwrap_or_default()
}

/// Slack relay configuration (Socket Mode, forwards to tmux like Telegram relay)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackRelayConfigCore {
    /// Bot token (xoxb-...). Falls back to SLACK_BOT_TOKEN env var.
    #[serde(default = "default_slack_bot_token", skip_serializing)]
    pub bot_token: String,
    /// App-level token for Socket Mode (xapp-...). Falls back to SLACK_APP_TOKEN env var.
    #[serde(default = "default_slack_app_token", skip_serializing)]
    pub app_token: String,
    /// Channel IDs to listen on (empty = all channels the bot is in)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_ids: Option<String>,
    /// Allowed user IDs (comma-separated, empty = all users)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_users: Option<String>,
    /// Require @mention in channels (default: true)
    #[serde(default = "default_true")]
    pub require_mention_in_channels: bool,
    /// Tmux session to forward messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session: Option<String>,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

/// Email relay configuration (IMAP polling → agent.run(), dedicated relay)
///
/// Like `[matrix_relay]`, this provides a standalone Email relay that
/// routes inbound emails to `agent.run()` and sends replies back via SMTP.
/// Env var fallbacks: `ZEUS_EMAIL_ADDRESS`, `ZEUS_EMAIL_PASSWORD`,
/// `ZEUS_SMTP_SERVER`, `ZEUS_IMAP_SERVER`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailRelayConfig {
    /// SMTP server for sending replies
    #[serde(default = "default_email_relay_smtp")]
    pub smtp_server: String,
    /// SMTP port (default: 587)
    #[serde(default = "default_email_smtp_port")]
    pub smtp_port: u16,
    /// IMAP server for receiving (required for relay to work)
    #[serde(default = "default_email_relay_imap")]
    pub imap_server: String,
    /// IMAP port (default: 993)
    #[serde(default = "default_email_imap_port")]
    pub imap_port: u16,
    /// Email address
    #[serde(default = "default_email_relay_address")]
    pub email: String,
    /// Password (use app password for Gmail)
    #[serde(default, skip_serializing)]
    pub password: String,
    /// IMAP inbox folder (default: "INBOX")
    #[serde(default = "default_email_inbox")]
    pub inbox_folder: String,
    /// Use TLS (default: true)
    #[serde(default = "default_true")]
    pub use_tls: bool,
    /// Poll interval in seconds (default: 60)
    #[serde(default = "default_email_poll_interval")]
    pub poll_interval_secs: u64,
    /// Allowed sender email addresses (empty = accept all)
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    /// Access policy (DM access control)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_email_relay_smtp() -> String {
    std::env::var("ZEUS_SMTP_SERVER").unwrap_or_default()
}

fn default_email_smtp_port() -> u16 {
    587
}

fn default_email_relay_imap() -> String {
    std::env::var("ZEUS_IMAP_SERVER").unwrap_or_default()
}

fn default_email_imap_port() -> u16 {
    993
}

fn default_email_relay_address() -> String {
    std::env::var("ZEUS_EMAIL_ADDRESS").unwrap_or_default()
}

fn default_email_inbox() -> String {
    "INBOX".to_string()
}

fn default_email_poll_interval() -> u64 {
    60
}

/// MQTT relay configuration (rumqttc subscribe loop → agent.run(), dedicated relay)
///
/// Like `[matrix_relay]`, this provides a standalone MQTT relay that
/// routes inbound topic messages to `agent.run()` and publishes replies.
/// Env var fallbacks: `MQTT_BROKER_URL`, `MQTT_PORT`, `MQTT_CLIENT_ID`,
/// `MQTT_USERNAME`, `MQTT_PASSWORD`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttRelayConfig {
    /// Broker host (e.g., "localhost" or "mqtt.example.com")
    #[serde(default = "default_mqtt_relay_broker")]
    pub broker: String,
    /// Broker port (default: 1883)
    #[serde(default = "default_mqtt_relay_port")]
    pub port: u16,
    /// MQTT client ID (must be unique per connection)
    #[serde(default = "default_mqtt_relay_client_id")]
    pub client_id: String,
    /// Topics to subscribe to (supports MQTT wildcards: +, #)
    #[serde(default)]
    pub topics: Vec<String>,
    /// Topic prefix for published replies
    #[serde(default)]
    pub reply_topic_prefix: String,
    /// QoS level: 0, 1, or 2 (default: 1)
    #[serde(default = "default_mqtt_relay_qos")]
    pub qos: u8,
    /// Optional username for broker auth
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Optional password for broker auth
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Keep-alive interval in seconds (default: 30)
    #[serde(default = "default_mqtt_relay_keepalive")]
    pub keep_alive_secs: u64,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
}

fn default_mqtt_relay_broker() -> String {
    std::env::var("MQTT_BROKER_URL").unwrap_or_else(|_| "localhost".to_string())
}

fn default_mqtt_relay_port() -> u16 {
    std::env::var("MQTT_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1883)
}

fn default_mqtt_relay_client_id() -> String {
    std::env::var("MQTT_CLIENT_ID").unwrap_or_else(|_| "zeus-gateway".to_string())
}

fn default_mqtt_relay_qos() -> u8 {
    1
}

fn default_mqtt_relay_keepalive() -> u64 {
    30
}

/// Email channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailChannelConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub imap_host: String,
    pub imap_port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
    /// Access policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Named per-account Email configurations.
    /// Each entry spawns a separate adapter instance.
    #[serde(default)]
    pub accounts: HashMap<String, EmailAccountConfig>,
}

/// Per-account Email configuration for multi-mailbox support.
///
/// Each entry in `EmailChannelConfig.accounts` spawns a dedicated adapter
/// with its own SMTP/IMAP credentials.
///
/// TOML example:
/// ```toml
/// [channels.email.accounts.support]
/// username = "support@example.com"
/// password = "app-password"
/// agent_id = "support-agent"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmailAccountConfig {
    /// SMTP server (falls back to top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_host: Option<String>,
    /// SMTP port
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_port: Option<u16>,
    /// IMAP server (falls back to top-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_host: Option<String>,
    /// IMAP port
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_port: Option<u16>,
    /// Email address
    #[serde(default)]
    pub username: String,
    /// Password
    #[serde(default, skip_serializing)]
    pub password: String,
    /// Use TLS
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_tls: Option<bool>,
    /// Access policy for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ChannelPolicyConfig>,
    /// Agent ID to route inbound messages to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            fallback_models: None,
            workspace: default_workspace(),
            sessions: default_sessions(),
            tui: TuiConfig::default(),
            auth: AuthConfig::default(),
            oauth: OAuthConfig::default(),
            provider_credentials: CredentialsConfig::default(),
            ollama: OllamaConfig::default(),
            max_iterations: default_max_iterations(),
            max_subagent_iterations: default_max_subagent_iterations(),
            mnemosyne: None,
            athena: None,
            aegis: None,
            hermes: None,
            prometheus: None,
            nous: None,
            talos: None,
            channels: None,
            hooks: None,
            council: None,
            search: Some(SearchConfig::default()),
            gateway: None,
            logging: None,
            pantheon: None,
            star_office: None,
            session_compaction: Some(SessionCompactionConfig::default()),
            pruning: None,
            thinking_level: Some("high".to_string()),
            onboarding_complete: false,
            session_maintenance: None,
            image_gen: None,
            video_gen: None,
            voice: None,
            overflow: None,
            suppress_tool_errors: false,
            ws_auth: None,
            telegram_relay: None,
            slack_relay: None,
            matrix_relay: None,
            signal_relay: None,
            email_relay: None,
            mqtt_relay: None,
            whatsapp_relay: None,
            mattermost_relay: None,
            mcp_server: None,
            wallet: None,
            model_routing: None,
            agent_pool: None,
            network: None,
            deployment: None,
            economy: None,
            verbosity: Verbosity::default(),
            loaded_from_default: false,
            credentials: std::collections::HashMap::new(),
            agents: Vec::new(),
            bindings: Vec::new(),
            features: std::collections::HashMap::new(),
            enabled_skills: Vec::new(),
            persona: None,
            name: None,
            agent: None,
            skill_matcher_threshold: None,
            skill_matcher_top_k: None,
        }
    }
}

/// Outcome of [`Config::persist_gateway_port_guarded`] (#311).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortPersist {
    /// No config.toml on disk — nothing to persist into.
    NoConfig,
    /// On-disk port already matches (or there was nothing to rewrite).
    NoChange,
    /// Config had no explicit gateway port; the runtime port was recorded.
    Persisted,
    /// Config HAS an explicit port that differs from the runtime port.
    /// The config was NOT touched — callers must log this loudly.
    OverrideDetected {
        /// The explicit port recorded in config.toml.
        config_port: u16,
    },
}

impl Config {
    /// Resolve the Zeus config directory.
    /// Checks `ZEUS_HOME` env var first (used by deploy scripts and tests),
    /// falls back to `~/.zeus`.
    pub fn zeus_home() -> Result<PathBuf> {
        if let Ok(home) = std::env::var("ZEUS_HOME") {
            if !home.is_empty() {
                return Ok(PathBuf::from(home));
            }
        }
        Ok(dirs::home_dir()
            .ok_or_else(|| Error::Config("Could not find home directory".to_string()))?
            .join(".zeus"))
    }

    /// Resolve the path to config.toml.
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::zeus_home()?.join("config.toml"))
    }

    /// #105 fix #1 (root): persist the *real* gateway port back into
    /// `config.toml` so the on-disk `[gateway] port` is the single source of
    /// truth. When the gateway is started with an explicit `--port` (or any
    /// runtime-resolved port) that differs from what config.toml records, the
    /// TUI/client would otherwise probe the stale config port and report a
    /// false "offline". Surgically rewrites only the `port = N` line inside the
    /// `[gateway]` table, preserving all comments and formatting. Idempotent:
    /// returns `Ok(false)` (no write) when the on-disk value already matches.
    ///
    /// ⚠️ #311: prefer [`Config::persist_gateway_port_guarded`] — this
    /// unguarded variant will overwrite an explicit on-disk port with
    /// whatever the CLI/service passed, which is how a stale
    /// `zeus_gateway_port` in rc.conf hijacked a fresh install's config.
    ///
    /// Returns `Ok(true)` if the file was rewritten, `Ok(false)` if no change
    /// was needed.
    pub fn persist_gateway_port(real_port: u16) -> Result<bool> {
        let config_path = Self::config_path()?;
        if !config_path.exists() {
            return Ok(false);
        }
        let content = std::fs::read_to_string(&config_path)?;
        let updated = Self::rewrite_gateway_port(&content, real_port);
        if updated == content {
            return Ok(false);
        }
        Self::atomic_write(&config_path, &updated)?;
        Ok(true)
    }

    /// #311 port-persist guard: persist the runtime port back to config.toml
    /// **only when the config has no explicit `[gateway] port` yet**, and
    /// report an override instead of silently rewriting when it does.
    ///
    /// Rationale: the unguarded persist treats whatever the CLI/service
    /// passed as the source of truth. On the .224 install a stale
    /// `zeus_gateway_port=3001` left in rc.conf by an old uninstall got fed
    /// to `zeus gateway --port 3001`, which then *overwrote* the fresh
    /// config.toml's `port = 8080` — the stale service var hijacked the new
    /// install. With the guard, an existing explicit port is never clobbered;
    /// the caller gets [`PortPersist::OverrideDetected`] and must log loudly.
    pub fn persist_gateway_port_guarded(real_port: u16) -> Result<PortPersist> {
        Self::persist_gateway_port_guarded_at(&Self::config_path()?, real_port)
    }

    /// Path-parameterized core of [`Config::persist_gateway_port_guarded`]
    /// (separated for testability — no env mutation needed in tests).
    pub fn persist_gateway_port_guarded_at(
        config_path: &std::path::Path,
        real_port: u16,
    ) -> Result<PortPersist> {
        if !config_path.exists() {
            return Ok(PortPersist::NoConfig);
        }
        let content = std::fs::read_to_string(config_path)?;
        match Self::gateway_port_in(&content) {
            // Explicit port already recorded and matches — nothing to do.
            Some(existing) if existing == real_port => Ok(PortPersist::NoChange),
            // Explicit port differs — the runtime value is overriding the
            // operator's config. Do NOT rewrite; surface it.
            Some(existing) => Ok(PortPersist::OverrideDetected { config_port: existing }),
            // No explicit port in [gateway] — first write wins; record it so
            // the TUI/client probes the right port (#105 intent, kept).
            None => {
                let mut updated = Self::rewrite_gateway_port(&content, real_port);
                if updated == content {
                    // No active `port` key to rewrite — insert one right
                    // after the [gateway] header if the table exists.
                    updated = Self::insert_gateway_port(&content, real_port);
                }
                if updated == content {
                    // No [gateway] table at all — nothing to record into.
                    return Ok(PortPersist::NoChange);
                }
                Self::atomic_write(config_path, &updated)?;
                Ok(PortPersist::Persisted)
            }
        }
    }

    /// Pure helper: insert `port = N` immediately after the `[gateway]`
    /// header. Returns the input unchanged when no `[gateway]` table exists.
    fn insert_gateway_port(content: &str, real_port: u16) -> String {
        let mut out = String::with_capacity(content.len() + 16);
        let mut inserted = false;
        for line in content.lines() {
            out.push_str(line);
            out.push('\n');
            if !inserted && line.trim_start().starts_with("[gateway]") {
                out.push_str(&format!("port = {real_port}\n"));
                inserted = true;
            }
        }
        if !content.ends_with('\n') {
            out.pop();
        }
        if inserted { out } else { content.to_string() }
    }

    /// Pure helper: read the explicit `port = N` value inside the `[gateway]`
    /// table, ignoring comments and other tables. `None` when the config has
    /// no explicit gateway port.
    pub fn gateway_port_in(content: &str) -> Option<u16> {
        let mut in_gateway = false;
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('[') {
                in_gateway = trimmed.starts_with("[gateway]");
                continue;
            }
            if in_gateway
                && !trimmed.starts_with('#')
                && let Some(eq) = trimmed.find('=')
                && trimmed[..eq].trim() == "port"
            {
                let val = trimmed[eq + 1..].trim();
                let val = val.split(&['#', ' ', '\t'][..]).next().unwrap_or(val);
                if let Ok(p) = val.parse::<u16>() {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Pure helper: rewrite the `port = N` line within the `[gateway]` table.
    /// Comment/format-preserving. Returns the input unchanged if no `[gateway]`
    /// table or `port` key is found (callers should not have to special-case).
    pub fn rewrite_gateway_port(content: &str, real_port: u16) -> String {
        let mut out = String::with_capacity(content.len() + 8);
        let mut in_gateway = false;
        let mut rewrote = false;
        for line in content.lines() {
            let trimmed = line.trim_start();
            // Section headers toggle table context.
            if trimmed.starts_with('[') {
                in_gateway = trimmed.starts_with("[gateway]");
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_gateway && !rewrote {
                // Match a bare `port = N` key (ignore commented lines).
                let key = trimmed.trim_start_matches('#').trim_start();
                if !trimmed.starts_with('#') && key.starts_with("port") {
                    if let Some(eq) = key.find('=') {
                        if key[..eq].trim() == "port" {
                            // Preserve leading indentation.
                            let indent_len = line.len() - trimmed.len();
                            out.push_str(&line[..indent_len]);
                            out.push_str(&format!("port = {}", real_port));
                            out.push('\n');
                            rewrote = true;
                            continue;
                        }
                    }
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        // Preserve absence of trailing newline if the original lacked one.
        if !content.ends_with('\n') {
            out.pop();
        }
        out
    }

    /// Decide whether the onboarding wizard should run for this config.
    ///
    /// Maps the 3-condition spec:
    /// - **fresh install** (no `config.toml` on disk → `loaded_from_default`) → onboard
    /// - **nuked config** (`model` empty) → onboard, **even if the `onboarding_complete`
    ///   marker is still set** — self-heal a box whose model got wiped (#267(b)).
    ///   A legitimately completed config always persists a non-empty
    ///   `model` (`provider_id/model_id`), so an empty `model` flags **only** a
    ///   nuked config — never a healthy completed one.
    /// - **completed** (`onboarding_complete` marker persisted, model present) → skip
    /// - **legacy migration** (a `model` is configured but the marker predates this
    ///   field, so `onboarding_complete` is false) → treat as done, skip
    ///
    /// Returns `true` when the wizard must run: the model is empty (nuked-box
    /// self-heal) OR the marker is unset on a fresh default load.
    pub fn needs_onboarding(&self) -> bool {
        self.model.is_empty() || (!self.onboarding_complete && self.loaded_from_default)
    }

    /// Load config from ~/.zeus/config.toml or return defaults
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(Config { loaded_from_default: true, ..Config::default() });
        }

        let content = std::fs::read_to_string(&config_path)?;
        match toml::from_str::<Config>(&content) {
            Ok(mut config) => {
                // Parse SUCCEEDED — only now back up the known-good config.
                // (Backing up *before* the parse would clobber a good .bak with a
                //  corrupt/wiped config on the load that follows a bad write.)
                let backup_path = config_path.with_extension("toml.bak");
                if let Err(e) = std::fs::copy(&config_path, &backup_path) {
                    tracing::debug!("Config backup failed (non-fatal): {}", e);
                }
                config.expand_tildes();
                config.ensure_required_sections();
                Ok(config)
            }
            Err(e) => {
                // Parse FAILED (e.g. a newer binary with an incompatible schema).
                // CRITICAL: do NOT overwrite the user's real config with defaults.
                // Return a *guarded* default (loaded_from_default = true) so that any
                // subsequent Config::save() is refused by the save-guard — the real
                // config.toml on disk is preserved untouched. Also stash the
                // unparseable config to `.parse-error-backup` for recovery, and log
                // loudly so the operator knows the gateway is running on defaults.
                let recovery = config_path.with_extension("toml.parse-error-backup");
                let _ = std::fs::copy(&config_path, &recovery);
                tracing::error!(
                    "config.toml at {} FAILED to parse ({e}). Running on DEFAULTS this \
                     session WITHOUT saving (your config is preserved on disk + copied to \
                     {}). Fix the config and restart — saves are blocked until it parses.",
                    config_path.display(),
                    recovery.display(),
                );
                Ok(Config { loaded_from_default: true, ..Config::default() })
            }
        }
    }

    /// Load config from a specific path
    pub fn load_from(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let mut config: Config = toml::from_str(&content)?;
        config.expand_tildes();
        Ok(config)
    }

    /// Expand `~` to actual home directory in all path fields
    fn expand_tildes(&mut self) {
        fn expand(path: &mut PathBuf) {
            let s = path.to_string_lossy();
            if let Some(rest) = s.strip_prefix("~/")
                && let Some(home) = dirs::home_dir()
            {
                *path = home.join(rest);
            }
        }
        expand(&mut self.workspace);
        expand(&mut self.sessions);
        if let Some(ref mut m) = self.mnemosyne {
            expand(&mut m.db_path);
        }
        if let Some(ref mut a) = self.athena {
            expand(&mut a.vault_path);
        }
        for agent in &mut self.agents {
            if let Some(ref mut w) = agent.workspace {
                expand(w);
            }
            if let Some(ref mut s) = agent.sessions {
                expand(s);
            }
        }
    }

    /// Collapse an absolute home-rooted path back to `~/…` form — the inverse of
    /// [`Self::expand_tildes`]. Applied to a CLONE right before serialization in
    /// both `save()` and `save_unchecked()` so `config.toml` always stores the
    /// portable `~/` form. This is the real fix for the fleet config-nuke (#291):
    /// `expand_tildes()` rewrites `~/.zeus/*` → `dirs::home_dir().join(…)` at LOAD,
    /// and under a temp HOME (launchd restart / spawned subproc / sandboxed env)
    /// that yields `/var/folders/…`. Collapsing on save guarantees a temp-HOME
    /// expansion can never round-trip to disk as an absolute temp path.
    fn collapse_tildes(&mut self) {
        fn collapse(path: &mut PathBuf) {
            let Some(home) = dirs::home_dir() else {
                return;
            };
            if let Ok(rest) = path.strip_prefix(&home) {
                // `~` + `/` + the remainder relative to home.
                *path = PathBuf::from("~").join(rest);
            }
        }
        collapse(&mut self.workspace);
        collapse(&mut self.sessions);
        if let Some(ref mut m) = self.mnemosyne {
            collapse(&mut m.db_path);
        }
        if let Some(ref mut a) = self.athena {
            collapse(&mut a.vault_path);
        }
    }

    /// Refuse to persist a config whose load-bearing path fields point into a
    /// temp directory (`/var/folders/`, `/tmp/`, `tmp.`). Shared by `save()` and
    /// `save_unchecked()` so the two writers' guards can never drift — that drift
    /// is exactly what caused the #291 fleet config-nuke (`save()` guarded,
    /// `save_unchecked()` did not). Mirrors the `reject_sentinel_credentials`
    /// shared-helper pattern. Belt-and-suspenders behind `collapse_tildes`.
    fn reject_temp_paths(&self) -> Result<()> {
        fn is_temp(path: &std::path::Path) -> bool {
            let s = path.to_string_lossy();
            s.contains("/var/folders/") || s.starts_with("/tmp/") || s.starts_with("/tmp.")
        }
        let fields: [(&str, &std::path::Path); 4] = [
            ("workspace", self.workspace.as_path()),
            ("sessions", self.sessions.as_path()),
            (
                "mnemosyne.db_path",
                self.mnemosyne
                    .as_ref()
                    .map(|m| m.db_path.as_path())
                    .unwrap_or_else(|| std::path::Path::new("")),
            ),
            (
                "athena.vault_path",
                self.athena
                    .as_ref()
                    .map(|a| a.vault_path.as_path())
                    .unwrap_or_else(|| std::path::Path::new("")),
            ),
        ];
        for (name, path) in fields {
            if is_temp(path) {
                return Err(Error::Config(format!(
                    "Refusing to save config: {} path '{}' is a temp directory",
                    name,
                    path.to_string_lossy()
                )));
            }
        }
        Ok(())
    }

    /// Ensure required config sections exist with sensible defaults.
    /// Called on load to fill in missing `[mnemosyne]`, `[nous]`, `[hermes]` sections
    /// so the gateway always has these subsystems configured.
    fn ensure_required_sections(&mut self) {
        if self.mnemosyne.is_none() {
            self.mnemosyne = Some(MnemosyneConfig::default());
        }
        if self.nous.is_none() {
            self.nous = Some(NousConfig {
                enable_intent: true,
                enable_learning: true,
            });
        }
        if self.hermes.is_none() {
            self.hermes = Some(HermesConfig::default());
        }
        // Auto-enable session pruning if not explicitly configured.
        // Prevents session bloat (100s of JSONL files) that degrades context quality
        // and causes over-cooking. Users can override by adding [pruning] to config.toml.
        if self.pruning.is_none() {
            self.pruning = Some(PruningConfig::default());
        }
    }

    /// Get workspace path
    pub fn workspace_path(&self) -> &std::path::Path {
        &self.workspace
    }

    /// Get sessions path
    pub fn sessions_path(&self) -> &std::path::Path {
        &self.sessions
    }

    /// Save config to ~/.zeus/config.toml
    ///
    /// Includes corruption guards: refuses to save if workspace/sessions point to
    /// temp directories or if the config appears to be uninitialized defaults that
    /// would overwrite a real config file.
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        // Guard: refuse to save if config was loaded from defaults (not from a file).
        // This is the primary defense against the startup bug where Config::default()
        // overwrites a real config.toml after a crash/failed start.
        if self.loaded_from_default {
            return Err(Error::Config(
                "Refusing to save config: loaded from defaults, not from a config file. \
                 This prevents overwriting a real config after a crash."
                    .to_string(),
            ));
        }

        // Guard: refuse to save if any path field points to a temp directory.
        // This prevents Config::default() or onboarding temp state from overwriting
        // a real config file. Shared helper so save()/save_unchecked() can't drift.
        self.reject_temp_paths()?;

        // Guard: if a real config already exists on disk, don't overwrite it with
        // a config that has onboarding_complete = false (likely default/corrupted state).
        if config_path.exists() && !self.onboarding_complete {
            // Read existing config to check if it was previously completed
            if let Ok(existing) = std::fs::read_to_string(&config_path)
                && existing.contains("onboarding_complete = true") {
                    return Err(Error::Config(
                        "Refusing to save config: would overwrite onboarding_complete = true with false".to_string()
                    ));
                }
        }

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Merge strategy: serialize our struct, then merge into the existing
        // TOML on disk. This preserves sections like [channels.discord] and
        // [[bindings]] that may be empty in the struct (skip_serializing_if)
        // but present on disk from manual config or onboarding.
        //
        // Collapse home-rooted paths back to `~/` on a CLONE first (#291): the
        // struct in memory holds expanded absolute paths, but config.toml must
        // store the portable `~/` form so a temp-HOME expansion can't round-trip.
        let mut to_persist = self.clone();
        to_persist.collapse_tildes();
        let new_content =
            toml::to_string_pretty(&to_persist).map_err(|e| Error::Config(e.to_string()))?;

        // Sentinel guard: refuse to persist a config that carries a test sentinel
        // credential. Real configs never contain "sk-ant-test" — only onboarding/
        // config unit tests use it. This is the last line of defense against a test
        // (which loaded a real config, so the loaded_from_default/temp-path/onboarding
        // guards all pass) overwriting a live config.toml with test state.
        Self::reject_sentinel_credentials(&new_content)?;
        let mut config_debris_errors = Vec::new();
        let new_content_has_identity = Self::config_content_has_real_identity(&new_content);
        let content = if config_path.exists() {
            if let Ok(existing_str) = std::fs::read_to_string(&config_path) {
                config_debris_errors.extend(Self::config_debris_errors_for_path(
                    &existing_str,
                    &config_path,
                ));
                if new_content_has_identity
                    && Self::has_identityless_template_debris_at(&existing_str, &config_path)
                {
                    config_debris_errors.push(
                        "identity-less template markers coexisting with real agent identity"
                            .to_string(),
                    );
                }
                if let (Ok(mut existing_table), Ok(new_table)) = (
                    existing_str.parse::<toml::Table>(),
                    new_content.parse::<toml::Table>(),
                ) {
                    for (key, value) in new_table {
                        existing_table.insert(key, value);
                    }
                    toml::to_string_pretty(&existing_table)
                        .unwrap_or(new_content)
                } else {
                    new_content
                }
            } else {
                new_content
            }
        } else {
            new_content
        };

        let content = Self::post_process_config_toml(&content);
        config_debris_errors.extend(Self::config_debris_errors_for_path(
            &content,
            &config_path,
        ));
        if !config_debris_errors.is_empty() {
            config_debris_errors.sort();
            config_debris_errors.dedup();
            return Err(Error::Config(format!(
                "Refusing to write config.toml with debris shape (#309): {}",
                config_debris_errors.join("; ")
            )));
        }
        Self::atomic_write(&config_path, &content)?;
        Self::verify_config_write(&config_path, &content)?;
        Ok(())
    }

    /// Write a complete config.toml body with the same low-level guarantees as
    /// [`Config::save`]: debris-shape rejection, atomic temp+rename, and
    /// verify-after-write. Use this for explicit full-file config writers such
    /// as first-run setup and classic onboarding. Merge-preserving updates
    /// should still use [`Config::save`].
    pub fn write_config_toml_verified(path: &std::path::Path, content: &str) -> Result<()> {
        let content = Self::post_process_config_toml(content);
        Self::reject_sentinel_credentials(&content)?;
        let mut errors = Self::config_debris_errors_for_path(&content, path);
        if !errors.is_empty() {
            errors.sort();
            errors.dedup();
            return Err(Error::Config(format!(
                "Refusing to write config.toml with debris shape (#309): {}",
                errors.join("; ")
            )));
        }
        Self::atomic_write(path, &content)?;
        Self::verify_config_write(path, &content)?;
        Ok(())
    }

    /// Save config without corruption guards. Use only when you are certain the
    /// config is valid (e.g., onboarding wizard completion, explicit user action).
    /// Respects `ZEUS_HOME` env var — safe to call from tests with ZEUS_HOME set.
    pub fn save_unchecked(&self) -> Result<()> {
        let config_path = Self::zeus_home()?.join("config.toml");

        // GUARD (#277): a config that `loaded_from_default` (Config::load() falls
        // back to a guarded default when config.toml is MISSING or unreadable/
        // parse-fails) must NEVER overwrite an EXISTING config file. That
        // read-fail → default → save_unchecked-overwrite chain is the random
        // seat-config nuke (spark/112/106): save_unchecked is the only config
        // writer with no merge and no guards. First-run onboarding (no file on
        // disk yet) is still allowed through, so genuine setup is unaffected.
        if self.loaded_from_default && config_path.exists() {
            return Err(Error::Config(
                "Refusing save_unchecked: config loaded_from_default but config.toml \
                 exists on disk — a read/parse failure must not overwrite the real \
                 config with defaults (#277)"
                    .to_string(),
            ));
        }

        // Guard (#291): refuse to persist temp paths. save_unchecked() is the only
        // config writer with no merge and no guards — it lacked this check while
        // save() had it, which is exactly the drift that nuked fleet configs. Same
        // shared helper as save() so they can never diverge again.
        self.reject_temp_paths()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Collapse home-rooted paths back to `~/` on a CLONE before serialization
        // (#291) so config.toml stores the portable form, not an expanded temp path.
        let mut to_persist = self.clone();
        to_persist.collapse_tildes();
        let content =
            toml::to_string_pretty(&to_persist).map_err(|e| Error::Config(e.to_string()))?;
        Self::reject_sentinel_credentials(&content)?;
        let content = Self::post_process_config_toml(&content);
        let mut errors = Self::config_debris_errors_for_path(&content, &config_path);
        if !errors.is_empty() {
            errors.sort();
            errors.dedup();
            return Err(Error::Config(format!(
                "Refusing to write config.toml with debris shape (#309): {}",
                errors.join("; ")
            )));
        }
        Self::atomic_write(&config_path, &content)?;
        Self::verify_config_write(&config_path, &content)?;
        Ok(())
    }

    /// Safely persist a single secret line into `~/.zeus/.env` via a MERGE.
    ///
    /// 🔴 SACRED (CLAUDE.md primary orders): `.env` holds `DISCORD_BOT_TOKEN`,
    /// `DISCORD_RELAY_CHANNEL_IDS`, and every other secret. A naive truncate-write
    /// once wiped the Discord tokens and silently broke the relay for hours.
    /// This writer NEVER truncates: it reads the existing file, updates the
    /// matching `KEY=...` line in place if present, appends it if absent, and
    /// preserves every other line (including comments and blanks) byte-for-byte.
    ///
    /// Used by onboarding completion to persist the provider API key to the
    /// env-var the runtime actually reads (e.g. `ANTHROPIC_API_KEY` — see
    /// `Provider::env_key()`). Empty `key` or `value` is a no-op (nothing to
    /// persist), so callers can pass through unset fields harmlessly.
    pub fn persist_env_key(key: &str, value: &str) -> Result<()> {
        if key.is_empty() || value.is_empty() {
            return Ok(());
        }
        let env_path = Self::zeus_home()?.join(".env");

        // Read existing content (empty string if the file doesn't exist yet —
        // we create it, never assume a scaffolded default).
        let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
        let content = Self::merge_env_line(&existing, key, value);

        // Reuse the atomic, 0o600 writer — secrets never land world-readable
        // and a crash mid-write can't leave a half-truncated .env.
        Self::atomic_write(&env_path, &content)?;
        Ok(())
    }

    /// Pure merge transform behind [`persist_env_key`]: given the current `.env`
    /// content, return new content with `key=value` updated-in-place (first
    /// match) or appended, preserving every other line byte-for-byte. Factored
    /// out so the SACRED no-clobber invariant is unit-testable without touching
    /// the filesystem or the (thread-unsafe) `ZEUS_HOME` env var.
    fn merge_env_line(existing: &str, key: &str, value: &str) -> String {
        let new_line = format!("{key}={value}");
        let mut replaced = false;
        let mut out_lines: Vec<String> = Vec::new();

        for line in existing.lines() {
            // Match a `KEY=...` assignment for this exact key, tolerating leading
            // whitespace and an optional `export ` prefix. Comments (`# KEY=...`)
            // and unrelated keys are preserved untouched.
            let trimmed = line.trim_start();
            let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
            let is_match = !trimmed.starts_with('#')
                && body
                    .split_once('=')
                    .map(|(k, _)| k.trim() == key)
                    .unwrap_or(false);

            if is_match && !replaced {
                out_lines.push(new_line.clone());
                replaced = true;
            } else {
                out_lines.push(line.to_string());
            }
        }

        if !replaced {
            out_lines.push(new_line);
        }

        // Preserve a trailing newline (POSIX text-file convention).
        let mut content = out_lines.join("\n");
        content.push('\n');
        content
    }

    /// Test-sentinel credentials that must never reach a persisted config.toml.
    /// Real onboarding/users never enter these — they appear only in unit tests.
    /// Used by `save()` and `save_unchecked()` as a last-line defense against a
    /// test overwriting a live config (the bug that wiped fleet configs).
    const SENTINEL_CREDENTIALS: &'static [&'static str] = &["sk-ant-test"];

    /// Refuse to persist serialized config content that contains a test sentinel
    /// credential. Distinguishes test state by credential value, NOT by env
    /// presence — production onboarding legitimately writes ~/.zeus with no
    /// ZEUS_HOME set, so env-based refusal would break real installs.
    fn reject_sentinel_credentials(serialized: &str) -> Result<()> {
        for sentinel in Self::SENTINEL_CREDENTIALS {
            if serialized.contains(sentinel) {
                return Err(Error::Config(format!(
                    "Refusing to save config: contains test sentinel credential '{}'. \
                     This is a guard against tests overwriting a real config.toml.",
                    sentinel
                )));
            }
        }
        Ok(())
    }

    fn config_debris_errors_for_path(content: &str, path: &std::path::Path) -> Vec<String> {
        let (mut errors, _has_real_identity, has_any_identity, _identityless_template_marker) =
            Self::scan_config_shape(content);
        if !has_any_identity
            && path
                .parent()
                .map(Self::config_home_has_veteran_state)
                .unwrap_or(false)
        {
            errors.push("identity-less template config marker with veteran state".to_string());
        }
        errors
    }

    fn has_identityless_template_debris_at(content: &str, path: &std::path::Path) -> bool {
        let (_errors, _has_real_identity, has_any_identity, _identityless_template_marker) =
            Self::scan_config_shape(content);
        !has_any_identity
            && path
                .parent()
                .map(Self::config_home_has_veteran_state)
                .unwrap_or(false)
    }

    fn config_content_has_real_identity(content: &str) -> bool {
        let (_errors, has_real_identity, _has_any_identity, _identityless_template_marker) =
            Self::scan_config_shape(content);
        has_real_identity
    }

    fn config_home_has_veteran_state(zeus_home: &std::path::Path) -> bool {
        const MARKERS: &[&str] = &[
            "memory.db",
            "sessions",
            "goals.db",
            "scheduler.db",
            "learning.db",
            "cooking_checkpoints.db",
        ];
        MARKERS.iter().any(|marker| zeus_home.join(marker).exists())
    }

    fn scan_config_shape(content: &str) -> (Vec<String>, bool, bool, bool) {
        let mut errors = Vec::new();
        let mut seen_top_level_keys = HashSet::new();
        let mut seen_top_level_tables = HashSet::new();
        let mut current_table: Option<String> = None;
        let mut agent_name: Option<String> = None;
        let mut agent_persona: Option<String> = None;
        let mut top_level_name: Option<String> = None;
        let mut top_level_persona: Option<String> = None;
        let mut model_is_template_default = false;

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.starts_with('[') {
                if let Some(end) = line.find(']') {
                    let header = &line[..=end];
                    let table = header.trim_matches(&['[', ']'][..]).to_string();
                    let is_array_table = line.starts_with("[[");
                    current_table = Some(table.clone());
                    if !is_array_table
                        && !table.contains('.')
                        && !seen_top_level_tables.insert(table.clone())
                    {
                        errors.push(format!("duplicate top-level table [{}]", table));
                    }
                }
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim();
            let value = Self::toml_line_value(raw_value);

            match current_table.as_deref() {
                None => {
                    if !key.contains('.') && !seen_top_level_keys.insert(key.to_string()) {
                        errors.push(format!("duplicate top-level key {}", key));
                    }
                    match key {
                        "model" if value == "ollama/llama3.2" => {
                            model_is_template_default = true;
                        }
                        "name" if !value.is_empty() => top_level_name = Some(value),
                        "persona" if !value.is_empty() => top_level_persona = Some(value),
                        _ => {}
                    }
                }
                Some("agent") => match key {
                    "name" if !value.is_empty() => agent_name = Some(value),
                    "persona" if !value.is_empty() => agent_persona = Some(value),
                    _ => {}
                },
                Some("network") => match key {
                    "agent_name" if !value.is_empty() => top_level_name = Some(value),
                    _ => {}
                },
                _ => {}
            }
        }

        let identity = agent_name.as_deref().or(top_level_name.as_deref());
        let has_any_identity = identity.map(|name| !name.is_empty()).unwrap_or(false);
        let has_real_identity = identity
            .map(|name| !name.starts_with('$') && name != "zeus")
            .unwrap_or(false);
        let template_persona = agent_persona
            .as_deref()
            .or(top_level_persona.as_deref())
            .map(Self::is_template_persona_marker)
            .unwrap_or(false);
        let identityless_template_marker =
            model_is_template_default && template_persona && !has_any_identity;

        (errors, has_real_identity, has_any_identity, identityless_template_marker)
    }

    fn toml_line_value(raw_value: &str) -> String {
        raw_value
            .split('#')
            .next()
            .unwrap_or(raw_value)
            .trim()
            .trim_matches('"')
            .to_string()
    }

    fn is_template_persona_marker(value: &str) -> bool {
        matches!(value, "The Herald" | "helpful, precise, and proactive")
    }

    /// Verify the final bytes and debris shape after atomic rename. This closes
    /// the save-path gap where a temp write could succeed but the durable file was
    /// not the clean config shape the writer intended.
    fn verify_config_write(path: &std::path::Path, expected: &str) -> Result<()> {
        let written = std::fs::read_to_string(path).map_err(|e| {
            Error::Config(format!("Config verify-after-write read failed: {}", e))
        })?;
        if written != expected {
            return Err(Error::Config(
                "Config verify-after-write mismatch after atomic rename".to_string(),
            ));
        }

        let mut errors = Self::config_debris_errors_for_path(&written, path);
        if !errors.is_empty() {
            errors.sort();
            errors.dedup();
            return Err(Error::Config(format!(
                "Config verify-after-write found debris shape (#309): {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }

    /// Post-process generated TOML to apply Track C blockers:
    /// 1. Ensure `[talos]` block exists on macOS.
    /// 2. Migrate legacy `[images]` → `[talos.image]`.
    /// 3. Ensure `[prometheus.heartbeat]` is not stripped.
    fn post_process_config_toml(content: &str) -> String {
        let mut out = content.to_string();

        // ── Fix 1: [talos] always-write on macOS ──
        #[cfg(target_os = "macos")]
        if !out.contains("[talos]") {
            out.push_str("\n[talos]\ncalendar = true\nnotes = true\nreminders = true\ncontacts = true\nbrowser = true\nsystem = true\nnetwork = true\n");
        }

        // ── Fix 2: [images] → [talos.image] migration ──
        // If legacy [images] exists and [talos.image] does not, migrate fields.
        if out.contains("[images]") && !out.contains("[talos.image]") {
            // Extract fields from [images] block
            let mut provider = None;
            let mut url = None;
            let mut api_key = None;
            let mut model = None;
            let mut default_width = None;
            let mut default_height = None;
            let mut store_path = None;

            let mut in_images = false;
            let mut lines = out.lines().peekable();
            let mut new_lines: Vec<String> = Vec::new();

            while let Some(line) = lines.next() {
                if line.trim() == "[images]" {
                    in_images = true;
                    continue;
                }
                if in_images {
                    if line.trim().starts_with('[') {
                        in_images = false;
                        new_lines.push(line.to_string());
                        continue;
                    }
                    if let Some((k, v)) = line.split_once('=') {
                        let key = k.trim();
                        let val = v.trim().trim_matches('"');
                        match key {
                            "provider" => provider = Some(val.to_string()),
                            "url" => url = Some(val.to_string()),
                            "api_key" => api_key = Some(val.to_string()),
                            "model" => model = Some(val.to_string()),
                            "default_width" => default_width = Some(val.to_string()),
                            "default_height" => default_height = Some(val.to_string()),
                            "store_path" => store_path = Some(val.to_string()),
                            _ => {}
                        }
                    }
                    continue;
                }
                new_lines.push(line.to_string());
            }

            // Build [talos.image] block
            let mut talos_image = vec!["[talos.image]".to_string()];
            if let Some(v) = provider { talos_image.push(format!("provider = \"{}\"", v)); }
            if let Some(v) = url { talos_image.push(format!("url = \"{}\"", v)); }
            if let Some(v) = api_key { talos_image.push(format!("api_key = \"{}\"", v)); }
            if let Some(v) = model { talos_image.push(format!("model = \"{}\"", v)); }
            if let Some(v) = default_width { talos_image.push(format!("default_width = {}", v)); }
            if let Some(v) = default_height { talos_image.push(format!("default_height = {}", v)); }
            if let Some(v) = store_path { talos_image.push(format!("store_path = \"{}\"", v)); }

            new_lines.push("".to_string());
            new_lines.extend(talos_image);

            out = new_lines.join("\n");
        }

        // ── Fix 3: [prometheus.heartbeat] persistence ──
        // If [prometheus] exists but [prometheus.heartbeat] does not, and the
        // config has heartbeat settings, ensure the nested block is preserved.
        // This is a no-op for serde-generated TOML (it writes nested tables
        // correctly), but guards against external scripts that strip it.
        if out.contains("[prometheus]") && !out.contains("[prometheus.heartbeat]") {
            // Add a minimal [prometheus.heartbeat] placeholder so external
            // tools know the section is intentional.
            out.push_str("\n[prometheus.heartbeat]\n# Heartbeat runtime config — managed by orchestration\n");
        }

        out
    }

    /// Write content to a file atomically: write to a temp file in the same
    /// directory, fsync, then rename over the target. Prevents truncated/empty
    /// config if the process crashes mid-write.
    fn atomic_write(path: &std::path::Path, content: &str) -> Result<()> {
        use std::io::Write;

        let parent = path.parent().ok_or_else(|| {
            Error::Config("Config path has no parent directory".to_string())
        })?;

        // Build temp path in same directory (same filesystem = atomic rename)
        let tmp_name = format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.toml")
        );
        let tmp_path = parent.join(&tmp_name);

        // Write to temp file
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp_path)?;
                f.write_all(content.as_bytes())?;
                f.sync_all()?;
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&tmp_path, content)?;
            }
        }

        // Backup existing config before overwriting (prevents data loss from partial writes)
        if path.exists() {
            let backup_path = parent.join("config.toml.known-good");
            if let Ok(existing) = std::fs::read_to_string(path) {
                // Only backup if existing file has real content (>10 bytes)
                if existing.len() > 10 {
                    let _ = std::fs::write(&backup_path, &existing);
                }
            }
        }

        // Atomic rename over the real config
        std::fs::rename(&tmp_path, path).map_err(|e| {
            // Clean up temp file on rename failure
            let _ = std::fs::remove_file(&tmp_path);
            Error::Config(format!("Atomic rename failed: {}", e))
        })?;

        Ok(())
    }

    /// Validate configuration, returning warnings for potential issues
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Check model string format
        if self.model.is_empty() {
            warnings.push("Model string is empty, will use default".to_string());
        } else if !self.model.contains('/') {
            warnings.push(format!(
                "Model '{}' has no provider prefix (e.g., 'anthropic/model-name'). Provider will be auto-detected.",
                self.model
            ));
        }

        // Check API key for selected provider — skip if model is empty (pre-onboarding)
        if !self.model.is_empty() {
        let (provider, _) = self.parse_model();
        match provider {
            Provider::Anthropic => {
                if std::env::var("ANTHROPIC_API_KEY").is_err() && !self.auth.use_oauth {
                    warnings.push("ANTHROPIC_API_KEY not set and OAuth not enabled. Anthropic calls will fail.".to_string());
                }
            }
            Provider::OpenAI => {
                if std::env::var("OPENAI_API_KEY").is_err() {
                    warnings.push("OPENAI_API_KEY not set. OpenAI calls will fail.".to_string());
                }
            }
            Provider::OpenRouter => {
                if std::env::var("OPENROUTER_API_KEY").is_err() {
                    warnings.push(
                        "OPENROUTER_API_KEY not set. OpenRouter calls will fail.".to_string(),
                    );
                }
            }
            Provider::Ollama => {
                // Ollama doesn't need an API key, just a running server
            }
            Provider::Google => {
                if std::env::var("GOOGLE_API_KEY").is_err() {
                    warnings.push("GOOGLE_API_KEY not set. Gemini calls will fail.".to_string());
                }
            }
            Provider::Groq => {
                if std::env::var("GROQ_API_KEY").is_err() {
                    warnings.push("GROQ_API_KEY not set. Groq calls will fail.".to_string());
                }
            }
            Provider::Mistral => {
                if std::env::var("MISTRAL_API_KEY").is_err() {
                    warnings.push("MISTRAL_API_KEY not set. Mistral calls will fail.".to_string());
                }
            }
            Provider::Together => {
                if std::env::var("TOGETHER_API_KEY").is_err() {
                    warnings
                        .push("TOGETHER_API_KEY not set. Together AI calls will fail.".to_string());
                }
            }
            Provider::Fireworks => {
                if std::env::var("FIREWORKS_API_KEY").is_err() {
                    warnings.push(
                        "FIREWORKS_API_KEY not set. Fireworks AI calls will fail.".to_string(),
                    );
                }
            }
            Provider::Azure => {
                if std::env::var("AZURE_OPENAI_API_KEY").is_err() {
                    warnings.push(
                        "AZURE_OPENAI_API_KEY not set. Azure OpenAI calls will fail.".to_string(),
                    );
                }
                if std::env::var("AZURE_OPENAI_ENDPOINT").is_err() {
                    warnings.push(
                        "AZURE_OPENAI_ENDPOINT not set. Azure OpenAI calls will fail.".to_string(),
                    );
                }
                if std::env::var("AZURE_OPENAI_DEPLOYMENT").is_err() {
                    warnings.push(
                        "AZURE_OPENAI_DEPLOYMENT not set. Azure OpenAI calls will fail."
                            .to_string(),
                    );
                }
            }
            Provider::Bedrock => {
                if std::env::var("AWS_ACCESS_KEY_ID").is_err() {
                    warnings
                        .push("AWS_ACCESS_KEY_ID not set. Bedrock calls will fail.".to_string());
                }
                if std::env::var("AWS_SECRET_ACCESS_KEY").is_err() {
                    warnings.push(
                        "AWS_SECRET_ACCESS_KEY not set. Bedrock calls will fail.".to_string(),
                    );
                }
            }
            Provider::DeepSeek => {
                if std::env::var("DEEPSEEK_API_KEY").is_err() {
                    warnings.push("DEEPSEEK_API_KEY not set. DeepSeek calls will fail.".to_string());
                }
            }
            Provider::XAI => {
                if std::env::var("XAI_API_KEY").is_err() {
                    warnings.push("XAI_API_KEY not set. xAI/Grok calls will fail.".to_string());
                }
            }
            Provider::Cerebras => {
                if std::env::var("CEREBRAS_API_KEY").is_err() {
                    warnings.push("CEREBRAS_API_KEY not set. Cerebras calls will fail.".to_string());
                }
            }
            Provider::Moonshot => {
                if std::env::var("MOONSHOT_API_KEY").is_err() {
                    warnings.push("MOONSHOT_API_KEY not set. Kimi/Moonshot calls will fail.".to_string());
                }
            }
            Provider::Zai => {
                if std::env::var("ZAI_API_KEY").is_err() {
                    warnings.push("ZAI_API_KEY not set. GLM/Zhipu calls will fail.".to_string());
                }
            }
            Provider::Qwen => {
                if std::env::var("QWEN_API_KEY").is_err()
                    && std::env::var("DASHSCOPE_API_KEY").is_err()
                    && std::env::var("MODELSTUDIO_API_KEY").is_err()
                {
                    warnings.push("QWEN_API_KEY not set. Qwen/Alibaba calls will fail.".to_string());
                }
            }
            Provider::Minimax => {
                // OAuth-only provider — no API key required
            }
            Provider::XiaomiMimo => {
                if std::env::var("XIAOMIMIMO_API_KEY").is_err() {
                    warnings.push("XIAOMIMIMO_API_KEY not set. Xiaomi MiMo calls will fail.".to_string());
                }
            }
            Provider::Sakana => {
                if std::env::var("SAKANA_API_KEY").is_err() {
                    warnings.push("SAKANA_API_KEY not set. Sakana Fugu calls will fail.".to_string());
                }
            }
            Provider::GoogleGeminiCli => {
                // OAuth-only provider — no API key required
            }
        }
        } // end if !self.model.is_empty()

        // Validate paths can be created
        if !self.workspace.exists()
            && let Some(parent) = self.workspace.parent()
            && !parent.exists()
        {
            warnings.push(format!(
                "Workspace parent directory does not exist: {}",
                parent.display()
            ));
        }

        // Validate max_iterations
        if self.max_iterations == 0 {
            warnings.push(
                "max_iterations is 0, agent will not be able to process messages".to_string(),
            );
        } else if self.max_iterations > 100 {
            warnings.push(format!(
                "max_iterations is very high ({}), consider reducing to avoid runaway loops",
                self.max_iterations
            ));
        }

        // Validate channel configs
        if let Some(ref channels) = self.channels {
            if let Some(ref tg) = channels.telegram {
                // Suppress MTProto warnings when using bot_token mode
                // (api_id/api_hash are not needed for bot auth)
                let uses_bot_token = tg.bot_token.as_ref().is_some_and(|t| !t.is_empty());
                if !uses_bot_token {
                    if tg.api_id == 0 {
                        warnings.push(
                            "Telegram api_id is 0, get your API credentials from my.telegram.org"
                                .to_string(),
                        );
                    }
                    if tg.api_hash.is_empty() {
                        warnings.push("Telegram api_hash is empty".to_string());
                    }
                }
            }
            if let Some(ref dc) = channels.discord
                && dc.token.is_empty()
                && dc.accounts.values().all(|a| a.token.is_empty())
            {
                warnings.push("Discord token is empty".to_string());
            }
            if let Some(ref sl) = channels.slack
                && sl.bot_token.is_empty()
            {
                warnings.push("Slack bot_token is empty".to_string());
            }
            if let Some(ref em) = channels.email {
                if em.username.is_empty() {
                    warnings.push("Email username is empty".to_string());
                }
                if em.smtp_host.is_empty() {
                    warnings.push("Email SMTP host is empty".to_string());
                }
            }
            if let Some(ref wa) = channels.whatsapp
                && wa.bridge_url.is_empty()
            {
                warnings.push("WhatsApp bridge_url is empty".to_string());
            }
            if let Some(ref sg) = channels.signal
                && sg.phone.is_empty()
            {
                warnings.push("Signal phone number is empty".to_string());
            }
            if let Some(ref mx) = channels.matrix {
                if mx.homeserver.is_empty() {
                    warnings.push("Matrix homeserver URL is empty".to_string());
                }
                if mx.access_token.is_empty() {
                    warnings.push("Matrix access_token is empty".to_string());
                }
            }
            if let Some(ref mq) = channels.mqtt
                && mq.broker_url.is_empty()
            {
                warnings.push("MQTT broker_url is empty".to_string());
            }
        }

        warnings
    }

    /// Validate channel configurations specifically, returning per-channel warnings.
    pub fn validate_channels(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let Some(ref channels) = self.channels else {
            return warnings;
        };

        if let Some(ref tg) = channels.telegram {
            // Suppress MTProto warnings when using bot_token mode
            let uses_bot_token = tg.bot_token.as_ref().is_some_and(|t| !t.is_empty());
            if !uses_bot_token {
                if tg.api_id == 0 {
                    warnings.push("Telegram: api_id is 0".to_string());
                }
                if tg.api_hash.is_empty() {
                    warnings.push("Telegram: api_hash is empty".to_string());
                }
                if tg.phone.is_empty() {
                    warnings.push("Telegram: phone is empty".to_string());
                }
            }
        }
        if let Some(ref dc) = channels.discord
            && dc.token.is_empty()
        {
            warnings.push("Discord: token is empty".to_string());
        }
        if let Some(ref sl) = channels.slack {
            if sl.bot_token.is_empty() {
                warnings.push("Slack: bot_token is empty".to_string());
            }
            if sl.app_token.is_empty() {
                warnings.push("Slack: app_token is empty".to_string());
            }
        }
        if let Some(ref em) = channels.email {
            if em.username.is_empty() {
                warnings.push("Email: username is empty".to_string());
            }
            if em.smtp_host.is_empty() {
                warnings.push("Email: smtp_host is empty".to_string());
            }
            if em.imap_host.is_empty() {
                warnings.push("Email: imap_host is empty".to_string());
            }
        }
        if let Some(ref wa) = channels.whatsapp
            && wa.bridge_url.is_empty()
        {
            warnings.push("WhatsApp: bridge_url is empty".to_string());
        }
        if let Some(ref sg) = channels.signal
            && sg.phone.is_empty()
        {
            warnings.push("Signal: phone is empty".to_string());
        }
        if let Some(ref mx) = channels.matrix {
            if mx.homeserver.is_empty() {
                warnings.push("Matrix: homeserver is empty".to_string());
            }
            if mx.access_token.is_empty() {
                warnings.push("Matrix: access_token is empty".to_string());
            }
        }
        if let Some(ref mq) = channels.mqtt
            && mq.broker_url.is_empty()
        {
            warnings.push("MQTT: broker_url is empty".to_string());
        }
        if let Some(ref irc) = channels.irc {
            if irc.server.is_empty() {
                warnings.push("IRC: server is empty".to_string());
            }
            if irc.nick.is_empty() {
                warnings.push("IRC: nick is empty".to_string());
            }
        }
        if let Some(ref tw) = channels.twitch {
            if tw.oauth_token.is_empty() {
                warnings.push("Twitch: oauth_token is empty".to_string());
            }
            if tw.username.is_empty() {
                warnings.push("Twitch: username is empty".to_string());
            }
        }

        warnings
    }

    /// Detect available API keys in the environment and return (provider, env_var) pairs.
    pub fn detect_environment() -> Vec<(Provider, &'static str)> {
        let checks: &[(Provider, &str)] = &[
            (Provider::Anthropic, "ANTHROPIC_API_KEY"),
            (Provider::OpenAI, "OPENAI_API_KEY"),
            (Provider::OpenRouter, "OPENROUTER_API_KEY"),
            (Provider::Google, "GOOGLE_API_KEY"),
            (Provider::Groq, "GROQ_API_KEY"),
            (Provider::Mistral, "MISTRAL_API_KEY"),
            (Provider::Together, "TOGETHER_API_KEY"),
            (Provider::Fireworks, "FIREWORKS_API_KEY"),
            (Provider::Azure, "AZURE_OPENAI_API_KEY"),
            (Provider::Bedrock, "AWS_ACCESS_KEY_ID"),
            (Provider::XAI, "XAI_API_KEY"),
            (Provider::Moonshot, "MOONSHOT_API_KEY"),
            (Provider::Zai, "ZAI_API_KEY"),
            (Provider::Qwen, "QWEN_API_KEY"),
            (Provider::XiaomiMimo, "XIAOMIMIMO_API_KEY"),
            (Provider::Sakana, "SAKANA_API_KEY"),
        ];

        checks
            .iter()
            .filter(|(_, env_var)| std::env::var(env_var).is_ok())
            .map(|(p, v)| (*p, *v))
            .collect()
    }

    /// Suggest the best provider based on available environment variables.
    /// Returns (provider, default_model) or None if nothing is detected.
    pub fn suggest_provider() -> Option<(Provider, &'static str)> {
        let detected = Self::detect_environment();
        // Priority order: Anthropic > OpenAI > OpenRouter > Google > Groq > others
        // No hardcoded model names — just detect provider, user selects model during onboarding
        let priority: &[(Provider, &str)] = &[
            (Provider::Anthropic, "anthropic/"),
            (Provider::OpenAI, "openai/"),
            (Provider::OpenRouter, "openrouter/"),
            (Provider::Google, "google/"),
            (Provider::Groq, "groq/"),
            (Provider::Mistral, "mistral/"),
            (Provider::Together, "together/"),
            (Provider::Fireworks, "fireworks/"),
            (Provider::Azure, "azure/"),
            (Provider::Bedrock, "bedrock/"),
            (Provider::XAI, "xai/"),
            (Provider::Moonshot, "moonshot/"),
            (Provider::Zai, "zai/"),
            (Provider::Qwen, "qwen/"),
            (Provider::XiaomiMimo, "xiaomimimo/"),
            (Provider::Sakana, "sakana/"),
        ];

        for (provider, model) in priority {
            if detected.iter().any(|(p, _)| p == provider) {
                return Some((*provider, model));
            }
        }
        None
    }

    /// Export config to a TOML string.
    pub fn export_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("Serialization error: {}", e)))
    }

    /// Import config from a TOML string.
    pub fn import_toml(content: &str) -> Result<Self> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }

    /// Export config to a file path.
    pub fn export_to_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let content = self.export_toml()?;
        std::fs::write(path.as_ref(), content)?;
        Ok(())
    }

    /// Import config from a file path.
    pub fn import_from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        Self::import_toml(&content)
    }

    /// Parse model string into (provider, model)
    pub fn parse_model(&self) -> (Provider, String) {
        if let Some((provider, model)) = self.model.split_once('/') {
            let p = Provider::from_prefix(provider);
            (p, model.to_string())
        } else {
            // Auto-detect from model name
            let model = &self.model;
            if model.contains("claude") {
                (Provider::Anthropic, model.clone())
            } else if model.contains("gpt") || model.contains("o1") || model.contains("o3") {
                (Provider::OpenAI, model.clone())
            } else if model.contains("gemini") {
                (Provider::Google, model.clone())
            } else if model.contains("llama") || model.contains("mistral") || model.contains("qwen")
            {
                (Provider::Ollama, model.clone())
            } else if model.contains("mimo") {
                (Provider::XiaomiMimo, model.clone())
            } else {
                (Provider::Anthropic, model.clone())
            }
        }
    }
}

// ============================================================================
// Provider (~20 lines)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    OpenAI,
    Ollama,
    OpenRouter,
    Google,
    Groq,
    Mistral,
    Together,
    Fireworks,
    Azure,
    Bedrock,
    DeepSeek,
    XAI,
    Cerebras,
    #[serde(rename = "google-gemini-cli")]
    GoogleGeminiCli,
    Moonshot,
    Zai,
    Qwen,
    Minimax,
    #[serde(rename = "xiaomimimo")]
    XiaomiMimo,
    Sakana,
}

impl Provider {
    /// Map a model-string prefix (the `{provider}` in `{provider}/{model}`) to a
    /// `Provider`. Accepts display aliases (`glm`, `kimi`, `grok`, …) as well as
    /// canonical ids. Single source of truth for prefix→Provider resolution —
    /// `Config::parse_model` and onboarding's model-string canonicalization both
    /// route through here, guaranteeing the persisted prefix resolves identically
    /// at the gateway read-side. Unknown prefixes default to Anthropic (warned).
    pub fn from_prefix(prefix: &str) -> Provider {
        match prefix.to_lowercase().as_str() {
            "anthropic" => Provider::Anthropic,
            "openai" => Provider::OpenAI,
            "ollama" => Provider::Ollama,
            "openrouter" => Provider::OpenRouter,
            "google" | "gemini" => Provider::Google,
            "groq" => Provider::Groq,
            "mistral" => Provider::Mistral,
            "together" => Provider::Together,
            "fireworks" => Provider::Fireworks,
            "azure" => Provider::Azure,
            "bedrock" | "aws" => Provider::Bedrock,
            "deepseek" => Provider::DeepSeek,
            "xai" | "grok" => Provider::XAI,
            "cerebras" => Provider::Cerebras,
            "moonshot" | "kimi" => Provider::Moonshot,
            "zai" | "glm" | "zhipu" => Provider::Zai,
            "qwen" | "dashscope" | "modelstudio" => Provider::Qwen,
            "minimax" => Provider::Minimax,
            "xiaomimimo" | "mimo" => Provider::XiaomiMimo,
            "sakana" | "fugu" => Provider::Sakana,
            // "gemini-cli" is the TUI provider-screen id (screens/providers.rs);
            // "google-gemini-cli" is the canonical config/serde name. Both must
            // resolve here or onboarding's `from_prefix(display_id)` canonicalize
            // step (app.rs collect_and_persist) silently defaults gemini-cli →
            // Anthropic, mis-routing both the model AND its OAuth token. (#257)
            "google-gemini-cli" | "gemini-cli" => Provider::GoogleGeminiCli,
            other => {
                eprintln!("Warning: Unknown provider '{other}', defaulting to Anthropic");
                Provider::Anthropic
            }
        }
    }

    /// Lowercase provider name (used as key in credential store)
    pub fn name(&self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::Ollama => "ollama",
            Provider::OpenRouter => "openrouter",
            Provider::Google => "google",
            Provider::Groq => "groq",
            Provider::Mistral => "mistral",
            Provider::Together => "together",
            Provider::Fireworks => "fireworks",
            Provider::Azure => "azure",
            Provider::Bedrock => "bedrock",
            Provider::DeepSeek => "deepseek",
            Provider::XAI => "xai",
            Provider::Cerebras => "cerebras",
            Provider::Moonshot => "moonshot",
            Provider::Zai => "zai",
            Provider::Qwen => "qwen",
            Provider::Minimax => "minimax",
            Provider::XiaomiMimo => "xiaomimimo",
            Provider::Sakana => "sakana",
            Provider::GoogleGeminiCli => "google-gemini-cli",
        }
    }

    pub fn env_key(&self) -> &'static str {
        match self {
            Provider::Anthropic => "ANTHROPIC_API_KEY",
            Provider::OpenAI => "OPENAI_API_KEY",
            Provider::Ollama => "OLLAMA_HOST",
            Provider::OpenRouter => "OPENROUTER_API_KEY",
            Provider::Google => "GOOGLE_API_KEY",
            Provider::Groq => "GROQ_API_KEY",
            Provider::Mistral => "MISTRAL_API_KEY",
            Provider::Together => "TOGETHER_API_KEY",
            Provider::Fireworks => "FIREWORKS_API_KEY",
            Provider::Azure => "AZURE_OPENAI_API_KEY",
            Provider::Bedrock => "AWS_ACCESS_KEY_ID",
            Provider::DeepSeek => "DEEPSEEK_API_KEY",
            Provider::XAI => "XAI_API_KEY",
            Provider::Cerebras => "CEREBRAS_API_KEY",
            Provider::Moonshot => "MOONSHOT_API_KEY",
            Provider::Zai => "ZAI_API_KEY",
            Provider::Qwen => "QWEN_API_KEY",
            Provider::Minimax => "MINIMAX_API_KEY",
            Provider::XiaomiMimo => "XIAOMIMIMO_API_KEY",
            Provider::Sakana => "SAKANA_API_KEY",
            Provider::GoogleGeminiCli => "",
        }
    }
}

// ============================================================================
// Messages (~80 lines)
// ============================================================================

/// File or media attachment on a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// MIME type (e.g. "image/jpeg", "audio/mp3")
    pub mime_type: String,
    /// Raw bytes of the attachment (empty if source_url is used instead)
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
    /// Optional filename
    pub filename: Option<String>,
    /// Optional source URL — when set, providers that support URL references
    /// can pass the URL directly instead of base64-encoding the data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl Attachment {
    /// Create an attachment from raw bytes
    pub fn from_data(mime_type: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            mime_type: mime_type.into(),
            data,
            filename: None,
            source_url: None,
        }
    }

    /// Create an attachment that references an image URL.
    /// Data is empty — providers will use the URL directly or the caller
    /// can populate data later via `resolve_url()`.
    pub fn from_url(url: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            data: Vec::new(),
            filename: None,
            source_url: Some(url.into()),
        }
    }

    /// Whether this attachment has inline data
    pub fn has_data(&self) -> bool {
        !self.data.is_empty()
    }

    /// Whether this attachment is a URL reference
    pub fn is_url_ref(&self) -> bool {
        self.source_url.is_some() && self.data.is_empty()
    }

    /// Whether this is an image attachment
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

/// Serde helper for base64-encoding Vec<u8>
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

/// Text direction for bidi rendering support.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    /// Left-to-right (default for Latin, Cyrillic, CJK, etc.)
    #[default]
    Ltr,
    /// Right-to-left (Hebrew, Arabic, Syriac, Thaana, etc.)
    Rtl,
}

/// Detect whether a string is predominantly RTL.
///
/// Scans the first 200 characters for strong directional Unicode codepoints.
/// Returns `true` if RTL characters outnumber LTR characters.
///
/// Covers: Arabic (U+0600–06FF, U+0750–077F, U+FB50–FDFF, U+FE70–FEFF),
/// Hebrew (U+0590–05FF, U+FB1D–FB4F), Syriac (U+0700–074F),
/// Thaana (U+0780–07BF), N'Ko (U+07C0–07FF).
pub fn is_rtl(text: &str) -> bool {
    let mut rtl = 0u32;
    let mut ltr = 0u32;
    for ch in text.chars().take(200) {
        match ch {
            // Arabic block + Arabic Supplement
            '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' => rtl += 1,
            // Arabic Presentation Forms A & B
            '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}' => rtl += 1,
            // Hebrew block + Hebrew presentation forms
            '\u{0590}'..='\u{05FF}' | '\u{FB1D}'..='\u{FB4F}' => rtl += 1,
            // Syriac
            '\u{0700}'..='\u{074F}' => rtl += 1,
            // Thaana (Maldivian)
            '\u{0780}'..='\u{07BF}' => rtl += 1,
            // N'Ko
            '\u{07C0}'..='\u{07FF}' => rtl += 1,
            // Latin, Cyrillic, Greek, CJK — strong LTR
            'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' => ltr += 1,
            '\u{0400}'..='\u{04FF}' => ltr += 1, // Cyrillic
            '\u{0370}'..='\u{03FF}' => ltr += 1, // Greek
            '\u{4E00}'..='\u{9FFF}' => ltr += 1, // CJK Unified
            '\u{3040}'..='\u{30FF}' => ltr += 1, // Hiragana + Katakana
            '\u{AC00}'..='\u{D7AF}' => ltr += 1, // Hangul
            _ => {}
        }
    }
    rtl > 0 && rtl >= ltr
}

/// Detect text direction from content.
pub fn detect_direction(text: &str) -> TextDirection {
    if is_rtl(text) {
        TextDirection::Rtl
    } else {
        TextDirection::Ltr
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_results: Vec<ToolResult>,
    pub timestamp: DateTime<Utc>,
    /// Attached files/images/audio
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Unique message identifier (auto-generated UUID)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Parent message ID for reply threading
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Thread ID grouping related messages
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Text direction (auto-detected from content, frontends use for CSS dir attribute)
    #[serde(default)]
    pub direction: TextDirection,
    /// Channel source — where this message originated (S72: unified sessions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_source: Option<ChannelSource>,
    /// Compaction hint — controls whether this message survives session compaction.
    /// Preserve: task assignments, coordinator directives, commit refs — never compacted.
    /// Ephemeral: heartbeat acks, status pings — compacted first.
    /// Normal (default): standard compaction rules apply.
    #[serde(default)]
    pub compaction_hint: CompactionHint,
}

/// Controls message behavior during session compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompactionHint {
    /// Standard compaction rules apply (default)
    #[default]
    Normal,
    /// Never compact — task assignments, coordinator directives, critical context
    Preserve,
    /// Compact first — heartbeat acks, status pings, ephemeral chatter
    Ephemeral,
}

/// Metadata about which channel a message came from (S72: unified sessions)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelSource {
    /// Channel type: "discord", "telegram", "slack", "tui", "api", etc.
    pub channel_type: String,
    /// Channel/room ID (platform-specific)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Human-readable channel name (e.g. "#devs")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    /// Sender display name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    /// Sender platform ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub success: bool,
    pub output: String,
}

// ============================================================================
// Taint Tracking Labels
// ============================================================================

/// Information-flow taint labels for tracking data provenance through tool chains.
///
/// Labels form a power-set lattice: data can carry multiple labels simultaneously.
/// Sink checks prevent tainted data from reaching unauthorized destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintLabel {
    /// Data originating from external network (web_fetch, API responses)
    ExternalNetwork,
    /// Data from user input (chat messages, CLI args)
    UserInput,
    /// Personally identifiable information (emails, phones, SSNs)
    Pii,
    /// Secrets: API keys, tokens, passwords, credentials
    Secret,
    /// Data from untrusted or spawned agents
    UntrustedAgent,
}

impl std::fmt::Display for TaintLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaintLabel::ExternalNetwork => write!(f, "external_network"),
            TaintLabel::UserInput => write!(f, "user_input"),
            TaintLabel::Pii => write!(f, "pii"),
            TaintLabel::Secret => write!(f, "secret"),
            TaintLabel::UntrustedAgent => write!(f, "untrusted_agent"),
        }
    }
}

/// A set of taint labels attached to a data value.
///
/// Uses power-set lattice semantics: join = union, meet = intersection.
/// Empty set = untainted (bottom of lattice).
pub type TaintSet = std::collections::HashSet<TaintLabel>;

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        let content = content.into();
        let direction = detect_direction(&content);
        Self {
            role: Role::User,
            content,
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            thread_id: None,
            direction,
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    /// Create a user message with file/image attachments
    pub fn user_with_attachments(content: impl Into<String>, attachments: Vec<Attachment>) -> Self {
        let content = content.into();
        let direction = detect_direction(&content);
        Self {
            role: Role::User,
            content,
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments,
            message_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            thread_id: None,
            direction,
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        let content = content.into();
        let direction = detect_direction(&content);
        Self {
            role: Role::Assistant,
            content,
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            thread_id: None,
            direction,
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        let content = content.into();
        let direction = detect_direction(&content);
        Self {
            role: Role::System,
            content,
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            thread_id: None,
            direction,
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    pub fn tool(call_id: impl Into<String>, success: bool, output: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: String::new(),
            tool_calls: vec![],
            tool_results: vec![ToolResult {
                call_id: call_id.into(),
                success,
                output: output.into(),
            }],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            thread_id: None,
            direction: TextDirection::Ltr, // Tool output is always LTR
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    /// Set compaction hint on a message
    pub fn with_compaction_hint(mut self, hint: CompactionHint) -> Self {
        self.compaction_hint = hint;
        self
    }

    /// Set channel source metadata on a message
    pub fn with_channel_source(mut self, source: ChannelSource) -> Self {
        self.channel_source = Some(source);
        self
    }

    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Set the parent message ID (for reply threading)
    pub fn with_parent(mut self, parent_id: &str) -> Self {
        self.parent_id = Some(parent_id.to_string());
        self
    }

    /// Set the thread ID (groups related messages)
    pub fn with_thread(mut self, thread_id: &str) -> Self {
        self.thread_id = Some(thread_id.to_string());
        self
    }
}

// ============================================================================
// Tool Schema (~50 lines)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    pub fn with_param(
        mut self,
        name: &str,
        param_type: &str,
        description: &str,
        required: bool,
    ) -> Self {
        let props = self
            .parameters
            .as_object_mut()
            .and_then(|o| o.get_mut("properties"))
            .and_then(|p| p.as_object_mut());

        if let Some(props) = props {
            let schema = if param_type == "array" {
                serde_json::json!({
                    "type": "array",
                    "items": { "type": "string" },
                    "description": description
                })
            } else {
                serde_json::json!({
                    "type": param_type,
                    "description": description
                })
            };
            props.insert(name.to_string(), schema);
        }

        if required
            && let Some(req) = self
                .parameters
                .as_object_mut()
                .and_then(|o| o.get_mut("required"))
                .and_then(|r| r.as_array_mut())
        {
            req.push(serde_json::Value::String(name.to_string()));
        }

        self
    }

    /// Derive a human-readable category from the tool name.
    ///
    /// Used by the API and frontends to group tools consistently.
    pub fn category(&self) -> &'static str {
        tool_category(&self.name)
    }
}

/// Categorize a tool by its name prefix.
///
/// Shared logic used by TUI, API, and all frontends.
pub fn tool_category(name: &str) -> &'static str {
    match name {
        "read_file" | "write_file" | "edit_file" | "list_dir" => "Core",
        "shell" => "Core",
        "web_fetch" => "Core",
        "spawn" => "Core",
        "message" => "Core",
        _ if name.starts_with("browser_") => "Browser",
        _ if name.starts_with("git_") => "Git",
        _ if name.starts_with("safari_") => "Safari",
        _ if name.starts_with("music_") => "Music",
        _ if name.starts_with("mail_") || name.starts_with("email_") => "Mail",
        _ if name.starts_with("notes_") || name.starts_with("note_") => "Notes",
        _ if name.starts_with("calendar_") => "Calendar",
        _ if name.starts_with("reminders_") || name.starts_with("reminder_") => "Reminders",
        _ if name.starts_with("contacts_") || name.starts_with("contact_") => "Contacts",
        _ if name.starts_with("messages_") || name.starts_with("imessage_") => "iMessage",
        _ if name.starts_with("telegram_") => "Telegram",
        _ if name.starts_with("brew_") || name.starts_with("homebrew_") => "Homebrew",
        _ if name.starts_with("bluetooth_") || name.starts_with("bt_") => "Bluetooth",
        _ if name.starts_with("pdf_") => "PDF",
        _ if name.starts_with("ui_") => "UI Automation",
        _ if name.starts_with("file_") || name == "find_files" => "Files",
        _ if name.starts_with("network_") || name == "ping" || name == "port_check" => "Network",
        _ if name.starts_with("defaults_") || name.starts_with("config_") => "Defaults",
        _ if name.starts_with("voice_") || name == "speak_text" || name == "stt" => "Voice",
        _ if name.starts_with("system_")
            || name == "process_list"
            || name == "clipboard"
            || name == "screenshot"
            || name == "volume"
            || name == "wifi"
            || name == "focus"
            || name == "spotlight_search" =>
        {
            "System"
        }
        _ => "Other",
    }
}

// ============================================================================
// ============================================================================
// Workspace Templates
// ============================================================================

/// Pre-built workspace configurations for common use cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTemplate {
    /// Template identifier (e.g., "rust-project", "python-data-science")
    pub id: String,
    /// Human-readable template name
    pub name: String,
    /// Short description
    pub description: String,
    /// Category: "development", "data-science", "devops", "writing", "research"
    pub category: String,
    /// Files to create (path -> content)
    pub files: std::collections::HashMap<String, String>,
    /// Directories to create
    pub directories: Vec<String>,
    /// Default config overrides (merged into Config on apply)
    pub config_overrides: Option<serde_json::Value>,
    /// Skills to enable by default
    pub default_skills: Vec<String>,
    /// Tools to enable by default
    pub default_tools: Vec<String>,
}

impl WorkspaceTemplate {
    /// Get all built-in templates
    pub fn builtins() -> Vec<Self> {
        vec![
            Self {
                id: "rust-project".into(),
                name: "Rust Project".into(),
                description: "Rust workspace with cargo, clippy, and test setup".into(),
                category: "development".into(),
                files: [
                    ("memory/project.md".into(), "# Project\n\nRust workspace project.\n".into()),
                    ("memory/conventions.md".into(), "# Conventions\n\n- Use `cargo fmt` and `cargo clippy`\n- Write tests for all public APIs\n".into()),
                ].into(),
                directories: vec!["memory".into(), "scripts".into()],
                config_overrides: None,
                default_skills: vec!["code-review".into(), "git".into()],
                default_tools: vec!["read_file".into(), "write_file".into(), "edit_file".into(), "shell".into()],
            },
            Self {
                id: "python-data-science".into(),
                name: "Python Data Science".into(),
                description: "Python + Jupyter notebook environment".into(),
                category: "data-science".into(),
                files: [
                    ("memory/project.md".into(), "# Data Science Project\n\nPython + Jupyter notebooks.\n".into()),
                ].into(),
                directories: vec!["memory".into(), "notebooks".into(), "data".into()],
                config_overrides: None,
                default_skills: vec!["code-review".into(), "learn".into()],
                default_tools: vec!["read_file".into(), "write_file".into(), "shell".into(), "web_fetch".into()],
            },
            Self {
                id: "devops".into(),
                name: "DevOps / Infrastructure".into(),
                description: "Infrastructure as code, CI/CD, containers".into(),
                category: "devops".into(),
                files: [
                    ("memory/infra.md".into(), "# Infrastructure\n\nDevOps project.\n".into()),
                ].into(),
                directories: vec!["memory".into(), "terraform".into(), "ansible".into()],
                config_overrides: None,
                default_skills: vec!["devops-automator".into()],
                default_tools: vec!["read_file".into(), "write_file".into(), "shell".into(), "web_fetch".into()],
            },
            Self {
                id: "research".into(),
                name: "Research Assistant".into(),
                description: "Web research, note-taking, and summarization".into(),
                category: "research".into(),
                files: [
                    ("memory/research.md".into(), "# Research Notes\n\n".into()),
                ].into(),
                directories: vec!["memory".into(), "sources".into(), "notes".into()],
                config_overrides: None,
                default_skills: vec!["trend-researcher".into(), "learn".into()],
                default_tools: vec!["read_file".into(), "write_file".into(), "web_fetch".into(), "link_understanding".into()],
            },
            Self {
                id: "writing".into(),
                name: "Writing / Content".into(),
                description: "Long-form writing, editing, and publishing".into(),
                category: "writing".into(),
                files: [
                    ("memory/style.md".into(), "# Style Guide\n\n".into()),
                ].into(),
                directories: vec!["memory".into(), "drafts".into(), "published".into()],
                config_overrides: None,
                default_skills: vec!["content-creator".into(), "trend-researcher".into()],
                default_tools: vec!["read_file".into(), "write_file".into(), "edit_file".into()],
            },
        ]
    }

    /// Apply this template to a workspace directory
    pub fn apply(&self, workspace_root: &std::path::Path) -> Result<usize> {
        let mut created = 0;

        // Create directories
        for dir in &self.directories {
            let path = workspace_root.join(dir);
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .map_err(|e| Error::Config(format!("Failed to create {}: {}", dir, e)))?;
                created += 1;
            }
        }

        // Create files
        for (rel_path, content) in &self.files {
            let path = workspace_root.join(rel_path);
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&path, content)
                    .map_err(|e| Error::Config(format!("Failed to write {}: {}", rel_path, e)))?;
                created += 1;
            }
        }

        Ok(created)
    }

    /// Find a template by ID
    pub fn find(id: &str) -> Option<Self> {
        Self::builtins().into_iter().find(|t| t.id == id)
    }
}

// ============================================================================
// Agent Persona Templates
// ============================================================================

/// A reusable agent persona template loaded from a .md file.
///
/// Persona templates define specialized agent behaviors (e.g., code reviewer,
/// TDD guide, security auditor). They are stored as markdown files with YAML
/// frontmatter in `workspace/personas/` and can be used to spawn agents via
/// `POST /v1/agents/from-persona`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaTemplate {
    /// Template name / identifier (from frontmatter `name:`)
    pub name: String,
    /// Short description (from frontmatter `description:`)
    pub description: String,
    /// Recommended model string (from frontmatter `model:`)
    pub model: String,
    /// Tools the persona needs (from frontmatter `tools:`)
    pub tools: Vec<String>,
    /// Full persona instructions (markdown body after frontmatter)
    pub persona_text: String,
}

impl PersonaTemplate {
    /// Parse a persona template from markdown with YAML frontmatter.
    ///
    /// Expected format:
    /// ```text
    /// ---
    /// name: code-reviewer
    /// description: Expert code review specialist
    /// model: anthropic/claude-sonnet-4-20250514
    /// tools: ["read_file", "shell"]
    /// ---
    ///
    /// You are a senior code reviewer...
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> Result<Self> {
        let content = content.trim();
        if !content.starts_with("---") {
            return Err(Error::Config("Persona template must start with YAML frontmatter (---)".into()));
        }

        let after_first = &content[3..];
        let end = after_first.find("---").ok_or_else(|| {
            Error::Config("Persona template missing closing frontmatter (---)".into())
        })?;

        let frontmatter = &after_first[..end].trim();
        let body = after_first[end + 3..].trim().to_string();

        // Parse frontmatter fields manually (avoid pulling in yaml crate)
        let mut name = String::new();
        let mut description = String::new();
        let mut model = String::new();
        let mut tools: Vec<String> = Vec::new();

        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("model:") {
                model = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("tools:") {
                let val = val.trim();
                if val.starts_with('[') && val.ends_with(']') {
                    let inner = &val[1..val.len() - 1];
                    tools = inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }

        if name.is_empty() {
            return Err(Error::Config("Persona template missing 'name' in frontmatter".into()));
        }

        Ok(Self {
            name,
            description,
            model,
            tools,
            persona_text: body,
        })
    }

    /// Load a persona template from a file path.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read persona {}: {}", path.display(), e)))?;
        Self::from_str(&content)
    }

    /// Load all persona templates from a directory.
    pub fn load_all(dir: &std::path::Path) -> Vec<Self> {
        let mut templates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    match Self::from_file(&path) {
                        Ok(t) => templates.push(t),
                        Err(e) => {
                            eprintln!("[zeus-core] Failed to load persona {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }
        templates.sort_by(|a, b| a.name.cmp(&b.name));
        templates
    }

    /// Find a persona template by name from the default workspace location.
    pub fn find(name: &str, workspace: &std::path::Path) -> Option<Self> {
        let personas_dir = workspace.join("personas");
        Self::load_all(&personas_dir)
            .into_iter()
            .find(|t| t.name == name)
    }

    /// Get the built-in personas directory (relative to workspace).
    pub fn personas_dir(workspace: &std::path::Path) -> std::path::PathBuf {
        workspace.join("personas")
    }
}

// ============================================================================
// Per-Agent Auth Profiles
// ============================================================================

/// Authentication profile for a specific agent or delegation target.
/// Allows different agents to use different API keys/tokens with rate limiting.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Profile identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Provider (anthropic, openai, google, etc.)
    pub provider: String,
    /// API key or token
    #[serde(skip_serializing)]
    pub api_key: String,
    /// Model to use with this profile
    pub model: Option<String>,
    /// Maximum requests per minute (rate limiting)
    #[serde(default = "default_rpm")]
    pub max_rpm: u32,
    /// Maximum tokens per day (budget control)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_day: Option<u64>,
    /// Current token usage today (runtime, not persisted)
    #[serde(skip)]
    pub tokens_used_today: std::sync::atomic::AtomicU64,
    /// Whether this profile is enabled
    #[serde(default = "default_true_val")]
    pub enabled: bool,
    /// Priority (lower = preferred, for auto-selection)
    #[serde(default)]
    pub priority: u32,
    /// Cooldown period in seconds after rate limit hit
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    /// Timestamp of last rate limit hit (runtime)
    #[serde(skip)]
    pub last_rate_limit: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
}

fn default_rpm() -> u32 {
    60
}
fn default_cooldown() -> u64 {
    30
}
fn default_true_val() -> bool {
    true
}

impl AuthProfile {
    /// Check if this profile is available (enabled, not rate-limited, within budget)
    pub fn is_available(&self) -> bool {
        if !self.enabled {
            return false;
        }

        // Check cooldown
        if let Ok(guard) = self.last_rate_limit.lock()
            && let Some(last_hit) = *guard
        {
            let elapsed = chrono::Utc::now().signed_duration_since(last_hit);
            if elapsed.num_seconds() < self.cooldown_secs as i64 {
                return false;
            }
        }

        // Check daily budget
        if let Some(max_tokens) = self.max_tokens_per_day {
            let used = self
                .tokens_used_today
                .load(std::sync::atomic::Ordering::Relaxed);
            if used >= max_tokens {
                return false;
            }
        }

        true
    }

    /// Record token usage
    pub fn record_usage(&self, tokens: u64) {
        self.tokens_used_today
            .fetch_add(tokens, std::sync::atomic::Ordering::Relaxed);
    }

    /// Mark as rate-limited
    pub fn mark_rate_limited(&self) {
        if let Ok(mut guard) = self.last_rate_limit.lock() {
            *guard = Some(chrono::Utc::now());
        }
    }
}

/// Manager for multiple auth profiles with auto-rotation
#[derive(Debug, Default)]
pub struct AuthProfileManager {
    profiles: Vec<AuthProfile>,
}

impl AuthProfileManager {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
        }
    }

    pub fn add(&mut self, profile: AuthProfile) {
        self.profiles.push(profile);
        self.profiles.sort_by_key(|p| p.priority);
    }

    /// Get the best available profile for a given provider
    pub fn get_available(&self, provider: &str) -> Option<&AuthProfile> {
        self.profiles
            .iter()
            .find(|p| p.provider == provider && p.is_available())
    }

    /// Get all profiles
    pub fn all(&self) -> &[AuthProfile] {
        &self.profiles
    }

    /// Get profile by ID
    pub fn get(&self, id: &str) -> Option<&AuthProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Remove a profile by ID
    pub fn remove(&mut self, id: &str) -> bool {
        let len_before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        self.profiles.len() < len_before
    }
}

// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_p0_sparse_onboarding_config_loads() {
        // Exactly what the new onboarding writes: model + [credentials] only.
        let sparse = r#"
model = "glm/glm-5.2"

[credentials]
ZAI_API_KEY = "test-key-123"
"#;
        let c: Config = toml::from_str(sparse).expect("sparse config must deserialize via serde defaults");
        assert_eq!(c.model, "glm/glm-5.2");
        assert_eq!(c.credentials.get("ZAI_API_KEY").map(String::as_str), Some("test-key-123"));
        // Operational sections are all Option/defaulted — sparse config leaves them None/default.
        assert!(c.session_compaction.is_none(), "sparse config leaves session_compaction defaulted");
        assert!(c.mnemosyne.is_none(), "sparse config leaves mnemosyne defaulted");
    }

    // ---- #176 H1: task-derived cooking budget extraction ----
    const H: u64 = 3600;
    fn ceiling_4h() -> std::time::Duration {
        std::time::Duration::from_secs(4 * H)
    }

    #[test]
    fn test_extract_task_timeout_explicit_hours() {
        let d = extract_task_timeout("work on this for 3h please", ceiling_4h());
        assert_eq!(d, Some(std::time::Duration::from_secs(3 * H)));
    }

    #[test]
    fn test_extract_task_timeout_for_the_next_pattern() {
        let d = extract_task_timeout("keep cooking for the next 2 hours", ceiling_4h());
        assert_eq!(d, Some(std::time::Duration::from_secs(2 * H)));
    }

    #[test]
    fn test_extract_task_timeout_minutes() {
        let d = extract_task_timeout("run this for 45 minutes", ceiling_4h());
        assert_eq!(d, Some(std::time::Duration::from_secs(45 * 60)));
    }

    #[test]
    fn test_extract_task_timeout_no_explicit_intent_returns_none() {
        // bare duration mention in body must NOT trigger (false-positive guard)
        assert_eq!(extract_task_timeout("fix the 2h timeout bug", ceiling_4h()), None);
        assert_eq!(extract_task_timeout("the 30 hour cook died", ceiling_4h()), None);
        assert_eq!(extract_task_timeout("no duration here at all", ceiling_4h()), None);
    }

    #[test]
    fn test_extract_task_timeout_ceiling_caps_misparse() {
        // explicit 30h but ceiling 4h → capped at 4h, never exceeds bound
        let d = extract_task_timeout("work on this for 30h", ceiling_4h());
        assert_eq!(d, Some(ceiling_4h()));
    }

    #[test]
    fn test_resolve_cooking_loop_max_default_and_override() {
        let mut cfg = PrometheusConfig::default();
        assert_eq!(resolve_cooking_loop_max(&cfg), std::time::Duration::from_secs(24 * H));
        cfg.cooking_loop_max = Some("6h".to_string());
        assert_eq!(resolve_cooking_loop_max(&cfg), std::time::Duration::from_secs(6 * H));
        cfg.cooking_loop_max = Some("garbage".to_string());
        assert_eq!(resolve_cooking_loop_max(&cfg), std::time::Duration::from_secs(24 * H));
    }

    // ---- #176-b H2/#284B: progress-recency resolvers + idle watchdog ----
    #[test]
    fn test_resolve_cooking_recency_window_default_and_override() {
        let mut cfg = PrometheusConfig::default();
        assert_eq!(resolve_cooking_recency_window(&cfg), std::time::Duration::from_secs(120));
        cfg.cooking_recency_window = Some("90s".to_string());
        assert_eq!(resolve_cooking_recency_window(&cfg), std::time::Duration::from_secs(90));
        cfg.cooking_recency_window = Some("garbage".to_string());
        assert_eq!(resolve_cooking_recency_window(&cfg), std::time::Duration::from_secs(120));
    }

    #[test]
    fn test_resolve_cooking_extension_quantum_default_and_override() {
        let mut cfg = PrometheusConfig::default();
        assert_eq!(resolve_cooking_extension_quantum(&cfg), std::time::Duration::from_secs(600));
        cfg.cooking_extension_quantum = Some("15m".to_string());
        assert_eq!(resolve_cooking_extension_quantum(&cfg), std::time::Duration::from_secs(15 * 60));
        cfg.cooking_extension_quantum = Some("garbage".to_string());
        assert_eq!(resolve_cooking_extension_quantum(&cfg), std::time::Duration::from_secs(600));
    }

    #[test]
    fn test_should_abort_for_idle_no_activity_past_window_kills() {
        assert!(should_abort_for_idle(1_000, 4_601, std::time::Duration::from_secs(3_600)));
    }

    #[test]
    fn test_should_abort_for_idle_recent_tool_activity_continues() {
        assert!(!should_abort_for_idle(4_500, 5_000, std::time::Duration::from_secs(3_600)));
    }

    #[test]
    fn test_should_abort_for_idle_recent_text_activity_continues() {
        assert!(!should_abort_for_idle(7_200, 7_240, std::time::Duration::from_secs(3_600)));
    }

    #[test]
    fn test_should_abort_for_idle_active_cook_can_exceed_old_static_cap() {
        assert!(!should_abort_for_idle(9_500, 10_000, std::time::Duration::from_secs(3_600)));
    }

    #[test]
    fn test_should_abort_for_idle_boundary_is_not_past_window() {
        assert!(!should_abort_for_idle(1_000, 4_600, std::time::Duration::from_secs(3_600)));
    }

    #[test]
    fn test_config_defaults() {
        let config = Config::default();
        assert!(config.model.is_empty()); // No default — user must select during onboarding
        assert_eq!(config.max_iterations, 20);
        // Advanced configs default to None (opt-in)
        assert!(config.mnemosyne.is_none());
        assert!(config.athena.is_none());
        assert!(config.aegis.is_none());
        assert!(config.search.is_some()); // #149: search enabled by default
        assert!(config.gateway.is_none());
        assert!(config.session_compaction.is_some()); // S57: enabled by default
        assert_eq!(config.thinking_level.as_deref(), Some("high")); // #149: high by default
    }

    #[test]
    fn test_verbosity_default_is_normal() {
        let config = Config::default();
        assert_eq!(config.verbosity, Verbosity::Normal);
    }

    #[test]
    fn test_verbosity_serde_roundtrip() {
        let toml_str = r#"
model = "ollama/llama3.2"
verbosity = "silent"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.verbosity, Verbosity::Silent);

        let toml_str = r#"
model = "ollama/llama3.2"
verbosity = "barfly"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.verbosity, Verbosity::Barfly);
    }

    #[test]
    fn test_verbosity_system_prompt() {
        assert!(Verbosity::Silent.system_prompt_instructions().contains("SILENT"));
        assert!(Verbosity::Normal.system_prompt_instructions().contains("NORMAL"));
        assert!(Verbosity::Barfly.system_prompt_instructions().contains("BARFLY"));
    }

    #[test]
    fn test_sender_type_default_is_unknown() {
        assert_eq!(SenderType::default(), SenderType::Unknown);
    }

    #[test]
    fn test_sender_type_is_bot() {
        assert!(SenderType::Bot.is_bot());
        assert!(!SenderType::Human.is_bot());
        assert!(!SenderType::System.is_bot());
        assert!(!SenderType::Unknown.is_bot());
    }

    #[test]
    fn test_sender_type_is_human() {
        assert!(SenderType::Human.is_human());
        assert!(!SenderType::Bot.is_human());
        assert!(!SenderType::System.is_human());
        assert!(!SenderType::Unknown.is_human());
    }

    #[test]
    fn test_sender_type_serde_roundtrip() {
        for variant in [SenderType::Human, SenderType::Bot, SenderType::System, SenderType::Unknown] {
            let json = serde_json::to_string(&variant).unwrap();
            let parsed: SenderType = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, parsed);
        }
    }

    #[test]
    fn test_sender_type_display() {
        assert_eq!(SenderType::Human.to_string(), "human");
        assert_eq!(SenderType::Bot.to_string(), "bot");
        assert_eq!(SenderType::System.to_string(), "system");
        assert_eq!(SenderType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_parse_model() {
        let config = Config {
            model: "openai/gpt-4o".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::OpenAI);
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn test_message_creation() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello");
        assert!(msg.attachments.is_empty());
    }

    #[test]
    fn test_tool_schema() {
        let schema = ToolSchema::new("read_file", "Read a file").with_param(
            "path",
            "string",
            "File path",
            true,
        );

        assert_eq!(schema.name, "read_file");
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
    }

    #[test]
    fn test_search_config_defaults() {
        let cfg = SearchConfig::default();
        assert_eq!(cfg.provider, "duckduckgo");
        assert!(cfg.api_key.is_none());
        assert_eq!(cfg.max_results, 5);
    }

    #[test]
    fn test_gateway_config_defaults() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.public_url, "");
        assert!(cfg.enable_channels);
        assert!(cfg.enable_api);
    }

    #[test]
    fn test_mnemosyne_config_hybrid_weights() {
        let cfg = MnemosyneConfig::default();
        assert!((cfg.vector_weight - 0.7).abs() < f64::EPSILON);
        assert!((cfg.text_weight - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_multiplier, 4);
    }

    #[test]
    fn test_session_compaction_config_defaults() {
        let cfg = SessionCompactionConfig::default();
        assert_eq!(cfg.max_context_tokens, 180000);
        assert!((cfg.compaction_threshold - 0.8).abs() < f32::EPSILON);
        assert!(cfg.summary_model.is_none());
    }

    #[test]
    fn test_attachment_serialization_roundtrip() {
        let attachment = Attachment {
            mime_type: "image/jpeg".to_string(),
            data: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
            filename: Some("photo.jpg".to_string()),
            source_url: None,
        };
        let json = serde_json::to_string(&attachment).expect("should serialize to JSON");
        let deserialized: Attachment =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.mime_type, "image/jpeg");
        assert_eq!(deserialized.data, vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
        assert_eq!(deserialized.filename, Some("photo.jpg".to_string()));
    }

    #[test]
    fn test_message_with_attachments() {
        let att = Attachment {
            mime_type: "image/png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47],
            filename: None,
            source_url: None,
        };
        let msg = Message::user_with_attachments("Look at this", vec![att]);
        assert_eq!(msg.content, "Look at this");
        assert_eq!(msg.attachments.len(), 1);
        assert_eq!(msg.attachments[0].mime_type, "image/png");
    }

    #[test]
    fn test_dm_policy_serde() {
        let policy = DmPolicy::Pairing;
        let json = serde_json::to_string(&policy).expect("should serialize to JSON");
        assert_eq!(json, "\"pairing\"");
        let deserialized: DmPolicy =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized, DmPolicy::Pairing);
    }

    #[test]
    fn test_group_policy_serde() {
        let policy = GroupPolicy::MentionOnly;
        let json = serde_json::to_string(&policy).expect("should serialize to JSON");
        assert_eq!(json, "\"mentiononly\"");
        let deserialized: GroupPolicy =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized, GroupPolicy::MentionOnly);
    }

    #[test]
    fn test_channels_config_with_new_channels() {
        let cfg = ChannelsConfig {
            whatsapp: Some(WhatsAppChannelConfig {
                bridge_url: "ws://localhost:3001".to_string(),
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            signal: Some(SignalChannelConfig {
                signal_cli_path: "/usr/local/bin/signal-cli".to_string(),
                phone: "+1234567890".to_string(),
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            matrix: Some(MatrixChannelConfig {
                homeserver: "https://matrix.org".to_string(),
                access_token: "test_token".to_string(),
                username: None,
                password: None,
                user_id: None,
                rooms: vec![],
                display_name: None,
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            ..Default::default()
        };
        assert!(cfg.whatsapp.is_some());
        assert!(cfg.signal.is_some());
        assert!(cfg.matrix.is_some());
    }

    #[test]
    fn test_parse_model_mistral() {
        let config = Config {
            model: "mistral/mistral-large-latest".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Mistral);
        assert_eq!(model, "mistral-large-latest");
    }

    #[test]
    fn test_mistral_env_key() {
        assert_eq!(Provider::Mistral.env_key(), "MISTRAL_API_KEY");
    }

    #[test]
    fn test_parse_model_together() {
        let config = Config {
            model: "together/meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Together);
        assert_eq!(model, "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo");
    }

    #[test]
    fn test_parse_model_fireworks() {
        let config = Config {
            model: "fireworks/accounts/fireworks/models/llama-v3p1-405b-instruct".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Fireworks);
        assert_eq!(model, "accounts/fireworks/models/llama-v3p1-405b-instruct");
    }

    #[test]
    fn test_together_env_key() {
        assert_eq!(Provider::Together.env_key(), "TOGETHER_API_KEY");
    }

    #[test]
    fn test_minimax_env_key() {
        assert_eq!(Provider::Minimax.env_key(), "MINIMAX_API_KEY");
    }

    #[test]
    fn test_fireworks_env_key() {
        assert_eq!(Provider::Fireworks.env_key(), "FIREWORKS_API_KEY");
    }

    #[test]
    fn test_parse_model_azure() {
        let config = Config {
            model: "azure/gpt-4o".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Azure);
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn test_azure_env_key() {
        assert_eq!(Provider::Azure.env_key(), "AZURE_OPENAI_API_KEY");
    }

    #[test]
    fn test_parse_model_bedrock() {
        let config = Config {
            model: "bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Bedrock);
        assert_eq!(model, "anthropic.claude-3-5-sonnet-20241022-v2:0");
    }

    #[test]
    fn test_parse_model_bedrock_aws_prefix() {
        let config = Config {
            model: "aws/amazon.titan-text-premier-v1:0".to_string(),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Bedrock);
        assert_eq!(model, "amazon.titan-text-premier-v1:0");
    }

    #[test]
    fn test_from_prefix_canonicalizes_display_aliases() {
        // #185 P0: onboarding must canonicalize the TUI display id → canonical
        // Provider id so config.model is byte-identical to the proven-working
        // spark config and resolves at the gateway read-side. Generic for ALL
        // providers — display alias and canonical id both map to the same enum,
        // and .name() emits the canonical prefix.
        for (display, canonical, prov) in [
            ("glm", "zai", Provider::Zai),
            ("zai", "zai", Provider::Zai),
            ("kimi", "moonshot", Provider::Moonshot),
            ("moonshot", "moonshot", Provider::Moonshot),
            ("grok", "xai", Provider::XAI),
            ("mimo", "xiaomimimo", Provider::XiaomiMimo),
            ("anthropic", "anthropic", Provider::Anthropic),
        ] {
            let p = Provider::from_prefix(display);
            assert_eq!(p, prov, "display '{display}' must map to {prov:?}");
            assert_eq!(
                p.name(),
                canonical,
                "display '{display}' must canonicalize to '{canonical}'"
            );
        }
    }

    #[test]
    fn test_from_prefix_matches_parse_model() {
        // Single-source-of-truth guard: parse_model now delegates to from_prefix,
        // so the canonicalized prefix must round-trip to the same Provider the
        // gateway resolves at read-time.
        let canonical = Provider::from_prefix("glm").name(); // "zai"
        let config = Config {
            model: format!("{canonical}/glm-5.2"),
            ..Default::default()
        };
        let (provider, model) = config.parse_model();
        assert_eq!(provider, Provider::Zai);
        assert_eq!(model, "glm-5.2");
    }

    #[test]
    fn test_bedrock_env_key() {
        assert_eq!(Provider::Bedrock.env_key(), "AWS_ACCESS_KEY_ID");
    }

    // ========================================================================
    // BindingRule tests
    // ========================================================================

    #[test]
    fn test_binding_rule_peer_match() {
        let rule = BindingRule::Peer("user123".to_string());
        assert!(rule.matches("telegram", "user123", "chat456"));
        assert!(!rule.matches("telegram", "other_user", "chat456"));
    }

    #[test]
    fn test_binding_rule_guild_match() {
        let rule = BindingRule::Guild("chat456".to_string());
        assert!(rule.matches("discord", "any_user", "chat456"));
        assert!(!rule.matches("discord", "any_user", "other_chat"));
    }

    #[test]
    fn test_binding_rule_team_match() {
        let rule = BindingRule::Team("telegram".to_string());
        assert!(rule.matches("telegram", "any", "any"));
        assert!(!rule.matches("discord", "any", "any"));
    }

    #[test]
    fn test_binding_rule_account_never_matches_channel() {
        let rule = BindingRule::Account("agent-1".to_string());
        assert!(!rule.matches("telegram", "agent-1", "agent-1"));
    }

    #[test]
    fn test_binding_rule_serde_roundtrip() {
        let rules = vec![
            BindingRule::Peer("u1".to_string()),
            BindingRule::Guild("g1".to_string()),
            BindingRule::Team("slack".to_string()),
            BindingRule::Account("a1".to_string()),
        ];
        let json = serde_json::to_string(&rules).expect("should serialize to JSON");
        let deserialized: Vec<BindingRule> =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(rules, deserialized);
    }

    // ========================================================================
    // AgentToolPolicy tests
    // ========================================================================

    #[test]
    fn test_tool_policy_empty_allows_all() {
        let policy = AgentToolPolicy::default();
        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("shell"));
        assert!(policy.is_tool_allowed("anything"));
    }

    #[test]
    fn test_tool_policy_allowlist_filters() {
        let policy = AgentToolPolicy {
            allowed_tools: vec!["read_file".to_string(), "list_dir".to_string()],
            denied_tools: vec![],
        };
        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("list_dir"));
        assert!(!policy.is_tool_allowed("shell"));
        assert!(!policy.is_tool_allowed("write_file"));
    }

    #[test]
    fn test_tool_policy_denylist_precedence() {
        let policy = AgentToolPolicy {
            allowed_tools: vec!["*".to_string()],
            denied_tools: vec!["shell".to_string()],
        };
        assert!(policy.is_tool_allowed("read_file"));
        assert!(!policy.is_tool_allowed("shell"));
    }

    #[test]
    fn test_tool_policy_wildcard_patterns() {
        let policy = AgentToolPolicy {
            allowed_tools: vec!["read_*".to_string(), "list_*".to_string()],
            denied_tools: vec![],
        };
        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("read_dir"));
        assert!(policy.is_tool_allowed("list_dir"));
        assert!(!policy.is_tool_allowed("write_file"));
        assert!(!policy.is_tool_allowed("shell"));
    }

    #[test]
    fn test_tool_policy_namespace_wildcard() {
        let policy = AgentToolPolicy {
            allowed_tools: vec!["talos.*".to_string()],
            denied_tools: vec!["talos.shell*".to_string()],
        };
        assert!(policy.is_tool_allowed("talos.calendar_list"));
        assert!(policy.is_tool_allowed("talos.notes_read"));
        assert!(!policy.is_tool_allowed("talos.shell_exec"));
        assert!(!policy.is_tool_allowed("read_file"));
    }

    #[test]
    fn test_tool_policy_serde_roundtrip() {
        let policy = AgentToolPolicy {
            allowed_tools: vec!["read_*".to_string()],
            denied_tools: vec!["shell".to_string()],
        };
        let json = serde_json::to_string(&policy).expect("should serialize to JSON");
        let deserialized: AgentToolPolicy =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(policy, deserialized);
    }

    // ========================================================================
    // AgentBinding tests
    // ========================================================================

    #[test]
    fn test_agent_binding_serde_roundtrip() {
        let binding = AgentBinding {
            agent_id: "agent-1".to_string(),
            bindings: vec![
                BindingRule::Peer("user1".to_string()),
                BindingRule::Team("telegram".to_string()),
            ],
            tool_policy: AgentToolPolicy {
                allowed_tools: vec!["read_*".to_string()],
                denied_tools: vec!["shell".to_string()],
            },
            priority: 10,
        };
        let json = serde_json::to_string(&binding).expect("should serialize to JSON");
        let deserialized: AgentBinding =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.agent_id, "agent-1");
        assert_eq!(deserialized.bindings.len(), 2);
        assert_eq!(deserialized.priority, 10);
        assert!(!deserialized.tool_policy.is_tool_allowed("shell"));
    }

    #[test]
    fn test_agent_config_resolve_mnemosyne_db_default() {
        let cfg = AgentConfig {
            id: "z".to_string(),
            ..Default::default()
        };
        let base = std::path::PathBuf::from("/home/user/.zeus/workspace");
        let db = cfg.resolve_mnemosyne_db(&base);
        assert_eq!(db, std::path::PathBuf::from("/home/user/.zeus/workspace/agents/z/mnemosyne.db"));
    }

    #[test]
    fn test_agent_config_resolve_mnemosyne_db_explicit() {
        let cfg = AgentConfig {
            id: "z".to_string(),
            mnemosyne_db: Some(std::path::PathBuf::from("/custom/z.db")),
            ..Default::default()
        };
        let base = std::path::PathBuf::from("/home/user/.zeus/workspace");
        let db = cfg.resolve_mnemosyne_db(&base);
        assert_eq!(db, std::path::PathBuf::from("/custom/z.db"));
    }

    #[test]
    fn test_mcp_server_config_defaults() {
        let cfg = McpServerConfig::default();
        assert!(cfg.allowed_origins.is_empty());
        assert_eq!(cfg.max_connections, 32);
        assert!(cfg.enable_talos);
        assert!(!cfg.enable_agents);
        assert!(cfg.auth_token.is_none());
        assert!(cfg.enable_mnemosyne);
    }

    #[test]
    fn test_mcp_server_config_serde() {
        let cfg = McpServerConfig {
            allowed_origins: vec!["http://localhost:3000".to_string()],
            max_connections: 16,
            enable_talos: false,
            enable_agents: true,
            auth_token: Some("secret".to_string()),
            enable_mnemosyne: false,
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: McpServerConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.max_connections, 16);
        assert!(!parsed.enable_talos);
        assert!(parsed.enable_agents);
        assert_eq!(parsed.auth_token.as_deref(), Some("secret"));
    }

    #[test]
    fn test_config_with_mcp_server() {
        let mut config = Config::default();
        assert!(config.mcp_server.is_none());
        config.mcp_server = Some(McpServerConfig::default());
        assert!(config.mcp_server.is_some());
    }

    #[test]
    fn test_detect_environment() {
        let detected = Config::detect_environment();
        // Should return a vec (may be empty in test env)
        assert!(detected.len() <= 10);
    }

    #[test]
    fn test_suggest_provider() {
        // Just verify it doesn't panic
        let _ = Config::suggest_provider();
    }

    #[test]
    fn test_export_import_toml_roundtrip() {
        let config = Config::default();
        let toml_str = config.export_toml().unwrap();
        let imported = Config::import_toml(&toml_str).unwrap();
        assert_eq!(imported.model, config.model);
        assert_eq!(imported.max_iterations, config.max_iterations);
        assert!(imported.mcp_server.is_none());
    }

    #[test]
    fn test_export_import_file_roundtrip() {
        let config = Config {
            model: "openai/gpt-4o".to_string(),
            max_iterations: 42,
            ..Default::default()
        };
        let tmp = std::env::temp_dir().join("zeus_test_config_export.toml");
        config.export_to_file(&tmp).unwrap();

        let imported = Config::import_from_file(&tmp).unwrap();
        assert_eq!(imported.model, "openai/gpt-4o");
        assert_eq!(imported.max_iterations, 42);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_channels_empty() {
        let config = Config::default();
        let warnings = config.validate_channels();
        assert!(warnings.is_empty()); // no channels configured
    }

    #[test]
    fn test_validate_channels_telegram_bad() {
        let mut config = Config::default();
        config.channels = Some(ChannelsConfig {
            telegram: Some(TelegramChannelConfig {
                api_id: 0,
                api_hash: String::new(),
                phone: String::new(),
                bot_token: None,
                session_file: None,
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            discord: None,
            slack: None,
            email: None,
            whatsapp: None,
            signal: None,
            matrix: None,
            mqtt: None,
            mattermost: None,
            irc: None,
            twitch: None,
            x_twitter: None,
            instagram: None,
            tiktok: None,
            teams: None,
            webchat: None,
            googlechat: None,
            nextcloud: None,
            nostr: None,
            line: None,
            feishu: None,
            zalo: None,
            bluebubbles: None,
            sms: None,
            twilio_whatsapp: None,
            voice: None,
        });
        let warnings = config.validate_channels();
        assert!(warnings.iter().any(|w| w.contains("api_id")));
        assert!(warnings.iter().any(|w| w.contains("api_hash")));
        assert!(warnings.iter().any(|w| w.contains("phone")));
    }

    #[test]
    fn test_validate_channels_matrix_bad() {
        let mut config = Config::default();
        config.channels = Some(ChannelsConfig {
            telegram: None,
            discord: None,
            slack: None,
            email: None,
            whatsapp: None,
            signal: None,
            matrix: Some(MatrixChannelConfig {
                homeserver: String::new(),
                access_token: String::new(),
                username: None,
                password: None,
                user_id: None,
                rooms: vec![],
                display_name: None,
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            mqtt: None,
            mattermost: None,
            irc: None,
            twitch: None,
            x_twitter: None,
            instagram: None,
            tiktok: None,
            teams: None,
            webchat: None,
            googlechat: None,
            nextcloud: None,
            nostr: None,
            line: None,
            feishu: None,
            zalo: None,
            bluebubbles: None,
            sms: None,
            twilio_whatsapp: None,
            voice: None,
        });
        let warnings = config.validate_channels();
        assert!(warnings.iter().any(|w| w.contains("homeserver")));
        assert!(warnings.iter().any(|w| w.contains("access_token")));
    }

    #[test]
    fn test_validate_includes_channel_checks() {
        let mut config = Config::default();
        config.channels = Some(ChannelsConfig {
            telegram: None,
            discord: None,
            slack: None,
            email: None,
            whatsapp: Some(WhatsAppChannelConfig {
                bridge_url: String::new(),
                policy: None,
                accounts: HashMap::new(),
                allow_bots: None,
            }),
            signal: None,
            matrix: None,
            mqtt: None,
            mattermost: None,
            irc: None,
            twitch: None,
            x_twitter: None,
            instagram: None,
            tiktok: None,
            teams: None,
            webchat: None,
            googlechat: None,
            nextcloud: None,
            nostr: None,
            line: None,
            feishu: None,
            zalo: None,
            bluebubbles: None,
            sms: None,
            twilio_whatsapp: None,
            voice: None,
        });
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("WhatsApp")));
    }

    // ── PersonaTemplate Tests ─────────────────────────────────────

    #[test]
    fn test_persona_template_parse_valid() {
        let content = r#"---
name: code-reviewer
description: Expert code review specialist
model: anthropic/claude-sonnet-4-20250514
tools: ["read_file", "shell", "list_dir"]
---

You are a senior code reviewer ensuring high standards."#;

        let template = PersonaTemplate::from_str(content).unwrap();
        assert_eq!(template.name, "code-reviewer");
        assert_eq!(template.description, "Expert code review specialist");
        assert_eq!(template.model, "anthropic/claude-sonnet-4-20250514");
        assert_eq!(template.tools, vec!["read_file", "shell", "list_dir"]);
        assert!(template.persona_text.contains("senior code reviewer"));
    }

    #[test]
    fn test_persona_template_missing_frontmatter() {
        let content = "You are a code reviewer.";
        let result = PersonaTemplate::from_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_persona_template_missing_name() {
        let content = r#"---
description: No name field
model: ollama/llama3.2
tools: []
---

Body text."#;

        let result = PersonaTemplate::from_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_persona_template_empty_tools() {
        let content = r#"---
name: minimal
description: Minimal persona
model: ollama/llama3.2
tools: []
---

Minimal body."#;

        let template = PersonaTemplate::from_str(content).unwrap();
        assert_eq!(template.name, "minimal");
        assert!(template.tools.is_empty());
    }

    #[test]
    fn test_persona_template_load_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test-persona.md");
        std::fs::write(
            &path,
            r#"---
name: test-persona
description: Test persona for unit tests
model: ollama/llama3.2
tools: ["shell"]
---

You are a test persona."#,
        )
        .unwrap();

        let template = PersonaTemplate::from_file(&path).unwrap();
        assert_eq!(template.name, "test-persona");
        assert_eq!(template.tools, vec!["shell"]);
    }

    #[test]
    fn test_persona_template_load_all() {
        let tmp = tempfile::tempdir().unwrap();

        // Write 2 valid personas
        std::fs::write(
            tmp.path().join("alpha.md"),
            "---\nname: alpha\ndescription: Alpha\nmodel: m\ntools: []\n---\nAlpha body.",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("beta.md"),
            "---\nname: beta\ndescription: Beta\nmodel: m\ntools: []\n---\nBeta body.",
        )
        .unwrap();
        // Write 1 invalid file (no frontmatter)
        std::fs::write(tmp.path().join("bad.md"), "No frontmatter here.").unwrap();
        // Write 1 non-md file (should be ignored)
        std::fs::write(tmp.path().join("readme.txt"), "Not a persona.").unwrap();

        let templates = PersonaTemplate::load_all(tmp.path());
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].name, "alpha"); // sorted by name
        assert_eq!(templates[1].name, "beta");
    }

    #[test]
    fn test_persona_template_find() {
        let tmp = tempfile::tempdir().unwrap();
        let personas_dir = tmp.path().join("personas");
        std::fs::create_dir_all(&personas_dir).unwrap();
        std::fs::write(
            personas_dir.join("reviewer.md"),
            "---\nname: reviewer\ndescription: R\nmodel: m\ntools: []\n---\nReviewer body.",
        )
        .unwrap();

        let found = PersonaTemplate::find("reviewer", tmp.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "reviewer");

        let not_found = PersonaTemplate::find("nonexistent", tmp.path());
        assert!(not_found.is_none());
    }
}

#[cfg(test)]
mod sprint_tests {
    use super::*;

    #[test]
    fn test_received_file_path_layout_and_platform() {
        // Root-independent: assert the path structure, not the absolute prefix.
        let p = received_file_path("discord", "cat.png").expect("path");
        let s = p.to_string_lossy();
        assert!(
            s.contains("/workspace/channels/discord/received/"),
            "{}",
            s
        );
        assert!(s.ends_with("/cat.png"), "{}", s);
    }

    #[test]
    fn test_received_file_path_rejects_traversal_and_empty_platform() {
        // Hostile filename can't escape received/<date>/.
        let p = received_file_path("telegram", "../../etc/passwd").expect("path");
        let s = p.to_string_lossy();
        assert!(!s.contains(".."), "traversal leaked: {}", s);
        assert!(s.ends_with("/passwd"), "{}", s);
        // Empty platform → unknown bucket, never a bare segment.
        let q = received_file_path("", "f.bin").expect("path");
        assert!(
            q.to_string_lossy().contains("/channels/unknown/received/"),
            "{}",
            q.to_string_lossy()
        );
    }

    #[test]
    fn test_error_config() {
        let e = Error::config("bad");
        assert!(e.to_string().contains("bad"));
    }
    #[test]
    fn test_error_llm() {
        let e = Error::llm("fail");
        assert!(e.to_string().contains("fail"));
    }
    #[test]
    fn test_error_tool() {
        let e = Error::tool("x");
        assert!(e.to_string().contains("x"));
    }
    #[test]
    fn test_error_channel() {
        let e = Error::channel("c");
        assert!(e.to_string().contains("c"));
    }
    #[test]
    fn test_error_skill() {
        let e = Error::skill("s");
        assert!(e.to_string().contains("s"));
    }
    #[test]
    fn test_error_memory() {
        let e = Error::memory("m");
        assert!(e.to_string().contains("m"));
    }
    #[test]
    fn test_error_security() {
        let e = Error::security("sec");
        assert!(e.to_string().contains("sec"));
    }
    #[test]
    fn test_error_not_found() {
        let e = Error::not_found("x");
        assert!(e.to_string().contains("x"));
    }
    #[test]
    fn test_error_database() {
        let e = Error::database("db");
        assert!(e.to_string().contains("db"));
    }
    #[test]
    fn test_error_retryable_network() {
        assert!(Error::Network("n".into()).is_retryable());
    }
    #[test]
    fn test_error_retryable_timeout() {
        assert!(Error::Timeout("t".into()).is_retryable());
    }
    #[test]
    fn test_error_retryable_ratelimit() {
        assert!(Error::RateLimited("r".into()).is_retryable());
    }
    #[test]
    fn test_error_not_retryable_config() {
        assert!(!Error::Config("c".into()).is_retryable());
    }
    #[test]
    fn test_error_not_retryable_tool() {
        assert!(!Error::Tool("t".into()).is_retryable());
    }
    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }
    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }
    #[test]
    fn test_truncate_str_cut() {
        assert_eq!(truncate_str("hello world", 5), "hello");
    }
    #[test]
    fn test_truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
    }
    #[test]
    fn test_truncate_str_unicode() {
        let s = "héllo";
        let t = truncate_str(s, 3);
        assert!(t.len() <= 3);
    }
    #[test]
    fn test_message_user() {
        let m = Message::user("hi");
        assert_eq!(m.role, Role::User);
        assert_eq!(m.content, "hi");
    }
    #[test]
    fn test_message_assistant() {
        let m = Message::assistant("ok");
        assert_eq!(m.role, Role::Assistant);
    }
    #[test]
    fn test_message_system() {
        let m = Message::system("sys");
        assert_eq!(m.role, Role::System);
    }
    #[test]
    fn test_message_tool() {
        let m = Message::tool("id1", true, "out");
        assert_eq!(m.role, Role::Tool);
        assert_eq!(m.tool_results.len(), 1);
    }
    #[test]
    fn test_message_tool_success() {
        let m = Message::tool("c1", true, "ok");
        assert!(m.tool_results[0].success);
    }
    #[test]
    fn test_message_tool_failure() {
        let m = Message::tool("c1", false, "err");
        assert!(!m.tool_results[0].success);
    }
    #[test]
    fn test_message_with_tool_calls() {
        let m = Message::assistant("").with_tool_calls(vec![ToolCall {
            id: "1".into(),
            name: "test".into(),
            arguments: serde_json::json!({}),
        }]);
        assert_eq!(m.tool_calls.len(), 1);
    }
    #[test]
    fn test_message_attachments_default_empty() {
        let m = Message::user("hi");
        assert!(m.attachments.is_empty());
    }
    #[test]
    fn test_message_serialization() {
        let m = Message::user("test");
        let j = serde_json::to_string(&m).expect("should serialize to JSON");
        assert!(j.contains("user"));
    }
    #[test]
    fn test_role_serialization() {
        assert_eq!(
            serde_json::to_string(&Role::User).expect("should serialize to JSON"),
            "\"user\""
        );
    }
    #[test]
    fn test_role_deserialization() {
        let r: Role = serde_json::from_str("\"assistant\"").expect("should parse successfully");
        assert_eq!(r, Role::Assistant);
    }
    #[test]
    fn test_role_system() {
        let r: Role = serde_json::from_str("\"system\"").expect("should parse successfully");
        assert_eq!(r, Role::System);
    }
    #[test]
    fn test_role_tool() {
        let r: Role = serde_json::from_str("\"tool\"").expect("should parse successfully");
        assert_eq!(r, Role::Tool);
    }
    #[test]
    fn test_tool_schema_new() {
        let s = ToolSchema::new("test", "desc");
        assert_eq!(s.name, "test");
    }
    #[test]
    fn test_tool_schema_with_param() {
        let s = ToolSchema::new("t", "d").with_param("path", "string", "file path", true);
        let req = s.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(req.iter().any(|v| v == "path"));
    }
    #[test]
    fn test_tool_schema_with_optional_param() {
        let s = ToolSchema::new("t", "d").with_param("verbose", "boolean", "v", false);
        let req = s.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(!req.iter().any(|v| v == "verbose"));
    }
    #[test]
    fn test_tool_schema_multiple_params() {
        let s = ToolSchema::new("t", "d")
            .with_param("a", "string", "aa", true)
            .with_param("b", "integer", "bb", true);
        let req = s.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert_eq!(req.len(), 2);
    }
    #[test]
    fn test_tool_schema_serialization() {
        let s = ToolSchema::new("read", "reads");
        let j = serde_json::to_string(&s).expect("should serialize to JSON");
        assert!(j.contains("read"));
    }
    #[test]
    fn test_workspace_template_builtins_count() {
        assert_eq!(WorkspaceTemplate::builtins().len(), 5);
    }
    #[test]
    fn test_workspace_template_find_rust() {
        assert!(WorkspaceTemplate::find("rust-project").is_some());
    }
    #[test]
    fn test_workspace_template_find_python() {
        assert!(WorkspaceTemplate::find("python-data-science").is_some());
    }
    #[test]
    fn test_workspace_template_find_devops() {
        assert!(WorkspaceTemplate::find("devops").is_some());
    }
    #[test]
    fn test_workspace_template_find_research() {
        assert!(WorkspaceTemplate::find("research").is_some());
    }
    #[test]
    fn test_workspace_template_find_writing() {
        assert!(WorkspaceTemplate::find("writing").is_some());
    }
    #[test]
    fn test_workspace_template_find_missing() {
        assert!(WorkspaceTemplate::find("nonexistent").is_none());
    }
    #[test]
    fn test_workspace_template_has_files() {
        for t in WorkspaceTemplate::builtins() {
            assert!(!t.files.is_empty(), "{} empty", t.id);
        }
    }
    #[test]
    fn test_workspace_template_has_dirs() {
        for t in WorkspaceTemplate::builtins() {
            assert!(!t.directories.is_empty());
        }
    }
    #[test]
    fn test_workspace_template_has_description() {
        for t in WorkspaceTemplate::builtins() {
            assert!(!t.description.is_empty());
        }
    }
    #[test]
    fn test_workspace_template_has_tools() {
        for t in WorkspaceTemplate::builtins() {
            assert!(!t.default_tools.is_empty());
        }
    }
    #[test]
    fn test_workspace_template_apply() {
        let d = std::env::temp_dir().join("zeus_test_tpl");
        let _ = std::fs::remove_dir_all(&d);
        let t = WorkspaceTemplate::find("rust-project").expect("operation should succeed");
        assert!(t.apply(&d).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn test_auth_profile_available() {
        let p = AuthProfile {
            id: "p1".into(),
            name: "Test".into(),
            provider: "anthropic".into(),
            api_key: "sk-test".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        };
        assert!(p.is_available());
    }
    #[test]
    fn test_auth_profile_disabled() {
        let p = AuthProfile {
            id: "p1".into(),
            name: "T".into(),
            provider: "a".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: false,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        };
        assert!(!p.is_available());
    }
    #[test]
    fn test_auth_profile_record_usage() {
        let p = AuthProfile {
            id: "p1".into(),
            name: "T".into(),
            provider: "a".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        };
        p.record_usage(100);
        assert_eq!(
            p.tokens_used_today
                .load(std::sync::atomic::Ordering::Relaxed),
            100
        );
    }
    #[test]
    fn test_auth_profile_budget_exceeded() {
        let p = AuthProfile {
            id: "p1".into(),
            name: "T".into(),
            provider: "a".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: Some(100),
            tokens_used_today: std::sync::atomic::AtomicU64::new(200),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        };
        assert!(!p.is_available());
    }
    #[test]
    fn test_auth_profile_manager_empty() {
        let m = AuthProfileManager::new();
        assert!(m.all().is_empty());
    }
    #[test]
    fn test_auth_profile_manager_add() {
        let mut m = AuthProfileManager::new();
        m.add(AuthProfile {
            id: "p1".into(),
            name: "T".into(),
            provider: "a".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        });
        assert_eq!(m.all().len(), 1);
    }
    #[test]
    fn test_auth_profile_manager_get_by_id() {
        let mut m = AuthProfileManager::new();
        m.add(AuthProfile {
            id: "abc".into(),
            name: "T".into(),
            provider: "x".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        });
        assert!(m.get("abc").is_some());
        assert!(m.get("zzz").is_none());
    }
    #[test]
    fn test_auth_profile_manager_remove() {
        let mut m = AuthProfileManager::new();
        m.add(AuthProfile {
            id: "del".into(),
            name: "T".into(),
            provider: "x".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        });
        assert!(m.remove("del"));
        assert!(!m.remove("del"));
    }
    #[test]
    fn test_auth_profile_manager_get_available() {
        let mut m = AuthProfileManager::new();
        m.add(AuthProfile {
            id: "p1".into(),
            name: "T".into(),
            provider: "anthropic".into(),
            api_key: "k".into(),
            model: None,
            max_rpm: 60,
            max_tokens_per_day: None,
            tokens_used_today: Default::default(),
            enabled: true,
            priority: 0,
            cooldown_secs: 30,
            last_rate_limit: std::sync::Mutex::new(None),
        });
        assert!(m.get_available("anthropic").is_some());
        assert!(m.get_available("openai").is_none());
    }
    #[test]
    fn test_config_default() {
        let c = Config::default();
        assert!(c.max_iterations > 0);
    }
    #[test]
    fn test_config_serialization() {
        // Default config should skip default values
        let c = Config::default();
        let j = serde_json::to_string(&c).expect("should serialize to JSON");
        assert!(!j.contains("max_iterations"), "default max_iterations should be skipped");
        assert!(!j.contains("verbosity"), "default verbosity should be skipped");
        // Non-default values should serialize
        let mut c2 = Config::default();
        c2.max_iterations = 42;
        let j2 = serde_json::to_string(&c2).expect("should serialize to JSON");
        assert!(j2.contains("max_iterations"), "non-default max_iterations should be present");
    }
    #[test]
    fn test_gateway_port_in_reads_explicit_port() {
        // #311: guard helper must see the explicit [gateway] port…
        let c = "[gateway]\nhost = \"0.0.0.0\"\nport = 8080\n";
        assert_eq!(Config::gateway_port_in(c), Some(8080));
        // …with trailing comments…
        let c2 = "[gateway]\nport = 3001  # custom\n";
        assert_eq!(Config::gateway_port_in(c2), Some(3001));
        // …but never a commented-out port, another table's port, or none.
        assert_eq!(Config::gateway_port_in("[gateway]\n# port = 9999\n"), None);
        assert_eq!(Config::gateway_port_in("[api]\nport = 7777\n"), None);
        assert_eq!(Config::gateway_port_in("model = \"x\"\n"), None);
    }

    #[test]
    fn test_persist_gateway_port_guarded_respects_explicit_port() {
        // #311: an explicit on-disk port must never be clobbered by the
        // runtime port (the stale-rc.conf hijack). Path-parameterized —
        // no env mutation, real config untouched.
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "[gateway]\nport = 8080\n").expect("write");
        // Differing runtime port → override reported, file untouched.
        let r = Config::persist_gateway_port_guarded_at(&cfg, 3001).expect("guarded persist");
        assert_eq!(r, PortPersist::OverrideDetected { config_port: 8080 });
        let on_disk = std::fs::read_to_string(&cfg).expect("read");
        assert!(on_disk.contains("port = 8080"), "explicit port must survive");
        // Matching runtime port → no change.
        let r2 = Config::persist_gateway_port_guarded_at(&cfg, 8080).expect("guarded persist");
        assert_eq!(r2, PortPersist::NoChange);
    }

    #[test]
    fn test_persist_gateway_port_guarded_fills_missing_port() {
        // #311: when config has NO explicit port, first write wins.
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "[gateway]\nhost = \"0.0.0.0\"\n").expect("write");
        // No `port` key → guarded persist records the runtime port.
        let r = Config::persist_gateway_port_guarded_at(&cfg, 3001).expect("guarded persist");
        assert_eq!(r, PortPersist::Persisted);
        let on_disk = std::fs::read_to_string(&cfg).expect("read");
        assert!(on_disk.contains("port = 3001"), "runtime port recorded");
        assert!(on_disk.contains("host = \"0.0.0.0\""), "siblings preserved");
        // No [gateway] table at all → nothing to record into.
        let cfg2 = dir.path().join("config2.toml");
        std::fs::write(&cfg2, "model = \"x\"\n").expect("write");
        let r_no_table =
            Config::persist_gateway_port_guarded_at(&cfg2, 3001).expect("guarded persist");
        assert_eq!(r_no_table, PortPersist::NoChange);
        // Missing config entirely → NoConfig.
        let r2 = Config::persist_gateway_port_guarded_at(&dir.path().join("nope.toml"), 3001)
            .expect("guarded persist");
        assert_eq!(r2, PortPersist::NoConfig);
    }

    #[test]
    fn test_rewrite_gateway_port_preserves_comments() {
        // #105 fix #1: surgical, comment-preserving port rewrite
        let input = "model = \"x\"\n\n# ── Gateway ──\n[gateway]\nhost = \"127.0.0.1\"\nport = 8080\nenable_mcp = true\n";
        let out = Config::rewrite_gateway_port(input, 3001);
        assert!(out.contains("port = 3001"), "port should be rewritten to 3001");
        assert!(!out.contains("port = 8080"), "stale port should be gone");
        assert!(out.contains("# ── Gateway ──"), "comments preserved");
        assert!(out.contains("host = \"127.0.0.1\""), "sibling keys preserved");
        assert!(out.contains("enable_mcp = true"), "trailing keys preserved");
    }

    #[test]
    fn test_rewrite_gateway_port_idempotent_when_matching() {
        let input = "[gateway]\nport = 3001\n";
        let out = Config::rewrite_gateway_port(input, 3001);
        assert_eq!(out, input, "no change when already matching");
    }

    #[test]
    fn test_rewrite_gateway_port_only_touches_gateway_table() {
        // A `port` key in another table must NOT be rewritten.
        let input = "[gateway]\nport = 8080\n\n[other]\nport = 9999\n";
        let out = Config::rewrite_gateway_port(input, 3001);
        assert!(out.contains("[gateway]\nport = 3001"), "gateway port rewritten");
        assert!(out.contains("[other]\nport = 9999"), "other table's port untouched");
    }

    #[test]
    fn test_rewrite_gateway_port_ignores_commented_port() {
        let input = "[gateway]\n# port = 8080\nport = 8080\n";
        let out = Config::rewrite_gateway_port(input, 3001);
        assert!(out.contains("# port = 8080"), "commented line preserved verbatim");
        assert!(out.contains("\nport = 3001"), "real port line rewritten");
    }

    #[test]
    fn test_atomic_write_creates_file_and_cleans_tmp() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let target = dir.path().join("config.toml");
        let content = "[gateway]\nport = 8080\n";
        Config::atomic_write(&target, content).expect("atomic write should succeed");
        // Target file exists with correct content
        let read = std::fs::read_to_string(&target).expect("should read file");
        assert_eq!(read, content);
        // Temp file should be cleaned up
        let tmp = dir.path().join(".config.toml.tmp");
        assert!(!tmp.exists(), "temp file should not linger after rename");
    }
    #[test]
    fn test_atomic_write_preserves_existing_on_no_crash() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let target = dir.path().join("config.toml");
        let original = "[gateway]\nport = 8080\n";
        std::fs::write(&target, original).expect("write original");
        let updated = "[gateway]\nport = 9090\n";
        Config::atomic_write(&target, updated).expect("atomic write should succeed");
        let read = std::fs::read_to_string(&target).expect("should read file");
        assert_eq!(read, updated);
    }
    #[test]
    fn test_dm_policy_open() {
        let p = DmPolicy::Open;
        let j = serde_json::to_string(&p).expect("should serialize to JSON");
        assert_eq!(j, "\"open\"");
    }
    #[test]
    fn test_dm_policy_disabled() {
        let _p = DmPolicy::Disabled;
        assert!(format!("{:?}", _p).contains("Disabled"));
    }
    #[test]
    fn test_dm_policy_pairing() {
        let p = DmPolicy::Pairing;
        let j = serde_json::to_string(&p).expect("should serialize to JSON");
        let d: DmPolicy = serde_json::from_str(&j).expect("should parse successfully");
        assert_eq!(d, DmPolicy::Pairing);
    }
    #[test]
    fn test_dm_policy_allowlist() {
        let p = DmPolicy::Allowlist;
        let j = serde_json::to_string(&p).expect("should serialize to JSON");
        let d: DmPolicy = serde_json::from_str(&j).expect("should parse successfully");
        assert_eq!(d, DmPolicy::Allowlist);
    }
    #[test]
    fn test_channel_policy_config_commands_allowlist_serde() {
        let config = ChannelPolicyConfig {
            commands_allowlist: Some(vec!["status".to_string(), "memory".to_string()]),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        assert!(json.contains("commands_allowlist"));
        let deserialized: ChannelPolicyConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(
            deserialized.commands_allowlist,
            Some(vec!["status".to_string(), "memory".to_string()])
        );
    }
    #[test]
    fn test_channel_policy_config_backward_compat_no_commands() {
        // Old JSON without commands_allowlist should deserialize to None
        let json = r#"{"dm":"open","group":"mentiononly","allow_from":[],"allow_groups":[],"tools_allowlist":null}"#;
        let config: ChannelPolicyConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(config.commands_allowlist.is_none());
    }
    #[test]
    fn test_provider_anthropic() {
        let p = Provider::Anthropic;
        let j = serde_json::to_string(&p).expect("should serialize to JSON");
        assert!(j.contains("anthropic") || j.contains("Anthropic"));
    }
    #[test]
    fn test_provider_openai() {
        let j = serde_json::to_string(&Provider::OpenAI).expect("should serialize to JSON");
        assert!(!j.is_empty());
    }
    #[test]
    fn test_provider_ollama() {
        let j = serde_json::to_string(&Provider::Ollama).expect("should serialize to JSON");
        assert!(!j.is_empty());
    }
    #[test]
    fn test_tool_call_serialization() {
        let tc = ToolCall {
            id: "1".into(),
            name: "test".into(),
            arguments: serde_json::json!({"x": 1}),
        };
        let j = serde_json::to_string(&tc).expect("should serialize to JSON");
        assert!(j.contains("test"));
    }
    #[test]
    fn test_tool_result_serialization() {
        let tr = ToolResult {
            call_id: "1".into(),
            success: true,
            output: "ok".into(),
        };
        let j = serde_json::to_string(&tr).expect("should serialize to JSON");
        assert!(j.contains("ok"));
    }
    #[test]
    fn test_attachment_struct() {
        let a = Attachment {
            mime_type: "text/plain".into(),
            data: vec![1, 2, 3],
            filename: Some("test.rs".into()),
            source_url: None,
        };
        assert_eq!(a.filename.as_deref(), Some("test.rs"));
    }
    #[test]
    fn test_attachment_image() {
        let a = Attachment {
            mime_type: "image/png".into(),
            data: vec![],
            filename: Some("photo.png".into()),
            source_url: None,
        };
        assert_eq!(a.mime_type, "image/png");
    }

    // ========================================================================
    // Message threading tests
    // ========================================================================

    #[test]
    fn test_message_auto_generates_id() {
        let user = Message::user("hello");
        assert!(user.message_id.is_some());
        let assistant = Message::assistant("hi");
        assert!(assistant.message_id.is_some());
        let system = Message::system("prompt");
        assert!(system.message_id.is_some());
        let tool = Message::tool("call1", true, "output");
        assert!(tool.message_id.is_some());
        // All IDs should be unique
        assert_ne!(user.message_id, assistant.message_id);
        assert_ne!(assistant.message_id, system.message_id);
        assert_ne!(system.message_id, tool.message_id);
    }

    #[test]
    fn test_message_serde_with_threading() {
        let msg = Message::user("test")
            .with_parent("parent-123")
            .with_thread("thread-456");
        assert_eq!(msg.parent_id.as_deref(), Some("parent-123"));
        assert_eq!(msg.thread_id.as_deref(), Some("thread-456"));

        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("parent-123"));
        assert!(json.contains("thread-456"));

        let parsed: Message = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.parent_id.as_deref(), Some("parent-123"));
        assert_eq!(parsed.thread_id.as_deref(), Some("thread-456"));
        assert!(parsed.message_id.is_some());
    }

    #[test]
    fn test_message_serde_backward_compat() {
        // Old format without threading fields should deserialize fine
        let json = r#"{"role":"user","content":"hello","tool_calls":[],"tool_results":[],"timestamp":"2026-02-14T12:00:00Z","attachments":[]}"#;
        let msg: Message = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(msg.content, "hello");
        assert!(msg.message_id.is_none());
        assert!(msg.parent_id.is_none());
        assert!(msg.thread_id.is_none());
    }

    #[test]
    fn test_message_with_parent_builder() {
        let msg = Message::assistant("response")
            .with_parent("msg-abc")
            .with_thread("msg-abc");
        assert_eq!(msg.parent_id.as_deref(), Some("msg-abc"));
        assert_eq!(msg.thread_id.as_deref(), Some("msg-abc"));
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "response");
    }

    // ========================================================================
    // RTL text direction tests
    // ========================================================================

    #[test]
    fn test_is_rtl_arabic() {
        assert!(is_rtl("مرحبا بالعالم")); // "Hello world" in Arabic
    }

    #[test]
    fn test_is_rtl_hebrew() {
        assert!(is_rtl("שלום עולם")); // "Hello world" in Hebrew
    }

    #[test]
    fn test_is_rtl_syriac() {
        assert!(is_rtl("\u{0710}\u{0712}\u{0713}")); // Syriac characters
    }

    #[test]
    fn test_is_rtl_english_is_ltr() {
        assert!(!is_rtl("Hello, world!"));
    }

    #[test]
    fn test_is_rtl_empty_string() {
        assert!(!is_rtl(""));
    }

    #[test]
    fn test_is_rtl_numbers_and_punctuation_only() {
        // No strong directional characters — defaults to LTR
        assert!(!is_rtl("12345 !@#$%"));
    }

    #[test]
    fn test_is_rtl_mixed_majority_arabic() {
        // Arabic with some English — RTL should win
        assert!(is_rtl("مرحبا hello بالعالم"));
    }

    #[test]
    fn test_is_rtl_mixed_majority_english() {
        // English with one Arabic word — LTR should win
        assert!(!is_rtl(
            "Hello world مرحبا and goodbye everyone in this long sentence"
        ));
    }

    #[test]
    fn test_detect_direction_arabic() {
        assert_eq!(detect_direction("مرحبا"), TextDirection::Rtl);
    }

    #[test]
    fn test_detect_direction_english() {
        assert_eq!(detect_direction("Hello"), TextDirection::Ltr);
    }

    #[test]
    fn test_text_direction_default_is_ltr() {
        assert_eq!(TextDirection::default(), TextDirection::Ltr);
    }

    #[test]
    fn test_text_direction_serde_roundtrip() {
        let rtl = TextDirection::Rtl;
        let json = serde_json::to_string(&rtl).expect("should serialize to JSON");
        assert_eq!(json, r#""rtl""#);
        let parsed: TextDirection = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed, TextDirection::Rtl);

        let ltr = TextDirection::Ltr;
        let json = serde_json::to_string(&ltr).expect("should serialize to JSON");
        assert_eq!(json, r#""ltr""#);
        let parsed: TextDirection = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed, TextDirection::Ltr);
    }

    #[test]
    fn test_message_auto_detects_rtl_direction() {
        let msg = Message::user("مرحبا بالعالم");
        assert_eq!(msg.direction, TextDirection::Rtl);

        let msg = Message::user("Hello world");
        assert_eq!(msg.direction, TextDirection::Ltr);

        let msg = Message::assistant("שלום עולם");
        assert_eq!(msg.direction, TextDirection::Rtl);
    }

    #[test]
    fn test_message_tool_always_ltr() {
        let msg = Message::tool("call1", true, "output");
        assert_eq!(msg.direction, TextDirection::Ltr);
    }

    #[test]
    fn test_message_direction_serde_backward_compat() {
        // Old JSON without direction field should deserialize with default (Ltr)
        let json = r#"{"role":"user","content":"hello","tool_calls":[],"tool_results":[],"timestamp":"2026-02-14T12:00:00Z","attachments":[]}"#;
        let msg: Message = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(msg.direction, TextDirection::Ltr);
    }

    #[test]
    fn test_message_direction_serde_roundtrip() {
        let msg = Message::user("مرحبا");
        assert_eq!(msg.direction, TextDirection::Rtl);

        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains(r#""direction":"rtl""#));

        let parsed: Message = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.direction, TextDirection::Rtl);
    }

    #[test]
    fn test_is_rtl_thaana() {
        // Thaana script (Maldivian) - U+0780 range
        assert!(is_rtl("\u{0780}\u{0781}\u{0782}"));
    }

    #[test]
    fn test_is_rtl_nko() {
        // N'Ko script - U+07C0 range
        assert!(is_rtl("\u{07C0}\u{07C1}\u{07C2}"));
    }

    #[test]
    fn test_is_rtl_cyrillic_is_ltr() {
        assert!(!is_rtl("Привет мир")); // Russian "Hello world"
    }

    #[test]
    fn test_is_rtl_cjk_is_ltr() {
        assert!(!is_rtl("你好世界")); // Chinese "Hello world"
    }

    // ── Config corruption guard tests ──────────────────────────────────

    // INVARIANT (#97 / #97-hardening): these tests mutate process-global ZEUS_HOME
    // via redirect_zeus_home() and therefore MUST run serially. ZEUS_HOME is a
    // process-wide env var; two env-mutating tests racing under a parallel test
    // runner would reintroduce the exact config-wipe #97 closed. We enforce serial
    // execution with a static Mutex (zero-dep alternative to `serial_test`): every
    // guard test holds ENV_GUARD for its full duration, so cargo's default parallel
    // harness cannot interleave them. See #97.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // SAFETY: All save()-invoking guard tests redirect ZEUS_HOME to a tempdir so
    // that even a guard regression cannot write to the REAL ~/.zeus/config.toml.
    // This is defense-in-depth on top of save()'s own temp-path / default guards.
    // Returns (env-lock guard, TempDir) — keep BOTH alive for the test's duration:
    // the MutexGuard serializes ZEUS_HOME mutation, the TempDir keeps the dir live.
    fn redirect_zeus_home() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
        // Lock first so no other guard test can be mutating ZEUS_HOME concurrently.
        // recover from a poisoned lock — a panicking prior test must not block the suite.
        let lock = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("create tempdir for ZEUS_HOME");
        // SAFETY: ENV_GUARD is held, so these synchronous guard tests are the only
        // writers of ZEUS_HOME at this instant; the value is a freshly-created
        // tempdir path. The lock guarantees no concurrent env mutation/read here.
        unsafe {
            std::env::set_var("ZEUS_HOME", tmp.path());
        }
        (lock, tmp)
    }

    #[test]
    fn test_save_rejects_tmp_workspace() {
        let (_lock, _home) = redirect_zeus_home();
        let mut config = Config::default();
        config.workspace = PathBuf::from("/tmp/.tmpABC123/workspace");
        let result = config.save();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("temp directory"));
    }

    #[test]
    fn test_save_rejects_var_folders_workspace() {
        let (_lock, _home) = redirect_zeus_home();
        let mut config = Config::default();
        config.workspace =
            PathBuf::from("/var/folders/ab/cd1234/T/.tmpXYZ/workspace");
        let result = config.save();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("temp directory"));
    }

    #[test]
    fn test_save_rejects_tmp_sessions() {
        let (_lock, _home) = redirect_zeus_home();
        let mut config = Config::default();
        config.sessions = PathBuf::from("/tmp/.tmpABC123/sessions");
        let result = config.save();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("temp directory"));
    }

    // #291: save_unchecked() — the only writer with no merge and no guards — must
    // refuse a /var/folders workspace, in parity with save() above. loaded_from_default
    // is forced FALSE so this isolates the temp-path guard from the #277 default guard
    // (a future #277 change must not silently mask the temp-path refusal).
    #[test]
    fn test_save_unchecked_rejects_var_folders_workspace() {
        let (_lock, _home) = redirect_zeus_home();
        let config = Config {
            loaded_from_default: false,
            workspace: PathBuf::from("/var/folders/ab/cd1234/T/.tmpXYZ/workspace"),
            ..Config::default()
        };
        let result = config.save_unchecked();
        assert!(
            result.is_err(),
            "save_unchecked must refuse a /var/folders workspace"
        );
        assert!(result.unwrap_err().to_string().contains("temp directory"));
    }

    // #291: same parity for a /tmp sessions path through save_unchecked().
    #[test]
    fn test_save_unchecked_rejects_tmp_sessions() {
        let (_lock, _home) = redirect_zeus_home();
        let config = Config {
            loaded_from_default: false,
            sessions: PathBuf::from("/tmp/.tmpABC123/sessions"),
            ..Config::default()
        };
        let result = config.save_unchecked();
        assert!(
            result.is_err(),
            "save_unchecked must refuse a /tmp sessions path"
        );
        assert!(result.unwrap_err().to_string().contains("temp directory"));
    }

    // #291 (the real fix): a config whose paths were expanded under a temp HOME must
    // round-trip to disk in portable `~/` form, NOT as an absolute /var/folders path.
    // Simulates the bug: expand_tildes() under a temp HOME turns ~/.zeus/* into
    // <temp>/.zeus/*; collapse-on-save must rewrite it back to ~/ before serialization.
    #[test]
    fn test_collapse_on_save_writes_tilde_not_temp_home() {
        let (_lock, home) = redirect_zeus_home();
        let home_dir = dirs::home_dir().expect("home dir for test");
        // Paths as expand_tildes() would have produced them (home-rooted, absolute).
        let config = Config {
            loaded_from_default: false,
            onboarding_complete: true,
            workspace: home_dir.join(".zeus/workspace"),
            sessions: home_dir.join(".zeus/sessions"),
            ..Config::default()
        };
        config
            .save_unchecked()
            .expect("home-rooted paths must save (collapsed to ~/)");

        let written = std::fs::read_to_string(home.path().join("config.toml"))
            .expect("config.toml must exist after save");
        assert!(
            written.contains("workspace = \"~/.zeus/workspace\""),
            "workspace must persist as ~/ form, got:\n{written}"
        );
        assert!(
            written.contains("sessions = \"~/.zeus/sessions\""),
            "sessions must persist as ~/ form, got:\n{written}"
        );
        assert!(
            !written.contains("/var/folders/") && !written.contains(home_dir.to_string_lossy().as_ref()),
            "no absolute home/temp path may round-trip to disk, got:\n{written}"
        );
    }

    // #291: collapse_tildes is the exact inverse of expand_tildes for the 4 path
    // fields — a load→save round-trip is path-stable.
    #[test]
    fn test_collapse_tildes_inverts_expand_tildes() {
        let home_dir = dirs::home_dir().expect("home dir for test");
        let mut config = Config {
            workspace: PathBuf::from("~/.zeus/workspace"),
            sessions: PathBuf::from("~/.zeus/sessions"),
            ..Config::default()
        };
        config.expand_tildes();
        assert_eq!(config.workspace, home_dir.join(".zeus/workspace"));
        config.collapse_tildes();
        assert_eq!(config.workspace, PathBuf::from("~/.zeus/workspace"));
        assert_eq!(config.sessions, PathBuf::from("~/.zeus/sessions"));
    }

    #[test]
    fn test_save_unchecked_refuses_default_over_existing_config() {
        // #277: the random seat-config nuke. Config::load() returns a guarded
        // default (loaded_from_default = true) when config.toml is missing OR
        // unreadable/parse-fails. save_unchecked must NOT overwrite an EXISTING
        // file with that default — but first-run onboarding (no file) is allowed.
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");

        // First-run: no file on disk → a loaded_from_default save IS allowed.
        let fresh = Config { loaded_from_default: true, ..Config::default() };
        assert!(
            fresh.save_unchecked().is_ok(),
            "first-run onboarding (no config file yet) must be allowed to write"
        );
        assert!(cfg_path.exists(), "first-run save_unchecked should create config.toml");

        // Now a real config exists on disk. A loaded_from_default config (e.g.
        // from a read/parse failure) must be REFUSED — this is the nuke guard.
        let nuke = Config { loaded_from_default: true, ..Config::default() };
        let result = nuke.save_unchecked();
        assert!(
            result.is_err(),
            "save_unchecked must refuse a loaded_from_default config when config.toml already exists"
        );
        assert!(
            result.unwrap_err().to_string().contains("loaded_from_default"),
            "the refusal must cite loaded_from_default"
        );

        // A genuinely-loaded config (loaded_from_default = false) still writes.
        let real = Config { loaded_from_default: false, ..Config::default() };
        assert!(
            real.save_unchecked().is_ok(),
            "a real (non-default) config must still save_unchecked normally"
        );
    }

    fn named_real_config(agent_name: &str) -> Config {
        Config {
            loaded_from_default: false,
            onboarding_complete: true,
            model: "anthropic/claude-sonnet-4-6".to_string(),
            agent: Some(AgentSection {
                name: Some(agent_name.to_string()),
                persona: Some("The Operator".to_string()),
                role: Some("Infra".to_string()),
                coordinator: Some("Zeus100".to_string()),
            }),
            ..Config::default()
        }
    }

    #[test]
    fn test_save_rejects_prepended_duplicate_top_level_config_debris() {
        // #309: a template prepended over a real config creates duplicate
        // top-level keys/tables. Even if TOML parsing would otherwise fall back
        // to writing the new struct, the central save path must refuse before
        // replacing recoverable debris on disk.
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");
        let debris = r#"model = "ollama/llama3.2"
workspace = "/tmp/template-workspace"
model = "anthropic/claude-sonnet-4-6"
onboarding_complete = true

[agent]
name = "zeus106"
"#;
        std::fs::write(&cfg_path, debris).unwrap();

        let err = named_real_config("zeus106")
            .save()
            .expect_err("duplicate top-level debris must be refused");
        assert!(
            err.to_string().contains("duplicate top-level key model"),
            "unexpected error: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&cfg_path).unwrap(),
            debris,
            "refused write must leave the debris file untouched for recovery"
        );
    }

    #[test]
    fn test_save_rejects_identityless_template_in_veteran_home_before_real_identity_merge() {
        // #309 boot-check signature: no configured identity in a ZEUS_HOME that
        // already has veteran runtime state. A later real-identity save must not
        // merge/preserve that bootstrap/template body as if it were a healthy
        // forward-compatible config.
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");
        std::fs::write(home.path().join("memory.db"), b"veteran state").unwrap();
        let identityless_template = r#"model = "anthropic/claude-sonnet-4-6"
onboarding_complete = false

[agent]
persona = "The Herald"
"#;
        std::fs::write(&cfg_path, identityless_template).unwrap();

        let err = named_real_config("zeus106")
            .save()
            .expect_err("identity-less veteran-state config must be refused");
        assert!(
            err.to_string().contains("identity-less"),
            "unexpected error: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&cfg_path).unwrap(),
            identityless_template,
            "refused write must preserve the identity-less template for operator recovery"
        );
    }

    #[test]
    fn test_save_preserves_unknown_sections_when_no_debris_shape_exists() {
        // Forward-compat is intentional: unknown sections must survive a normal
        // Config::save(). The #309 guard is narrow and rejects only debris shapes,
        // not future config tables the current binary does not understand.
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");
        let existing = r#"model = "anthropic/claude-opus-4"
onboarding_complete = true

[agent]
name = "zeus106"
persona = "The Operator"

[future_feature]
enabled = true
label = "keep-me"

[future_feature.nested]
value = "still-here"
"#;
        std::fs::write(&cfg_path, existing).unwrap();

        named_real_config("zeus106")
            .save()
            .expect("unknown sections are not debris and must be preserved");

        let written = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(written.contains("[future_feature]"), "{written}");
        assert!(written.contains("enabled = true"), "{written}");
        assert!(written.contains("label = \"keep-me\""), "{written}");
        assert!(written.contains("[future_feature.nested]"), "{written}");
        assert!(written.contains("value = \"still-here\""), "{written}");
        assert!(written.contains("name = \"zeus106\""), "{written}");
    }


    #[test]
    fn test_load_parse_failure_preserves_config_and_guards_save() {
        // #123: a config.toml that fails to parse (e.g. a newer binary with an
        // incompatible schema) must NOT be silently overwritten with defaults.
        // load() returns a GUARDED default (loaded_from_default = true) so save()
        // is refused, the real file is preserved untouched, and the unparseable
        // content is stashed to .parse-error-backup for recovery.
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");
        let malformed = "model = \"anthropic/claude-x\"\nthis is = not valid = toml [[[";
        std::fs::write(&cfg_path, malformed).unwrap();

        let loaded = Config::load().expect("load() must not Err on parse-fail");
        assert!(
            loaded.loaded_from_default,
            "parse-fail must yield a guarded default so save() is refused"
        );
        assert!(
            loaded.save().is_err(),
            "save() must be refused for a parse-fail-guarded default (no wipe)"
        );

        // The real config on disk is preserved, NOT replaced with defaults.
        assert_eq!(
            std::fs::read_to_string(&cfg_path).unwrap(),
            malformed,
            "config.toml must be preserved verbatim on parse-fail"
        );

        // The unparseable content is stashed for recovery.
        let recovery = home.path().join("config.toml.parse-error-backup");
        assert!(recovery.exists(), ".parse-error-backup must be created");
        assert_eq!(std::fs::read_to_string(&recovery).unwrap(), malformed);
    }

    #[test]
    fn test_load_success_backs_up_known_good_only() {
        // #123: the .bak must hold the last KNOWN-GOOD config — backup happens
        // only AFTER a successful parse, never before (which previously clobbered
        // a good .bak with a corrupt/wiped config on the post-bad-write load).
        let (_lock, home) = redirect_zeus_home();
        let cfg_path = home.path().join("config.toml");
        let good = "model = \"anthropic/claude-sonnet-4-6\"\nonboarding_complete = true\n";
        std::fs::write(&cfg_path, good).unwrap();

        let loaded = Config::load().expect("valid config must load");
        assert!(!loaded.loaded_from_default, "a real config is not a default");

        let bak = home.path().join("config.toml.bak");
        assert!(bak.exists(), "successful load must back up to .bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), good);
    }

    #[test]
    fn test_ensure_required_sections_fills_missing() {
        let mut config = Config::default();
        assert!(config.mnemosyne.is_none());
        assert!(config.nous.is_none());
        assert!(config.hermes.is_none());

        config.ensure_required_sections();

        assert!(config.mnemosyne.is_some());
        assert!(config.nous.is_some());
        assert!(config.hermes.is_some());
        assert!(config.nous.as_ref().unwrap().enable_learning);
    }

    #[test]
    fn test_ensure_required_sections_preserves_existing() {
        let mut config = Config::default();
        config.mnemosyne = Some(MnemosyneConfig {
            db_path: PathBuf::from("/custom/path.db"),
            ..Default::default()
        });

        config.ensure_required_sections();

        // Should preserve custom path, not overwrite with default
        assert_eq!(
            config.mnemosyne.as_ref().unwrap().db_path,
            PathBuf::from("/custom/path.db")
        );
    }

    #[test]
    fn test_save_guards_reject_tmp_paths() {
        // Verify that save() rejects temp workspace paths.
        // DO NOT call save_unchecked() — it writes to the REAL ~/.zeus/config.toml
        // and will corrupt production config! (This was the root cause of config
        // wipes on every `cargo test` run.)
        let (_lock, _home) = redirect_zeus_home();
        let mut config = Config::default();
        config.workspace = PathBuf::from("/tmp/.tmpABC123/workspace");
        config.loaded_from_default = false; // bypass the default guard
        let result = config.save();
        assert!(result.is_err(), "save() should reject /tmp/ workspace paths");
    }
}

#[cfg(test)]
mod track_c_tests {
    use super::Config;

    #[test]
    fn test_images_to_talos_image_migration() {
        let legacy = r#"model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"

[images]
provider = "fooocus"
url = "http://localhost:7865"
api_key = "sk-test"
model = "sdxl-turbo"
default_width = 512
default_height = 512
store_path = "~/.zeus/images"
"#;
        let processed = Config::post_process_config_toml(legacy);
        assert!(!processed.contains("[images]"), "orphan [images] should be removed");
        assert!(processed.contains("[talos.image]"), "[talos.image] should exist");
        assert!(processed.contains("provider = \"fooocus\""), "provider should migrate");
        assert!(processed.contains("url = \"http://localhost:7865\""), "url should migrate");
        assert!(processed.contains("store_path = \"~/.zeus/images\""), "store_path should migrate");
    }

    #[test]
    fn test_prometheus_heartbeat_preserved() {
        let toml = r#"model = "anthropic/claude-sonnet-4-20250514"

[prometheus]
enable_heartbeat = true
heartbeat_interval_secs = 300
"#;
        let processed = Config::post_process_config_toml(toml);
        assert!(processed.contains("[prometheus.heartbeat]"), "[prometheus.heartbeat] should be added");
    }

    #[test]
    fn test_no_double_talos_image() {
        let toml = r#"model = "anthropic/claude-sonnet-4-20250514"

[talos.image]
provider = "fooocus"
"#;
        let processed = Config::post_process_config_toml(toml);
        // Should NOT add [images] or duplicate [talos.image]
        assert!(!processed.contains("[images]"));
        assert_eq!(processed.matches("[talos.image]").count(), 1);
    }
}

#[cfg(test)]
mod phase_3_5_typed_sections_tests {
    //! Round-trip tests for the Phase 3.5 typed sections:
    //!   • `[voice]` → `VoiceConfig`
    //!   • `[prometheus.scheduler]` → `PrometheusSchedulerConfig`
    //!   • `[prometheus.autonomy]` → `PrometheusAutonomyConfig`
    //!   • `[prometheus.learning]` → `PrometheusLearningConfig`
    //!   • `[prometheus.monitor]` → `PrometheusMonitorConfig`
    //!   • `[prometheus.heartbeat]` → `PrometheusHeartbeatConfig`
    //!
    //! Each test stamps a typed value into `Config`, serializes to TOML,
    //! parses back, and asserts the deserialized struct fields match.

    use super::*;

    fn roundtrip(c: &Config) -> Config {
        let toml = toml::to_string(c).expect("serialize Config to TOML");
        toml::from_str::<Config>(&toml).expect("parse Config from TOML")
    }

    #[test]
    fn voice_config_roundtrips() {
        let mut c = Config::default();
        c.voice = Some(VoiceConfig {
            provider: Some("elevenlabs".into()),
            model: Some("rachel".into()),
            enabled: true,
            extra: Default::default(),
        });
        let back = roundtrip(&c);
        let v = back.voice.expect("voice section present after roundtrip");
        assert_eq!(v.provider.as_deref(), Some("elevenlabs"));
        assert_eq!(v.model.as_deref(), Some("rachel"));
        assert!(v.enabled);
    }

    #[test]
    fn voice_config_extra_fields_preserved() {
        let mut c = Config::default();
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("voice_id".to_string(), serde_json::json!("21m00Tcm4TlvDq8ikWAM"));
        extra.insert("stability".to_string(), serde_json::json!(0.7));
        c.voice = Some(VoiceConfig {
            provider: Some("elevenlabs".into()),
            model: None,
            enabled: false,
            extra,
        });
        let back = roundtrip(&c);
        let v = back.voice.expect("voice present");
        assert_eq!(v.extra.get("voice_id").and_then(|j| j.as_str()), Some("21m00Tcm4TlvDq8ikWAM"));
        assert!(v.extra.get("stability").and_then(|j| j.as_f64()).is_some());
    }

    #[test]
    fn prometheus_scheduler_typed_roundtrips() {
        let mut c = Config::default();
        c.prometheus = Some(PrometheusConfig {
            scheduler: Some(PrometheusSchedulerConfig {
                enabled: true,
                max_concurrent_jobs: 8,
                tasks: vec![serde_json::json!({
                    "name": "Daily review",
                    "cron": "0 9 * * *",
                })],
            }),
            ..PrometheusConfig::default()
        });
        let back = roundtrip(&c);
        let s = back.prometheus.unwrap().scheduler.expect("scheduler present");
        assert!(s.enabled);
        assert_eq!(s.max_concurrent_jobs, 8);
        assert_eq!(s.tasks.len(), 1);
    }

    #[test]
    fn prometheus_autonomy_typed_roundtrips() {
        let mut c = Config::default();
        c.prometheus = Some(PrometheusConfig {
            autonomy: Some(PrometheusAutonomyConfig {
                level: "full".into(),
                confidence_threshold: 0.85,
                max_autonomous_tools: 50,
                require_confirmation_for: vec!["shell.rm".into(), "git.push".into()],
                error_threshold: 5,
            }),
            ..PrometheusConfig::default()
        });
        let back = roundtrip(&c);
        let a = back.prometheus.unwrap().autonomy.expect("autonomy present");
        assert_eq!(a.level, "full");
        assert!((a.confidence_threshold - 0.85).abs() < 1e-6);
        assert_eq!(a.max_autonomous_tools, 50);
        assert_eq!(a.require_confirmation_for, vec!["shell.rm", "git.push"]);
        assert_eq!(a.error_threshold, 5);
    }

    #[test]
    fn prometheus_learning_typed_roundtrips() {
        let mut c = Config::default();
        c.prometheus = Some(PrometheusConfig {
            learning: Some(PrometheusLearningConfig {
                db_path: Some(std::path::PathBuf::from("/tmp/test-learning.db")),
                min_observations: Some(100),
                confidence_threshold: Some(0.6),
                extra: Default::default(),
            }),
            ..PrometheusConfig::default()
        });
        let back = roundtrip(&c);
        let l = back.prometheus.unwrap().learning.expect("learning present");
        assert_eq!(l.db_path.as_deref(), Some(std::path::Path::new("/tmp/test-learning.db")));
        assert_eq!(l.min_observations, Some(100));
    }

    #[test]
    fn prometheus_monitor_typed_roundtrips() {
        let mut c = Config::default();
        c.prometheus = Some(PrometheusConfig {
            monitor: Some(PrometheusMonitorConfig {
                check_interval_secs: Some(60),
                error_rate_threshold: Some(0.1),
                latency_threshold_ms: Some(5000),
                min_success_rate: Some(0.95),
                extra: Default::default(),
            }),
            ..PrometheusConfig::default()
        });
        let back = roundtrip(&c);
        let m = back.prometheus.unwrap().monitor.expect("monitor present");
        assert_eq!(m.check_interval_secs, Some(60));
        assert_eq!(m.latency_threshold_ms, Some(5000));
    }

    #[test]
    fn prometheus_heartbeat_typed_roundtrips() {
        let mut c = Config::default();
        c.prometheus = Some(PrometheusConfig {
            heartbeat: Some(PrometheusHeartbeatConfig {
                quiet_hours_start: 22,
                quiet_hours_end: 7,
                enable_quiet_hours: true,
                timezone: Some("America/Los_Angeles".into()),
                timeout_secs: 60,
                dedup_window_secs: 3600,
                active_interval_secs: 90,
                event_driven_only: false,
                safety_net_interval_secs: 1800,
                plan_resume_interval_secs: 1800,
                extra: Default::default(),
            }),
            ..PrometheusConfig::default()
        });
        let back = roundtrip(&c);
        let h = back.prometheus.unwrap().heartbeat.expect("heartbeat present");
        assert_eq!(h.quiet_hours_start, 22);
        assert_eq!(h.quiet_hours_end, 7);
        assert_eq!(h.timezone.as_deref(), Some("America/Los_Angeles"));
        assert!(!h.event_driven_only);
        assert_eq!(h.safety_net_interval_secs, 1800);
    }

    #[test]
    fn prometheus_heartbeat_serde_compat_with_engine_struct() {
        // Critical compat test: zeus-prometheus does
        //   serde_json::from_value::<heartbeat::HeartbeatConfig>(typed_struct)
        // — confirm a typed PrometheusHeartbeatConfig serializes to a JSON
        // shape the engine struct can consume. We mirror the engine field set
        // here without depending on zeus-prometheus.
        let typed = PrometheusHeartbeatConfig {
            quiet_hours_start: 23,
            quiet_hours_end: 8,
            enable_quiet_hours: true,
            timezone: None,
            timeout_secs: 30,
            dedup_window_secs: 86_400,
            active_interval_secs: 120,
            event_driven_only: true,
            safety_net_interval_secs: 3600,
            plan_resume_interval_secs: 3600,
            extra: Default::default(),
        };
        let json = serde_json::to_value(&typed).expect("typed → JSON");
        // Required engine fields all present?
        let obj = json.as_object().expect("JSON object");
        for k in [
            "quiet_hours_start",
            "quiet_hours_end",
            "enable_quiet_hours",
            "timeout_secs",
            "dedup_window_secs",
            "active_interval_secs",
            "event_driven_only",
            "safety_net_interval_secs",
            "plan_resume_interval_secs",
        ] {
            assert!(obj.contains_key(k), "engine-required field missing: {k}");
        }
    }

    #[test]
    fn prometheus_legacy_json_value_still_parses() {
        // Backward-compat: existing config.toml files written before Phase 3.5
        // had `[prometheus.heartbeat]` with TOML primitives. Confirm those
        // still parse into the new typed struct.
        let toml = r#"
model = "anthropic/claude-sonnet-4"

[prometheus]
enable_heartbeat = true

[prometheus.heartbeat]
quiet_hours_start = 22
quiet_hours_end = 6
enable_quiet_hours = false
event_driven_only = false
"#;
        let c: Config = toml::from_str(toml).expect("legacy toml parses");
        let h = c.prometheus.unwrap().heartbeat.expect("heartbeat parsed");
        assert_eq!(h.quiet_hours_start, 22);
        assert_eq!(h.quiet_hours_end, 6);
        assert!(!h.enable_quiet_hours);
        assert!(!h.event_driven_only);
    }

    #[test]
    fn prometheus_unknown_field_tolerated_via_extra() {
        // Fields zeus-prometheus may add later land in `extra` rather than
        // breaking the deserializer. Exercises `#[serde(flatten)] extra`.
        let toml = r#"
[prometheus]
[prometheus.heartbeat]
quiet_hours_start = 23
future_engine_field = 42
another_new_thing = "hello"
"#;
        let c: Config = toml::from_str(toml).expect("permissive parse");
        let h = c.prometheus.unwrap().heartbeat.expect("heartbeat");
        assert!(h.extra.contains_key("future_engine_field"));
        assert!(h.extra.contains_key("another_new_thing"));
    }

    // ---- SACRED .env merge (CLAUDE.md no-clobber incident) ----

    #[test]
    fn merge_env_preserves_all_other_keys() {
        // THE incident: writing the API key must NOT wipe DISCORD_BOT_TOKEN etc.
        let existing = "DISCORD_BOT_TOKEN=abc123\n\
                        DISCORD_RELAY_CHANNEL_IDS=1,2,3\n\
                        TELEGRAM_BOT_TOKEN=xyz\n";
        let out = Config::merge_env_line(existing, "ANTHROPIC_API_KEY", "sk-real");
        assert!(out.contains("DISCORD_BOT_TOKEN=abc123"));
        assert!(out.contains("DISCORD_RELAY_CHANNEL_IDS=1,2,3"));
        assert!(out.contains("TELEGRAM_BOT_TOKEN=xyz"));
        assert!(out.contains("ANTHROPIC_API_KEY=sk-real"));
    }

    #[test]
    fn merge_env_updates_in_place_not_duplicate() {
        // Re-running onboarding updates the key, never appends a 2nd line.
        let existing = "ANTHROPIC_API_KEY=sk-old\nDISCORD_BOT_TOKEN=keep\n";
        let out = Config::merge_env_line(existing, "ANTHROPIC_API_KEY", "sk-new");
        assert!(out.contains("ANTHROPIC_API_KEY=sk-new"));
        assert!(!out.contains("sk-old"));
        assert_eq!(out.matches("ANTHROPIC_API_KEY=").count(), 1);
        assert!(out.contains("DISCORD_BOT_TOKEN=keep"));
    }

    #[test]
    fn merge_env_appends_when_absent() {
        let existing = "DISCORD_BOT_TOKEN=keep\n";
        let out = Config::merge_env_line(existing, "OPENAI_API_KEY", "sk-o");
        assert!(out.contains("DISCORD_BOT_TOKEN=keep"));
        assert!(out.contains("OPENAI_API_KEY=sk-o"));
    }

    #[test]
    fn merge_env_into_empty_creates_single_line() {
        let out = Config::merge_env_line("", "XAI_API_KEY", "sk-x");
        assert_eq!(out, "XAI_API_KEY=sk-x\n");
    }

    #[test]
    fn merge_env_does_not_match_commented_line() {
        // A commented `# ANTHROPIC_API_KEY=...` must be preserved, and the real
        // key appended — never silently activate the comment.
        let existing = "# ANTHROPIC_API_KEY=placeholder\nDISCORD_BOT_TOKEN=keep\n";
        let out = Config::merge_env_line(existing, "ANTHROPIC_API_KEY", "sk-real");
        assert!(out.contains("# ANTHROPIC_API_KEY=placeholder"));
        assert!(out.contains("ANTHROPIC_API_KEY=sk-real"));
        assert!(out.contains("DISCORD_BOT_TOKEN=keep"));
    }

    #[test]
    fn merge_env_respects_export_prefix() {
        let existing = "export ANTHROPIC_API_KEY=sk-old\n";
        let out = Config::merge_env_line(existing, "ANTHROPIC_API_KEY", "sk-new");
        assert!(out.contains("sk-new"));
        assert!(!out.contains("sk-old"));
        assert_eq!(out.matches("ANTHROPIC_API_KEY=").count(), 1);
    }

    #[test]
    fn merge_env_no_partial_key_false_match() {
        // `ANTHROPIC_API_KEY_OLD` must NOT be treated as `ANTHROPIC_API_KEY`.
        let existing = "ANTHROPIC_API_KEY_BACKUP=keepme\n";
        let out = Config::merge_env_line(existing, "ANTHROPIC_API_KEY", "sk-real");
        assert!(out.contains("ANTHROPIC_API_KEY_BACKUP=keepme"));
        assert!(out.contains("ANTHROPIC_API_KEY=sk-real"));
    }

    #[test]
    fn merge_env_empty_key_or_value_is_noop_passthrough() {
        // persist_env_key short-circuits on empty; the transform is only reached
        // with real key+value, but guard the helper's contract directly too.
        let existing = "DISCORD_BOT_TOKEN=keep\n";
        // Empty value would write `KEY=` — persist_env_key guards before here,
        // so we just assert the transform never drops the preserved line.
        let out = Config::merge_env_line(existing, "OPENAI_API_KEY", "v");
        assert!(out.contains("DISCORD_BOT_TOKEN=keep"));
    }

    // ── needs_onboarding() — the 3-condition truth table ───────────────────

    #[test]
    fn needs_onboarding_fresh_default_install_runs_wizard() {
        // No config.toml on disk → loaded_from_default = true, marker unset,
        // model empty. This is the fresh-install case: onboard.
        let cfg = Config { loaded_from_default: true, ..Config::default() };
        assert!(cfg.needs_onboarding(), "fresh default install must onboard");
    }

    #[test]
    fn needs_onboarding_completed_marker_skips_wizard() {
        // #267(b) self-heal: a completed config ALWAYS persists a non-empty model
        // (`provider_id/model_id`), so a set marker with an EMPTY model is a nuked
        // box, not a healthy one — onboard to self-heal even though the stale marker
        // says complete. A healthy completed config (model present) still skips; see
        // `needs_onboarding_completed_marker_with_model_skips_wizard`.
        let cfg = Config {
            onboarding_complete: true,
            loaded_from_default: true,
            model: String::new(),
            ..Config::default()
        };
        assert!(
            cfg.needs_onboarding(),
            "set marker with empty model is a nuked box → onboard to self-heal"
        );
    }

    #[test]
    fn needs_onboarding_completed_marker_with_model_skips_wizard() {
        // Healthy completed config: marker set AND model present → skip. Guards
        // against #267(b) self-heal dragging healthy completed boxes back into
        // onboarding.
        let cfg = Config {
            onboarding_complete: true,
            loaded_from_default: true,
            model: "anthropic/claude-sonnet-4".to_string(),
            ..Config::default()
        };
        assert!(
            !cfg.needs_onboarding(),
            "completed marker with a model present must skip onboarding"
        );
    }

    #[test]
    fn needs_onboarding_legacy_model_set_no_marker_treats_done() {
        // Legacy migration: a real config from disk (loaded_from_default = false)
        // with a model configured but NO onboarding_complete marker (predates the
        // field). Must be treated as done — do NOT re-onboard an existing user.
        let cfg = Config {
            onboarding_complete: false,
            loaded_from_default: false,
            model: "anthropic/claude-opus-4".to_string(),
            ..Config::default()
        };
        assert!(
            !cfg.needs_onboarding(),
            "legacy config with model set must migrate to done, not re-onboard"
        );
    }

    #[test]
    fn needs_onboarding_real_load_no_model_no_marker_runs_wizard() {
        // A config that exists on disk (loaded_from_default = false) but never
        // completed onboarding and has no model → still needs onboarding.
        let cfg = Config {
            onboarding_complete: false,
            loaded_from_default: false,
            model: String::new(),
            ..Config::default()
        };
        assert!(
            cfg.needs_onboarding(),
            "real load with no model and no marker must onboard"
        );
    }

    #[test]
    fn needs_onboarding_marker_wins_over_empty_model() {
        // #267(b) self-heal: empty model wins over a stale marker. A real load
        // (loaded_from_default = false) whose model got nuked must re-onboard even
        // though the marker still says complete — a healthy completed config always
        // persists a non-empty model, so an empty model here flags a nuked box.
        let cfg = Config {
            onboarding_complete: true,
            loaded_from_default: false,
            model: String::new(),
            ..Config::default()
        };
        assert!(
            cfg.needs_onboarding(),
            "empty model wins over a stale marker → onboard to self-heal a nuked box"
        );
    }
}
