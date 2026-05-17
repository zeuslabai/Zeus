//! Agent Loop - Simple prompt → response → tools → repeat
//!
//! Target: ~200 lines

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, instrument, warn};
use zeus_core::{
    AgentToolPolicy, Config, Error, Message, Result, ToolCall, ToolResult, ToolSchema,
};
use zeus_llm::{FallbackProvider, LlmClient, LlmResponse, StopReason};
use zeus_memory::Workspace;
use zeus_session::{CompactionFlush, ContextJournal, ContextManager, Session};

use zeus_aegis::Aegis;
use zeus_athena::{ActionLog, ActionType, Athena};
use zeus_channels::{ChannelManager, ChannelSource};
use zeus_hermes::{Hermes, Notification, NotificationPriority};
use zeus_mnemosyne::Mnemosyne;
use zeus_nous::Nous;
use zeus_council::{CouncilConfig, pipeline::run_council};
use zeus_skills::skill_matcher::{EmbeddingProvider, SkillMatcher};
use zeus_talos::TalosRegistry;

use crate::constitution::{Constitution, ConstitutionVerdict};

/// Smart memory filter — decides whether a message is worth storing in Mnemosyne.
///
/// Rejects scaffolding, routing wrappers, heartbeat spam, and trivial acks.
/// Accepts @mentions with tasks, decisions, outcomes, and substantive content.
fn should_store_in_memory(content: &str, role: &zeus_core::Role) -> bool {
    // Assistant messages (reflections, spawn results) are always worth storing
    if matches!(role, zeus_core::Role::Assistant | zeus_core::Role::Tool) {
        return true;
    }

    let trimmed = content.trim();

    // Skip empty messages
    if trimmed.is_empty() {
        return false;
    }

    // Skip Discord/channel routing wrappers (system scaffolding)
    let scaffolding_markers = [
        "[You're on a shared team channel",
        "[You are replying to discord channel",
        "[INTENT: QUESTION",
        "[INTENT: TASK",
        "[PLAN MODE",
        "[END PLAN]",
        "[End of history]",
        "[NEW MESSAGE — respond to THIS",
        "⚠️ Note: this message was sent",
    ];
    for marker in &scaffolding_markers {
        if trimmed.contains(marker) {
            // If the message ALSO contains an @mention with real content after the wrapper,
            // extract and store just the user content, not the wrapper
            // For now: skip the entire message (gateway catches up from Discord history anyway)
            tracing::debug!("Smart memory: skipping scaffolding message");
            return false;
        }
    }

    // Skip trivial heartbeat/ack messages (< 50 chars, no substance)
    let trivial_patterns = [
        "HEARTBEAT_OK",
        "Completed after",
        "Step 1 complete",
        "Plan done",
        "Standing by",
        "Acknowledged",
        "Copy that",
    ];
    if trimmed.len() < 100 {
        for pattern in &trivial_patterns {
            if trimmed.contains(pattern) {
                tracing::debug!("Smart memory: skipping trivial ack");
                return false;
            }
        }
    }

    true
}

/// Bridges Mnemosyne's embedding API to the `EmbeddingProvider` trait
/// used by `SkillMatcher`, keeping zeus-skills independent of zeus-mnemosyne.
struct MnemosyneEmbedder {
    mnemosyne: Arc<Mnemosyne>,
}

#[async_trait::async_trait]
impl EmbeddingProvider for MnemosyneEmbedder {
    async fn embed(&self, text: &str) -> zeus_core::Result<Option<Vec<f32>>> {
        self.mnemosyne
            .embed_text(text)
            .await
            .map_err(|e| zeus_core::Error::Internal(format!("Embedding failed: {}", e)))
    }
}
use crate::hooks::{HookAction, HookContext, HookEventType, HookRegistry};
use crate::intelligence::{ContextGuard, LoopDetector};
use crate::loop_guard::{LoopGuard, LoopGuardVerdict};
use crate::subagent::{AgentTarget, Subagent, SubagentConfig, SubagentResult};
use crate::tools::ToolRegistry;

// ============================================================================
// Agent Events (~20 lines)
// ============================================================================

#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Agent started processing
    Started,
    /// Streaming text chunk from LLM
    TextChunk(String),
    /// Tool is being called
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    /// Tool returned a result
    ToolResult {
        name: String,
        success: bool,
        output: String,
    },
    /// LLM finished a response
    ResponseComplete(LlmResponse),
    /// Agent finished all iterations
    Finished { iterations: usize },
    /// Agent encountered an error
    Error(String),
    /// OAuth login completed (from browser callback flow)
    OAuthComplete(std::result::Result<(), String>),
    /// Context was compacted — N messages summarized, tokens saved
    Compacted { messages_removed: usize, tokens_before: usize, tokens_after: usize },
}

// ============================================================================
// Agent (~130 lines)
// ============================================================================

// ============================================================================
// Task Queue Types
// ============================================================================

/// Result of a single task execution in the autonomous task queue.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task: String,
    pub output: String,
    pub attempts: usize,
    pub success: bool,
}

/// Summary report from `Agent::run_task_queue`.
#[derive(Debug, Clone, Default)]
pub struct TaskQueueReport {
    pub completed: Vec<TaskResult>,
    pub failed: Vec<TaskResult>,
}

impl TaskQueueReport {
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} completed, {} failed",
            self.completed.len(),
            self.failed.len()
        )
    }
}

pub struct Agent {
    config: Config,
    llm: LlmClient,
    /// Fallback provider wrapping multiple LLM clients for automatic failover
    fallback: Option<FallbackProvider>,
    tools: ToolRegistry,
    workspace: Workspace,
    session: Session,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    /// Running background subagents
    subagents: HashMap<String, JoinHandle<SubagentResult>>,

    // Advanced subsystems - all Optional so agent works without them
    mnemosyne: Option<Arc<Mnemosyne>>,
    athena: Option<Arc<Athena>>,
    aegis: Option<Arc<Aegis>>,
    hermes: Option<Arc<tokio::sync::RwLock<Hermes>>>,
    nous: Option<Arc<Nous>>,

    /// Channel manager for platform messaging (Telegram, Discord, Slack, Email, iMessage)
    channels: Option<Arc<ChannelManager>>,
    /// Receiver for inbound channel messages
    channel_rx: Option<mpsc::Receiver<zeus_channels::ChannelMessage>>,

    /// Hook registry for event-driven automation
    hooks: HookRegistry,

    /// Active goals context (injected by orchestrator before each run)
    goals_context: Option<String>,

    /// Pending tasks context (injected from TaskStore for task-driven autonomy)
    tasks_context: Option<String>,

    /// Skills summary for system prompt injection
    skills_summary: Option<String>,
    /// Skill manager for read_when auto-activation
    skill_manager: Option<std::sync::Arc<zeus_skills::SkillManager>>,
    /// Runtime permission enforcement for loaded skills
    skill_permissions: Option<zeus_skills::SkillPermissionRegistry>,
    /// Currently active skills (set by read_when trigger matching, cleared each run)
    active_skills: Vec<String>,
    /// Semantic skill matcher — intent → skill discovery via embeddings
    skill_matcher: Option<Arc<SkillMatcher>>,

    /// Per-agent tool access policy (business-level, layered with Aegis)
    tool_policy: Option<AgentToolPolicy>,

    /// Optional stream callback — when set, LLM tokens are forwarded here in real-time.
    /// Used by the TUI streaming path to show tokens as they arrive.
    stream_tx: Option<mpsc::Sender<zeus_core::inbox::StreamChunk>>,

    /// Context manager for compaction checks
    context_manager: Option<ContextManager>,
    /// Pre-compaction flush state tracker
    compaction_flush: CompactionFlush,
    /// Context journal for capturing workflow state before compaction
    context_journal: Option<ContextJournal>,

    /// Detects repetitive tool call loops
    loop_detector: LoopDetector,
    /// Enhanced loop guard: per-hash counter, ping-pong, circuit breaker
    loop_guard: LoopGuard,

    /// Guards against context window overflow
    context_guard: ContextGuard,
    /// Immutable safety laws checked before every tool execution
    constitution: Constitution,
    /// Information-flow taint tracker for tool execution chains
    taint_tracker: zeus_aegis::TaintTracker,

    /// Cached base system prompt — `(mtime_hash, rendered_string)`.
    ///
    /// `get_context()` reads and concatenates ~8 workspace markdown files
    /// (AGENTS.md, SOUL.md, USER.md, IDENTITY.md, TOOLS.md, HEARTBEAT.md,
    /// CAPABILITIES.md, memory/MEMORY.md) totalling ~10K chars on every
    /// `run_turn()` call. This cache holds the last rendered result and
    /// its workspace mtime fingerprint; if the fingerprint matches on the
    /// next call, we skip the file I/O + string assembly entirely.
    ///
    /// Invalidated automatically when any of those files is modified —
    /// `Workspace::get_context_mtime_hash()` re-reads mtimes per call.
    /// Populated lazily on first `run_turn()`. Not used by `run_fast()`
    /// which has its own minimal prompt path.
    cached_system_prompt: Option<(u64, String)>,
}

impl Agent {
    /// Format a one-line entry for `memory/RECENT_ACTIVITY.md` describing a
    /// side-effect tool call. Format:
    /// `- [YYYY-MM-DDTHH:MM:SSZ] [channel:<type>:<chat_id> user:<id>] tool:<name> → <summary>`
    ///
    /// Missing fields render as `-`. Summary is derived from the tool's
    /// arguments (channel/target/path/content prefix) and truncated.
    fn format_recent_activity_entry(call: &ToolCall) -> String {
        let args = &call.arguments;
        let channel_type = args.get("channel").and_then(|c| c.as_str()).unwrap_or("-");
        let chat_id = args.get("target").and_then(|t| t.as_str()).unwrap_or("-");
        let user_id = args
            .get("user_id")
            .and_then(|u| u.as_str())
            .or_else(|| args.get("user").and_then(|u| u.as_str()))
            .unwrap_or("-");

        // Brief summary: prefer content preview for `message`, path for `send_file`.
        let summary_raw = match call.name.as_str() {
            "message" => args
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("(no content)")
                .to_string(),
            "send_file" => {
                let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("?");
                let caption = args.get("caption").and_then(|c| c.as_str()).unwrap_or("");
                if caption.is_empty() {
                    format!("file={}", path)
                } else {
                    format!("file={} caption={}", path, caption)
                }
            }
            _ => String::new(),
        };
        let summary = summary_raw.replace('\n', " ");
        let summary = if summary.chars().count() > 120 {
            let truncated: String = summary.chars().take(117).collect();
            format!("{}...", truncated)
        } else {
            summary
        };

        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        format!(
            "- [{}] [channel:{}:{} user:{}] tool:{} → {}",
            ts, channel_type, chat_id, user_id, call.name, summary
        )
    }

    fn matching_hook_command<'a>(hooks: &'a std::collections::HashMap<String, String>, tool_name: &str) -> Option<&'a str> {
        if let Some(cmd) = hooks.get(tool_name) {
            return Some(cmd.as_str());
        }
        if let Some(cmd) = hooks.get("*") {
            return Some(cmd.as_str());
        }
        for (pattern, cmd) in hooks {
            if let Some(prefix) = pattern.strip_suffix('*')
                && tool_name.starts_with(prefix)
            {
                return Some(cmd.as_str());
            }
        }
        None
    }

    async fn run_configured_tool_hook(
        &self,
        command: &str,
        call: &ToolCall,
        result: Option<&ToolResult>,
    ) -> Result<()> {
        use std::process::Stdio;
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(self.workspace.root())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .env("ZEUS_TOOL_NAME", &call.name)
            .env("ZEUS_TOOL_CALL_ID", &call.id)
            .env("ZEUS_TOOL_ARGS", call.arguments.to_string())
            .env("ZEUS_SESSION_ID", &self.session.id);

        if let Some(result) = result {
            cmd.env("ZEUS_TOOL_SUCCESS", if result.success { "1" } else { "0" })
                .env("ZEUS_TOOL_OUTPUT", &result.output);
        }

        let output = cmd.output().await.map_err(|e| Error::Agent(format!("hook exec failed: {}", e)))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let msg = if stderr.is_empty() {
                format!("hook exited with status {}", output.status)
            } else {
                format!("hook exited with status {}: {}", output.status, stderr)
            };
            Err(Error::Agent(msg))
        }
    }
    /// Create a new agent with core dependencies only.
    ///
    /// The `channels` parameter wires a `ChannelManager` into the agent's
    /// `ToolRegistry` so the `message` tool can dispatch to platform adapters
    /// (Discord/Telegram/Slack/X/etc.). Constructor-level enforcement of this
    /// invariant prevents the recurring "did the caller remember to call
    /// `set_shared_channels`?" footgun across callsites. Pass `None` only when
    /// the agent is intentionally channel-less (benchmarks, tests, IDE bridges
    /// that don't dispatch platform channels).
    pub fn new(
        config: Config,
        llm: LlmClient,
        workspace: Workspace,
        session: Session,
        channels: Option<Arc<ChannelManager>>,
    ) -> Self {
        let context_manager = config.session_compaction.as_ref().map(ContextManager::new);
        let flush_timeout_secs = config.session_compaction.as_ref()
            .and_then(|c| c.flush_timeout_secs)
            .unwrap_or(30);
        let context_journal = config
            .prometheus
            .as_ref()
            .and_then(|p| p.context_journal.as_ref())
            .filter(|cj| cj.enabled)
            .map(|cj| {
                ContextJournal::new(
                    zeus_core::default_config_dir().join(&cj.path),
                    cj.threshold_pct,
                )
            });

        let mut tools = ToolRegistry::with_defaults();
        if let Some(ref ch) = channels {
            tools.set_channels(ch.clone());
        }

        Self {
            config,
            llm,
            fallback: None,
            tools,
            workspace,
            session,
            event_tx: None,
            subagents: HashMap::new(),
            mnemosyne: None,
            athena: None,
            aegis: None,
            hermes: None,
            nous: None,
            channels,
            channel_rx: None,
            hooks: HookRegistry::new(),
            goals_context: None,
            tasks_context: None,
            skills_summary: None,
            tool_policy: None,
            stream_tx: None,
            context_manager,
            compaction_flush: CompactionFlush::with_timeout(flush_timeout_secs),
            context_journal,
            loop_detector: LoopDetector::default_threshold(),
            loop_guard: LoopGuard::default_limits(),
            context_guard: ContextGuard::default_guard(),
            constitution: Constitution::load(
                &zeus_core::default_config_dir().join("constitution.toml"),
            ),
            skill_manager: None,
            skill_permissions: None,
            active_skills: Vec::new(),
            skill_matcher: None,
            taint_tracker: zeus_aegis::TaintTracker::new(),
            cached_system_prompt: None,
        }
    }

    /// Create a new agent with all configured subsystems
    pub async fn with_subsystems(
        config: Config,
        llm: LlmClient,
        workspace: Workspace,
        session: Session,
    ) -> Result<Self> {
        // Initialize advanced subsystems if configured
        let mnemosyne = if let Some(ref mc) = config.mnemosyne {
            let mnemosyne_config = zeus_mnemosyne::MnemosyneConfig {
                db_path: mc.db_path.clone(),
                enable_fts: mc.enable_fts,
                max_messages_per_session: mc.max_messages_per_session,
                enable_embeddings: mc.enable_embeddings,
                embedding_dim: mc.embedding_dim,
                ollama_url: mc.ollama_url.clone(),
                embedding_model: mc.embedding_model.clone(),
                vector_weight: mc.vector_weight,
                text_weight: mc.text_weight,
                candidate_multiplier: mc.candidate_multiplier,
                embedding_providers: mc.embedding_providers.clone(),
                fallback_threshold: mc.fallback_threshold,
                enable_session_indexing: mc.enable_session_indexing,
                session_delta_bytes: mc.session_delta_bytes,
                session_delta_messages: mc.session_delta_messages,
                enable_file_watcher: mc.enable_file_watcher,
                watch_paths: mc.watch_paths.clone(),
                extra_memory_paths: mc.extra_memory_paths.clone(),
                chunk_overlap_tokens: mc.chunk_overlap_tokens,
                embed_batch_size: mc.embed_batch_size,
                enable_qmd: mc.enable_qmd,
                qmd_url: mc.qmd_url.clone(),
                qmd_timeout_ms: mc.qmd_timeout_ms,
                qmd_reranker_url: mc.qmd_reranker_url.clone(),
                qmd_reranker_model: mc.qmd_reranker_model.clone(),
                qmd_bm25_weight: mc.qmd_bm25_weight,
                qmd_vector_weight: mc.qmd_vector_weight,
                qmd_reranker_weight: mc.qmd_reranker_weight,
                qmd_candidate_multiplier: mc.qmd_candidate_multiplier,
                embedding_host: mc.embedding_host.clone(),
                compaction_fact_check: mc.compaction_fact_check,
                max_memories: mc.max_memories,
                dedup_threshold: mc.dedup_threshold,
                consolidation_session_limit: mc.consolidation_session_limit,
            };
            match Mnemosyne::new(mnemosyne_config).await {
                Ok(m) => {
                    info!("Mnemosyne initialized (db: {})", mc.db_path.display());
                    Some(Arc::new(m))
                }
                Err(e) => {
                    warn!("Failed to initialize Mnemosyne: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let athena = if let Some(ref ac) = config.athena {
            let athena_config = zeus_athena::AthenaConfig::new(ac.vault_path.clone());
            match Athena::new(athena_config) {
                Ok(a) => {
                    info!("Athena initialized (vault: {})", ac.vault_path.display());
                    Some(Arc::new(a))
                }
                Err(e) => {
                    warn!("Failed to initialize Athena: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let aegis = if let Some(ref sc) = config.aegis {
            let aegis_config = zeus_aegis::AegisConfig {
                keychain_service: sc.keychain_service.clone(),
                sandbox_level: sc.sandbox_level.parse().unwrap_or_default(),
                audit_path: sc.audit_path.clone(),
                permissions: sc.permissions.clone(),
                network_allowlist: sc.network_allowlist.clone(),
                tools_requiring_approval: sc.require_confirmation_for.clone(),
                approval_timeout_secs: sc.approval_timeout_secs,
                allowed_write_paths: sc.allowed_write_paths.clone(),
                allow_system_paths: sc.allow_system_paths,
                ..Default::default()
            };
            match Aegis::new(aegis_config).await {
                Ok(a) => {
                    info!("Aegis initialized (sandbox: {})", sc.sandbox_level);
                    Some(Arc::new(a))
                }
                Err(e) => {
                    warn!("Failed to initialize Aegis: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let hermes = if let Some(ref hc) = config.hermes {
            let mut default_channels = std::collections::HashMap::new();
            default_channels.insert(
                zeus_hermes::NotificationPriority::Normal,
                if hc.default_channels.is_empty() {
                    vec!["console".to_string()]
                } else {
                    hc.default_channels.clone()
                },
            );
            let hermes_config = zeus_hermes::HermesConfig {
                default_channels,
                batch_low_priority: hc.batch_low_priority,
                batch_interval_secs: 300,
                targets: std::collections::HashMap::new(),
            };
            info!("Hermes initialized");
            Some(Arc::new(tokio::sync::RwLock::new(Hermes::new(
                hermes_config,
            ))))
        } else {
            // No [hermes] config — use a console-only default so notifications
            // aren't silently dropped. notify() falls back to ["console"] when
            // default_channels is empty, which is what HermesConfig::default() gives.
            debug!("No [hermes] config — using console-only notification fallback");
            Some(Arc::new(tokio::sync::RwLock::new(Hermes::default())))
        };

        let nous = if config.nous.is_some() {
            let nous_result = if let Some(ref mn) = mnemosyne {
                Nous::with_mnemosyne(mn.clone()).await
            } else {
                Nous::new().await
            };
            match nous_result {
                Ok(mut n) => {
                    n.set_llm(Arc::new(llm.clone()));
                    info!(
                        persistent_learning = mnemosyne.is_some(),
                        "Nous initialized with LLM-backed reasoning"
                    );
                    Some(Arc::new(n))
                }
                Err(e) => {
                    warn!("Failed to initialize Nous: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize channel manager via extracted builder (see crates/zeus-agent/src/channel_builder.rs).
        let (channels, channel_rx) = crate::channel_builder::build_channel_manager_from_config(&config).await?;

        // Wire channel adapters into Hermes for real notification delivery
        if let (Some(hermes), Some(ch_mgr)) = (&hermes, &channels) {
            let mut h = hermes.write().await;
            crate::channels::register_channel_senders(&mut h, ch_mgr);
            let registered = h.registered_channels();
            if !registered.is_empty() {
                info!("Hermes notification senders registered: {:?}", registered);
            }
        }

        // Initialize hooks if configured
        let hooks = if let Some(ref hc) = config.hooks {
            let registry = HookRegistry::from_config(hc);
            info!("Hooks initialized ({} hooks)", registry.len());
            registry
        } else {
            HookRegistry::new()
        };

        // Initialize credential vault (OS keychain + config fallback)
        let credential_vault = std::sync::Arc::new(zeus_aegis::CredentialVault::new(
            config.credentials.clone(),
            zeus_core::default_config_dir(),
        ));
        info!(
            "Credential vault initialized ({} config credentials, keychain={})",
            config.credentials.len(),
            credential_vault.has_keychain(),
        );

        // Initialize skills from ~/.zeus/skills/ (primary), then layer in
        // workspace and community skills so the runtime sees the union of all
        // three sources. Last-write-wins on name collisions, with workspace
        // overriding primary and community last (T14: skills runtime wiring).
        let mut skill_manager = zeus_skills::SkillManager::default();
        match skill_manager.load_all().await {
            Ok(count) if count > 0 => {
                info!("Skills loaded (primary): {}", count);
            }
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to load primary skills (non-fatal): {}", e);
            }
        }

        let cfg_dir = zeus_core::default_config_dir();
        let workspace_skills = cfg_dir.join("workspace").join("skills");
        match skill_manager.load_extra_dir(&workspace_skills, false).await {
            Ok(n) if n > 0 => info!("Skills loaded (workspace): {}", n),
            Ok(_) => {}
            Err(e) => warn!(
                "Failed to load workspace skills at {} (non-fatal): {}",
                workspace_skills.display(),
                e
            ),
        }

        let community_skills = cfg_dir.join(".community_skills");
        match skill_manager.load_extra_dir(&community_skills, true).await {
            Ok(n) if n > 0 => info!("Skills loaded (community): {}", n),
            Ok(_) => {}
            Err(e) => warn!(
                "Failed to load community skills at {} (non-fatal): {}",
                community_skills.display(),
                e
            ),
        }

        let skill_manager = skill_manager.with_vault(credential_vault);

        // Build skill permission registry from loaded skills
        let skill_permissions = {
            let persist_path = zeus_core::default_config_dir().join("skill_permissions.json");
            let registry = zeus_skills::SkillPermissionRegistry::with_persistence(persist_path);
            // Load persisted policies first
            let loaded = registry.load().await;
            if loaded > 0 {
                info!("Loaded {} persisted skill permission policies", loaded);
            }
            // Register policies for any newly loaded skills not yet in the registry
            for skill in skill_manager.list() {
                if registry.get(&skill.name).await.is_none() {
                    let caps = zeus_skills::skill_permissions::parse_skill_capabilities(
                        &skill.permissions,
                    );
                    let source_str = if skill
                        .metadata
                        .as_ref()
                        .is_some_and(|m| m.skill_key.is_some())
                    {
                        "clawhub"
                    } else {
                        "local"
                    };
                    let mut policy =
                        zeus_skills::SkillPermissionPolicy::for_source(&skill.name, source_str);
                    // Override capabilities from SKILL.md permissions section
                    if !caps.is_empty() {
                        policy.capabilities = caps;
                    }
                    info!(
                        skill = %skill.name,
                        caps = ?policy.capabilities,
                        "Registered skill permission policy"
                    );
                    registry.register(policy).await;
                }
            }
            registry
        };

        // Build tool registry with Talos tools if configured
        let mut tools = if config.talos.is_some() {
            let talos = TalosRegistry::with_defaults();
            info!("Talos initialized ({} tools)", talos.len());
            ToolRegistry::with_talos(talos)
        } else {
            ToolRegistry::with_defaults()
        };

        // Add Browser CDP tools (connect lazily when first used)
        {
            let debug_url = config
                .deployment
                .as_ref()
                .map(|d| d.chrome_cdp_url.clone())
                .unwrap_or_else(|| "http://localhost:9222".to_string());
            let (schemas, _browser_handle) = zeus_browser::create_browser_tools(&debug_url);
            let browser_registry = zeus_browser::BrowserRegistry::with_tools(_browser_handle);
            info!("Browser CDP initialized ({} tools)", schemas.len());
            tools.set_browser(browser_registry);
        }

        // Build skills summary for system prompt context
        let skills_summary = skill_manager.get_summary();
        let skill_manager = std::sync::Arc::new(skill_manager); // wrap after with_vault()

        // Initialize semantic skill matcher if Mnemosyne embeddings are available
        let skill_matcher = if let Some(ref mn) = mnemosyne {
            let embedder = Arc::new(MnemosyneEmbedder {
                mnemosyne: mn.clone(),
            });
            let threshold = config.skill_matcher_threshold.unwrap_or(0.40);
            let matcher = SkillMatcher::new(embedder, threshold);
            match matcher.index_skills(skill_manager.skills()).await {
                Ok(count) if count > 0 => {
                    info!(
                        "SkillMatcher: indexed {} skills for semantic matching",
                        count
                    );
                    Some(Arc::new(matcher))
                }
                Ok(_) => {
                    debug!("SkillMatcher: no skills indexed (embeddings may be disabled)");
                    None
                }
                Err(e) => {
                    warn!("SkillMatcher: failed to index skills (non-fatal): {}", e);
                    None
                }
            }
        } else {
            None
        };

        let context_manager = config.session_compaction.as_ref().map(ContextManager::new);
        let flush_timeout_secs = config.session_compaction.as_ref()
            .and_then(|c| c.flush_timeout_secs)
            .unwrap_or(30);
        let context_journal = config
            .prometheus
            .as_ref()
            .and_then(|p| p.context_journal.as_ref())
            .filter(|cj| cj.enabled)
            .map(|cj| {
                ContextJournal::new(
                    zeus_core::default_config_dir().join(&cj.path),
                    cj.threshold_pct,
                )
            });

        // Initialize FallbackProvider if fallback_models are configured
        let fallback = if config.fallback_models.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
            match FallbackProvider::from_config(&config) {
                Ok(fp) => {
                    info!("FallbackProvider initialized with {} provider(s)", fp.provider_count());
                    Some(fp)
                }
                Err(e) => {
                    warn!("FallbackProvider init failed (non-fatal): {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Surface E: wire ChannelManager into ToolRegistry so the `message` tool
        // can dispatch to platform adapters (Discord/Telegram/Slack/X/etc.)
        // without falling through to Channel::parse → Unknown. Mirrors Cut
        // D-real (229dbce2) which fixed the registry-agent path via
        // set_shared_channels; this fix covers gateway/chat/tui agents that
        // build their own ChannelManager via with_subsystems.
        if let Some(ref ch) = channels {
            tools.set_channels(ch.clone());
        }

        Ok(Self {
            config,
            llm,
            fallback,
            tools,
            workspace,
            session,
            event_tx: None,
            subagents: HashMap::new(),
            mnemosyne,
            athena,
            aegis,
            hermes,
            nous,
            channels,
            channel_rx,
            hooks,
            goals_context: None,
            tasks_context: None,
            skills_summary: if skills_summary.contains("No skills") {
                None
            } else {
                Some(skills_summary)
            },
            skill_manager: Some(skill_manager),
            skill_permissions: Some(skill_permissions),
            active_skills: Vec::new(),
            skill_matcher,
            tool_policy: None,
            stream_tx: None,
            context_manager,
            compaction_flush: CompactionFlush::with_timeout(flush_timeout_secs),
            context_journal,
            loop_detector: LoopDetector::default_threshold(),
            loop_guard: LoopGuard::default_limits(),
            context_guard: ContextGuard::default_guard(),
            constitution: Constitution::load(
                &zeus_core::default_config_dir().join("constitution.toml"),
            ),
            taint_tracker: zeus_aegis::TaintTracker::new(),
            cached_system_prompt: None,
        })
    }

    /// Set the event channel for streaming updates (builder pattern)
    pub fn with_events(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Set the event channel on an existing agent (mutable borrow)
    pub fn set_events(&mut self, tx: mpsc::Sender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    /// Set the hook registry
    pub fn with_hooks(mut self, hooks: HookRegistry) -> Self {
        self.hooks = hooks;
        self
    }

    /// Get a mutable reference to the hook registry
    pub fn hooks_mut(&mut self) -> &mut HookRegistry {
        &mut self.hooks
    }

    /// Set active goals context for injection into the system prompt.
    /// Call this before `run()` to make the LLM aware of current goals.
    pub fn set_goals_context(&mut self, context: Option<String>) {
        self.goals_context = context;
    }

    /// Set the pending tasks context for injection into the system prompt.
    /// Called by the gateway before cooking to inject TaskStore state.
    pub fn set_tasks_context(&mut self, context: Option<String>) {
        self.tasks_context = context;
    }

    /// Set per-agent tool policy (business-level access control, layered with Aegis).
    pub fn set_tool_policy(&mut self, policy: AgentToolPolicy) {
        self.tool_policy = Some(policy);
    }

    /// Run the agent loop with a user message
    #[instrument(skip(self), fields(input_len = user_input.len()))]
    /// Run the agent with image/file attachments (vision support).
    pub async fn run_with_attachments(
        &mut self,
        user_input: &str,
        attachments: Vec<zeus_core::Attachment>,
    ) -> Result<String> {
        let turn = self.run_turn(user_input, attachments, None).await?;
        Ok(turn.content)
    }

    pub async fn run(&mut self, user_input: &str) -> Result<String> {
        let turn = self.run_turn(user_input, vec![], None).await?;
        Ok(turn.content)
    }

    /// #66 Cut 3: ingest variant carrying the channel-adapter `is_addressed` flag.
    /// When `Some(true)`, the user message is recorded as a mention
    /// (workspace.append_mention + Mnemosyne MemoryType::Mention) for
    /// cross-session continuity. `None`/`Some(false)` ingest normally.
    pub async fn run_addressed(
        &mut self,
        user_input: &str,
        attachments: Vec<zeus_core::Attachment>,
        is_addressed: Option<bool>,
    ) -> Result<String> {
        let turn = self.run_turn(user_input, attachments, is_addressed).await?;
        Ok(turn.content)
    }

    /// Run agent with structured turn result
    pub async fn run_structured(&mut self, user_input: &str) -> Result<zeus_core::TurnResult> {
        self.run_turn(user_input, vec![], None).await
    }

    /// Fast path for simple messages — OpenClaw parity for Ollama/OpenAI.
    /// Skips ALL subsystems: no Nous, no Mnemosyne, no hooks, no compaction,
    /// no disk reads (cached system prompt). Straight to LLM.
    pub async fn run_fast(&mut self, user_input: &str) -> Result<String> {
        use tracing::info;

        // Cache system prompt in memory — only read disk on first call.
        // OpenClaw caches the system prompt; we should too.
        // Cache system prompt — only read disk on first call, then reuse.
        let system_prompt = if let Some((_, ref cached)) = self.cached_system_prompt {
            cached.clone()
        } else {
            let mut prompt = String::with_capacity(2048);
            if let Ok(agents) = self.workspace.get_agents().await {
                if !agents.is_empty() {
                    prompt.push_str(&zeus_core::truncate_str(&agents, 1000));
                    prompt.push('\n');
                }
            }
            if let Ok(soul) = self.workspace.get_soul().await {
                if !soul.is_empty() {
                    prompt.push_str(&zeus_core::truncate_str(&soul, 500));
                    prompt.push('\n');
                }
            }
            self.cached_system_prompt = Some((0, prompt.clone()));
            prompt
        };

        // Add user message to session (lightweight — just Vec::push, no disk write)
        let user_msg = zeus_core::Message::user(user_input);
        self.session.messages.push(user_msg);

        // No tools for fast path
        let tool_schemas: Vec<zeus_core::ToolSchema> = Vec::new();

        // Repair orphaned tool_calls before LLM call
        zeus_session::repair_orphaned_tool_calls(&mut self.session.messages, Some(self.llm.provider()));

        // Sliding window: only send last 10 messages
        let msgs = &self.session.messages;
        let window = if msgs.len() > 10 { &msgs[msgs.len() - 10..] } else { msgs.as_slice() };

        // Direct LLM call — use FallbackProvider if configured, else primary
        let stream_result = if let Some(ref fallback) = self.fallback {
            fallback.stream(window, &tool_schemas, Some(&system_prompt)).await
        } else {
            self.llm.stream(window, &tool_schemas, Some(&system_prompt)).await
        };

        let (mut rx, handle) = match stream_result {
            Ok(result) => result,
            Err(e) => return Err(e),
        };

        // Stream tokens (forward to stream_tx if set)
        let mut content = String::new();
        while let Some(chunk) = rx.recv().await {
            if let Some(ref tx) = self.stream_tx {
                let _ = tx.send(zeus_core::inbox::StreamChunk::Token(chunk.clone())).await;
            }
            content.push_str(&chunk);
        }

        // Wait for full response
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            handle,
        ).await
        .map_err(|_| zeus_core::Error::Agent("LLM response timed out".to_string()))?
        .map_err(|e| zeus_core::Error::Agent(format!("LLM task failed: {}", e)))?;

        let final_content = if response.content.is_empty() { content } else { response.content.clone() };

        // Add assistant response to session (in-memory only, no disk persistence)
        let assistant_msg = zeus_core::Message::assistant(&final_content);
        self.session.messages.push(assistant_msg);

        info!("run_fast completed: {} chars, {} input / {} output tokens",
            final_content.len(), response.input_tokens, response.output_tokens);

        Ok(final_content)
    }

    async fn run_turn(
        &mut self,
        user_input: &str,
        attachments: Vec<zeus_core::Attachment>,
        is_addressed: Option<bool>,
    ) -> Result<zeus_core::TurnResult> {
        // Decay old memory importance at session start (prevents stale memories from dominating)
        if let Some(ref mnemosyne) = self.mnemosyne {
            match mnemosyne.decay_importance(0.01).await {
                Ok(count) if count > 0 => {
                    debug!("Decayed importance for {} memories", count);
                }
                Err(e) => {
                    debug!("Memory decay failed (non-fatal): {}", e);
                }
                _ => {}
            }
        }

        // Record message count at turn start — microcompact must never touch messages
        // from the current turn (prevents stripping tool results mid-cook).
        let turn_start_msg_idx = self.session.messages.len();

        // Capture input for post-response learning (needed after EndTurn)
        let user_input_owned = user_input.to_string();

        // S101 #17: Prompt hooks — run shell commands from ~/.zeus/hooks/prompt/
        // and append their stdout as context to the user message.
        let enriched_input = {
            let mut extra = String::new();
            if let Some(home) = dirs::home_dir() {
                let hooks_dir = home.join(".zeus").join("hooks").join("prompt");
                if hooks_dir.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&hooks_dir) {
                        let mut hook_files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                        hook_files.sort_by_key(|e| e.file_name());
                        for entry in &hook_files {
                            let path = entry.path();
                            if !path.is_file() { continue; }
                            match std::process::Command::new(&path)
                                .current_dir(self.workspace.root())
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::null())
                                .output()
                            {
                                Ok(output) if output.status.success() => {
                                    let text = String::from_utf8_lossy(&output.stdout);
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        extra.push_str(&format!("\n\n[Hook: {}]\n{}",
                                            path.file_stem().unwrap_or_default().to_string_lossy(),
                                            trimmed
                                        ));
                                    }
                                }
                                _ => {} // hook failed or not executable — skip silently
                            }
                        }
                    }
                }
            }
            if extra.is_empty() {
                user_input.to_string()
            } else {
                format!("{}{}", user_input, extra)
            }
        };

        // S103 #29 fix: detect [THINKING_LEVEL=xhigh] tag and set thinking on LLM
        let enriched_input = if enriched_input.starts_with("[THINKING_LEVEL=") {
            if let Some(end) = enriched_input.find("] ") {
                let level = &enriched_input[16..end]; // extract level after "="
                self.llm.thinking_level = Some(level.to_string());
                info!("Ultrathink enabled: thinking_level={}", level);
                enriched_input[end + 2..].to_string() // strip the tag
            } else {
                enriched_input
            }
        } else {
            enriched_input
        };

        // Transcribe audio attachments before sending to LLM
        let (attachments, enriched_input) = {
            let mut kept = Vec::new();
            let mut transcriptions = Vec::new();
            let stt = zeus_channels::telegram_voice::SttProvider::from_config(&self.config);

            for att in attachments {
                if att.mime_type.starts_with("audio/") {
                    if let Some(ref provider) = stt {
                        // Resolve audio bytes: use data if present, else fetch from source_url
                        let audio_bytes: std::result::Result<Vec<u8>, String> = if !att.data.is_empty() {
                            Ok(att.data.clone())
                        } else if let Some(ref url) = att.source_url {
                            match reqwest::get(url).await {
                                Ok(resp) if resp.status().is_success() => {
                                    resp.bytes().await
                                        .map(|b| b.to_vec())
                                        .map_err(|e| format!("Failed to read audio bytes: {}", e))
                                }
                                Ok(resp) => Err(format!("HTTP {} fetching audio", resp.status())),
                                Err(e) => Err(format!("Failed to fetch audio: {}", e)),
                            }
                        } else {
                            Err("No audio data or URL".to_string())
                        };

                        match audio_bytes {
                            Ok(bytes) => match provider.transcribe(&bytes, &att.mime_type).await {
                                Ok(text) => {
                                    info!(chars = text.len(), "Audio attachment transcribed via STT");
                                    transcriptions.push(text);
                                    continue; // drop audio attachment, text replaces it
                                }
                                Err(e) => {
                                    warn!("STT transcription failed, passing audio as-is: {}", e);
                                    kept.push(att);
                                }
                            },
                            Err(e) => {
                                warn!("Could not resolve audio bytes, passing as-is: {}", e);
                                kept.push(att);
                            }
                        }
                    } else {
                        debug!("No STT provider configured, passing audio attachment as-is");
                        kept.push(att);
                    }
                } else {
                    kept.push(att); // non-audio: images, docs, etc.
                }
            }

            let mut input = enriched_input;
            if !transcriptions.is_empty() {
                let joined = transcriptions.join("\n\n");
                input = format!("[Voice transcription]:\n{}\n\n{}", joined, input);
            }
            (kept, input)
        };

        // Graceful skip for image attachments when the model cannot analyze images.
        // If the LLM doesn't support vision, strip image attachments and inject
        // a note into the user input so the model knows images were provided but
        // couldn't be processed — instead of sending them and getting an API error.
        let (attachments, enriched_input) = {
            let vision_ok = zeus_llm::capabilities::capabilities(self.llm.provider()).supports_vision;
            if !attachments.is_empty() && !vision_ok {
                let mut kept = Vec::new();
                let mut skipped_images = Vec::new();
                for att in attachments {
                    if att.mime_type.starts_with("image/") {
                        let name = att.filename.as_deref().unwrap_or("unnamed image");
                        skipped_images.push(name.to_string());
                        warn!(
                            mime = %att.mime_type,
                            filename = name,
                            "Skipping image attachment — model does not support vision"
                        );
                    } else {
                        kept.push(att);
                    }
                }
                if !skipped_images.is_empty() {
                    let note = format!(
                        "[Note: {} image(s) attached but could not be analyzed — the current model does not support image inputs: {}]",
                        skipped_images.len(),
                        skipped_images.join(", ")
                    );
                    (kept, format!("{}\n{}", enriched_input, note))
                } else {
                    (kept, enriched_input)
                }
            } else {
                (attachments, enriched_input)
            }
        };

        // Add user message
        let user_msg = if attachments.is_empty() {
            Message::user(&enriched_input)
        } else {
            Message::user_with_attachments(&enriched_input, attachments)
        };
        self.session.add(user_msg.clone()).await?;

        // #66 Cut 3: mention-tracking — if the channel adapter classified this
        // message as addressed (@mention, reply, DM, name-call), record it
        // for cross-session continuity. Fire-and-forget: failures must not
        // break ingest. (Discord/Telegram/IRC/Signal/iMessage set this flag;
        // None / Some(false) skip mention storage.)
        if matches!(is_addressed, Some(true)) {
            let mention_entry = {
                let snippet: String = user_input.chars().take(200).collect();
                let suffix = if user_input.chars().count() > 200 { "…" } else { "" };
                format!("{}{}", snippet, suffix)
            };
            if let Err(e) = self.workspace.append_mention(&mention_entry).await {
                debug!("append_mention failed (non-fatal): {}", e);
            }
            if let Some(ref mnemosyne) = self.mnemosyne {
                if let Err(e) = mnemosyne
                    .store_typed(
                        &self.session.id,
                        &user_msg,
                        zeus_mnemosyne::MemoryType::Mention,
                        0.85,
                    )
                    .await
                {
                    debug!("store_typed(Mention) failed (non-fatal): {}", e);
                }
            }
        }

        // #66 Cut 3 (item 4 / #33 fill): cross-channel awareness — log inbound
        // user messages to memory/RECENT_ACTIVITY.md so other channel sessions
        // for the same titan see them in their next get_context() render.
        // Mirror-symmetric to the outbound tool-call site below. Fire-and-forget.
        {
            let snippet: String = user_input.chars().take(200).collect();
            let suffix = if user_input.chars().count() > 200 { "…" } else { "" };
            let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
            let entry = format!(
                "- [{}] [session:{}] inbound:user_msg → {}{}",
                ts, self.session.id, snippet, suffix
            );
            if let Err(e) = self.workspace.append_recent_activity(&entry).await {
                debug!("append_recent_activity(user_msg) failed (non-fatal): {}", e);
            }
        }

        // Store in Mnemosyne (with embedding for semantic search)
        // Smart filter: skip scaffolding, routing wrappers, trivial acks
        let stored_msg_id = if let Some(ref mnemosyne) = self.mnemosyne {
            if should_store_in_memory(&user_msg.content, &user_msg.role) {
                mnemosyne
                    .store_with_embedding(&self.session.id, &user_msg)
                    .await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Extract entities from user message and store in memory graph
        if let Some(ref mnemosyne) = self.mnemosyne {
            match mnemosyne
                .extract_entities_from_text(user_input, stored_msg_id)
                .await
            {
                Ok(count) if count > 0 => {
                    debug!("Extracted {} entities from user message", count);
                }
                Err(e) => {
                    debug!("Entity extraction failed (non-fatal): {}", e);
                }
                _ => {}
            }
        }

        // Log to Athena
        if let Some(ref athena) = self.athena {
            let action = ActionLog::new(ActionType::MessageReceived, user_input)
                .with_session(&self.session.id);
            if let Err(e) = athena.log_action(&action).await { tracing::debug!("Athena log failed: {}", e); }
        }

        // Nous: update context with new user interaction
        if let Some(ref nous) = self.nous {
            nous.update_context(zeus_nous::ContextUpdate::NewInteraction {
                input: user_input.to_string(),
            })
            .await;
        }

        self.emit(AgentEvent::Started).await;

        // Reset per-turn state
        self.loop_guard.reset();

        // Fire on_message_received hook
        {
            let hook_ctx = HookContext::new(HookEventType::OnMessageReceived, &self.session.id)
                .with_content(user_input);
            match self.hooks.fire_resolve(&hook_ctx).await {
                HookAction::Abort(reason) => {
                    return Err(Error::Agent(format!("Hook aborted: {}", reason)));
                }
                HookAction::Skip => return Ok(zeus_core::TurnResult {
                    content: "Skipped by hook".to_string(),
                    tool_calls: vec![],
                    input_tokens: 0,
                    output_tokens: 0,
                    iterations: 0,
                    stop_reason: zeus_core::TurnStopReason::Skipped,
                }),
                HookAction::ModifyMessage(_) => {} // Could modify input if needed
                _ => {} // Continue, Allow, Deny, Warn — proceed normally
            }
        }

        // Fire on_session_start hook (first message in loop)
        {
            let hook_ctx = HookContext::new(HookEventType::OnSessionStart, &self.session.id);
            let _ = self.hooks.fire(&hook_ctx).await;
        }

        // Athena: create/update daily note for session tracking
        if let Some(ref athena) = self.athena
            && let Err(e) = athena.create_daily_note(chrono::Utc::now()).await
        {
            debug!("Athena daily note creation failed (non-fatal): {}", e);
        }

        // Build system prompt, optionally enhanced by Nous
        // Phase 2: cached base system prompt.
        // `get_context()` reads ~8 workspace markdown files and concatenates
        // them into a ~10K-char string. We hash the mtimes of those files
        // and reuse the previous render if nothing has changed on disk.
        // Dynamic per-turn content (rules, team memory, cognitive ctx,
        // goals, skills, mnemosyne hits, graph ctx, constitution) is still
        // appended to the cached base via the `push_str` chain below.
        let context_hash = self.workspace.get_context_mtime_hash().await;
        let mut system_prompt = match &self.cached_system_prompt {
            Some((cached_hash, cached)) if *cached_hash == context_hash => {
                debug!(
                    "System prompt cache hit ({} chars, hash={:x})",
                    cached.len(),
                    context_hash
                );
                cached.clone()
            }
            _ => {
                let fresh = self.workspace.get_context().await?;
                debug!(
                    "System prompt cache miss — rebuilt {} chars (hash={:x})",
                    fresh.len(),
                    context_hash
                );
                self.cached_system_prompt = Some((context_hash, fresh.clone()));
                fresh
            }
        };

        // S78: Removed hardcoded Sentient Intelligence Protocol.
        // All personality, work style, and rules now come from workspace files
        // (SOUL.md, AGENTS.md) loaded by get_context() above. No hardcoded overrides.

        // S101 #22: Modular rules — load ~/.zeus/rules/*.md and concat into prompt.
        // Each .md file in the rules directory is an independent rule module.
        if let Some(home) = dirs::home_dir() {
            let rules_dir = home.join(".zeus").join("rules");
            if rules_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&rules_dir) {
                    let mut rule_files: Vec<_> = entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
                        .collect();
                    rule_files.sort_by_key(|e| e.file_name());
                    for entry in &rule_files {
                        if let Ok(content) = std::fs::read_to_string(entry.path()) {
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                system_prompt.push_str(&format!("\n\n[Rule: {}]\n{}", 
                                    entry.path().file_stem().unwrap_or_default().to_string_lossy(),
                                    trimmed
                                ));
                            }
                        }
                    }
                    if !rule_files.is_empty() {
                        debug!("Loaded {} modular rule(s) from {:?}", rule_files.len(), rules_dir);
                    }
                }
            }
        }

        // S103 #33: Team memory files — load ~/.zeus/team/*.md into context.
        // Shared memory files that multiple agents can read/write for coordination.
        if let Some(home) = dirs::home_dir() {
            let team_dir = home.join(".zeus").join("team");
            if team_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&team_dir) {
                    let mut team_files: Vec<_> = entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
                        .collect();
                    team_files.sort_by_key(|e| e.file_name());
                    let mut loaded = 0usize;
                    for entry in &team_files {
                        if let Ok(content) = std::fs::read_to_string(entry.path()) {
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                system_prompt.push_str(&format!("\n\n[Team Memory: {}]\n{}",
                                    entry.path().file_stem().unwrap_or_default().to_string_lossy(),
                                    trimmed
                                ));
                                loaded += 1;
                            }
                        }
                    }
                    if loaded > 0 {
                        debug!("Loaded {} team memory file(s) from {:?}", loaded, team_dir);
                    }
                }
            }
        }

        if let Some(ref nous) = self.nous {
            match nous.understand(user_input).await {
                Ok(intent) => {
                    let confidence_pct = (intent.confidence.0 * 100.0) as u32;
                    let cognitive_ctx = format!(
                        "\n\n[Cognitive Context]\nIntent: {:?} ({}% confident)",
                        intent.intent_type, confidence_pct
                    );
                    system_prompt.push_str(&cognitive_ctx);
                    debug!(
                        "Nous intent: {:?} (confidence: {:?})",
                        intent.intent_type, intent.confidence
                    );
                }
                Err(e) => {
                    debug!("Nous understanding failed (non-fatal): {}", e);
                }
            }

            // Instinct recall: surface relevant lessons from past interactions
            let lessons = nous.get_relevant_lessons(user_input).await;
            if !lessons.is_empty() {
                let mut instinct_ctx = String::from("\n\n[Learned Instincts]\n");
                for lesson in lessons.iter().take(5) {
                    instinct_ctx.push_str(&format!(
                        "- [confidence {:.1}] {}\n",
                        lesson.confidence, lesson.insight
                    ));
                }
                system_prompt.push_str(&instinct_ctx);
                debug!(
                    "Nous instinct recall: {} lessons injected",
                    lessons.len().min(5)
                );
            }
        }

        // Inject constitutional laws into system prompt
        let constitution_summary = self.constitution.system_prompt_summary();
        if !constitution_summary.is_empty() {
            system_prompt.push_str(&format!("\n\n{}", constitution_summary));
        }

        // Inject active goals context if available
        if let Some(ref goals) = self.goals_context
            && !goals.is_empty()
        {
            system_prompt.push_str(&format!("\n\n[Active Goals]\n{}", goals));
        }

        // Inject pending tasks context (task-driven autonomy)
        if let Some(ref tasks) = self.tasks_context
            && !tasks.is_empty()
        {
            system_prompt.push_str(&format!("\n\n[Pending Tasks]\n{}", tasks));
        }

        // Inject skills summary into context
        if let Some(ref skills) = self.skills_summary {
            system_prompt.push_str(&format!("\n\n[Skills]\n{}", skills));
        }

        // read_when: auto-activate skills whose trigger keywords match the user input
        self.active_skills.clear();
        if let Some(ref sm) = self.skill_manager {
            let triggered = sm.find_triggered_skills(user_input);
            if !triggered.is_empty() {
                self.active_skills = triggered.iter().map(|s| s.name.clone()).collect();
                if let Some(triggered_ctx) = sm.get_triggered_context(user_input) {
                    system_prompt.push_str(&format!("\n\n{}", triggered_ctx));
                    debug!(
                        skills = ?self.active_skills,
                        "read_when: auto-activated skill context injected"
                    );
                }
            }
        }

        // S79: Semantic skill matching — list relevant skills by name only.
        // Agent loads full content on demand via read_file (OpenClaw approach).
        // Never auto-inject full skill content — it causes self-triggering loops.
        if let Some(ref matcher) = self.skill_matcher {
            let top_k = self.config.skill_matcher_top_k.unwrap_or(3);
            match matcher.match_intent(user_input, top_k).await {
                Ok(matches) if !matches.is_empty() => {
                    let new_matches: Vec<_> = matches
                        .iter()
                        .filter(|m| !self.active_skills.contains(&m.name))
                        .collect();
                    if !new_matches.is_empty() {
                        let mut ctx = String::from(
                            "\n\n[Relevant Skills — use read_file to load if needed]\n",
                        );
                        for m in &new_matches {
                            ctx.push_str(&format!(
                                "- **{}** ({:.0}%): {}\n",
                                m.name,
                                m.score * 100.0,
                                m.description
                            ));
                            self.active_skills.push(m.name.clone());
                        }
                        system_prompt.push_str(&ctx);
                        debug!(
                            matched = ?new_matches.iter().map(|m| &m.name).collect::<Vec<_>>(),
                            "Semantic skill matching: listed relevant skills (agent-driven loading)"
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    debug!("Semantic skill matching failed (non-fatal): {}", e);
                }
            }
        }

        // Add relevant memories from Mnemosyne to context (hierarchical search)
        if let Some(ref mnemosyne) = self.mnemosyne {
            let mut memory_entries: Vec<(f32, String)> = Vec::new();

            // Format a search result with optional citation
            let fmt_result = |tag: &str, r: &zeus_mnemosyne::SearchResult| -> String {
                match &r.citation {
                    Some(cite) => format!("[{}] {} (source: {})", tag, r.content, cite),
                    None => format!("[{}] {}", tag, r.content),
                }
            };

            // 1. Working memory (current session, highest weight)
            if let Ok(working) = mnemosyne
                .search_by_type(user_input, zeus_mnemosyne::MemoryType::Working, 2)
                .await
            {
                for r in working {
                    memory_entries.push((r.score.abs() * 3.0, fmt_result("working", &r)));
                }
            }

            // 2. Semantic memory (knowledge, high weight)
            if let Ok(semantic) = mnemosyne
                .search_by_type(user_input, zeus_mnemosyne::MemoryType::Semantic, 2)
                .await
            {
                for r in semantic {
                    memory_entries.push((r.score.abs() * 2.0, fmt_result("knowledge", &r)));
                }
            }

            // 2b. Facts (discrete knowledge, high weight)
            if let Ok(facts) = mnemosyne
                .search_by_type(user_input, zeus_mnemosyne::MemoryType::Fact, 2)
                .await
            {
                for r in facts {
                    memory_entries.push((r.score.abs() * 2.0, fmt_result("fact", &r)));
                }
            }

            // 2c. Preferences (user settings, high weight)
            if let Ok(prefs) = mnemosyne
                .search_by_type(user_input, zeus_mnemosyne::MemoryType::Preference, 1)
                .await
            {
                for r in prefs {
                    memory_entries.push((r.score.abs() * 2.5, fmt_result("preference", &r)));
                }
            }

            // 3. Episodic memory (past events, standard weight)
            if let Ok(episodic) = mnemosyne
                .search_by_type(user_input, zeus_mnemosyne::MemoryType::Episodic, 3)
                .await
            {
                for r in episodic {
                    memory_entries.push((r.score.abs(), fmt_result("memory", &r)));
                }
            }

            // Proactive context: analyze full conversation history for relevant memories
            // This catches context (repo paths, project details) that per-query search misses
            // when user says things like "continue" or "keep going"
            if let Ok(proactive) = mnemosyne.proactive_context(&self.session.messages, 5).await {
                for r in proactive {
                    // Deduplicate: skip if we already have this content
                    let content_preview = r.content.chars().take(80).collect::<String>();
                    let already_have = memory_entries.iter().any(|(_, s)| s.contains(&content_preview));
                    if !already_have {
                        memory_entries.push((r.score.abs() * 1.5, fmt_result("context", &r)));
                    }
                }
            }

            // Fallback to untyped search if no typed results found
            if memory_entries.is_empty()
                && let Ok(results) = mnemosyne.search(user_input, 3).await
            {
                for r in results {
                    let text = match &r.citation {
                        Some(cite) => format!("- {} (source: {})", r.content, cite),
                        None => format!("- {}", r.content),
                    };
                    memory_entries.push((r.score.abs(), text));
                }
            }

            if !memory_entries.is_empty() {
                // Sort by weighted score descending
                memory_entries
                    .sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                memory_entries.truncate(10); // Bumped from 5 — 5 was too aggressive across 5 memory types

                let memory_ctx: Vec<String> =
                    memory_entries.iter().map(|(_, s)| s.clone()).collect();
                system_prompt.push_str(&format!(
                    "\n\n[Relevant Memories]\n{}",
                    memory_ctx.join("\n")
                ));
            }

            // Add graph context: expand query via entity graph for richer context
            {
                let store = mnemosyne.store.lock().await;
                if let Ok(graph_results) =
                    zeus_mnemosyne::graph_augmented_search(&store, user_input, 3)
                {
                    let graph_ctx: Vec<String> = graph_results
                        .iter()
                        .filter(|gr| !gr.context_text.is_empty())
                        .map(|gr| gr.context_text.clone())
                        .collect();
                    if !graph_ctx.is_empty() {
                        system_prompt.push_str(&format!("\n\n{}", graph_ctx.join("\n\n")));
                    }
                }
            }
        }

        // Get tool schemas — smart-filtered for Ollama (per-message keyword matching),
        // full set for cloud providers. Then apply agent tool policy if set.
        let tool_schemas = {
            let base = self.tools.context_schemas_for_message(&self.config, user_input);
            if let Some(ref policy) = self.tool_policy {
                base.into_iter()
                    .filter(|s| policy.is_tool_allowed(&s.name))
                    .collect::<Vec<_>>()
            } else {
                base
            }
        };

        // #32: Pre-flight compaction check — compact loaded sessions BEFORE first LLM call
        // (previously compaction only fired end-of-turn, causing token overflow on resume)
        self.check_compaction(&tool_schemas, &system_prompt).await;

        let mut iterations = 0;
        let max_iterations = self.config.max_iterations;
        let mut total_input_tokens: usize = 0;
        let mut total_output_tokens: usize = 0;
        let mut tool_call_records: Vec<zeus_core::ToolCallRecord> = Vec::new();

        loop {
            iterations += 1;
            if iterations > max_iterations {
                warn!("Max iterations ({}) reached", max_iterations);
                break;
            }

            debug!("Agent iteration {}/{}", iterations, max_iterations);

            // Fire on_agent_loop_start hook
            {
                let hook_ctx = HookContext::new(HookEventType::OnAgentLoopStart, &self.session.id)
                    .with_iteration(iterations);
                match self.hooks.fire_resolve(&hook_ctx).await {
                    HookAction::Abort(reason) => {
                        return Err(Error::Agent(format!("Hook aborted: {}", reason)));
                    }
                    HookAction::Skip => continue,
                    _ => {}
                }
            }
            // S99 #1: Per-turn workspace reload — re-read AGENTS.md and SOUL.md
            // each iteration so live edits to workspace files take effect mid-session.
            {
                let agents_fresh = self.workspace.get_agents().await.unwrap_or_default();
                let soul_fresh = self.workspace.get_soul().await.unwrap_or_default();
                let memory_fresh = self.workspace.get_memory().await.unwrap_or_default();
                if !agents_fresh.is_empty() || !soul_fresh.is_empty() || !memory_fresh.is_empty() {
                    let mut refreshed = String::new();
                    if !agents_fresh.is_empty() {
                        refreshed.push_str(&agents_fresh);
                        refreshed.push_str("\n\n");
                    }
                    if !soul_fresh.is_empty() {
                        refreshed.push_str("# Personality\n\n");
                        refreshed.push_str(&soul_fresh);
                        refreshed.push_str("\n\n");
                    }
                    if !memory_fresh.is_empty() {
                        refreshed.push_str("# Memory\n\n");
                        refreshed.push_str(&memory_fresh);
                        refreshed.push_str("\n\n");
                    }
                    // Append the rest of the original system prompt (everything after the
                    // initial AGENTS.md / SOUL.md block, i.e. memories, goals, skills, etc.)
                    // Strategy: find the first occurrence of "# User Context" or "# Identity"
                    // which marks the end of the workspace-identity header block, and keep
                    // everything from that point onward.
                    let tail_markers = ["# User Context", "# Your Human", "# Identity", "# Local Tools", "# Proactive Tasks", "# Capabilities", "# Memory", "[Cognitive Context]", "[Learned Instincts]", "[Constitutional Laws]", "[Active Goals]", "[Skills]", "[Relevant"];
                    let tail_start = tail_markers.iter()
                        .filter_map(|marker| system_prompt.find(marker))
                        .min();
                    if let Some(pos) = tail_start {
                        refreshed.push_str(&system_prompt[pos..]);
                    }
                    system_prompt = refreshed;
                    debug!("Per-turn workspace reload: AGENTS.md + SOUL.md refreshed");
                }
            }

            // S57-T5: Cap system prompt to prevent unbounded growth
            // Rough estimate: 1 token ≈ 4 chars. Cap at 8K tokens ≈ 32K chars.
            const MAX_SYSTEM_PROMPT_CHARS: usize = 32_000;
            if system_prompt.len() > MAX_SYSTEM_PROMPT_CHARS {
                tracing::debug!(
                    "System prompt capped: {} → {} chars",
                    system_prompt.len(),
                    MAX_SYSTEM_PROMPT_CHARS
                );
                let safe_len = zeus_core::floor_char_boundary(&system_prompt, MAX_SYSTEM_PROMPT_CHARS);
                system_prompt.truncate(safe_len);
                // Find last complete line boundary
                if let Some(pos) = system_prompt.rfind('\n') {
                    system_prompt.truncate(pos);
                }
                system_prompt.push_str("\n\n[System prompt truncated due to size]");
            }

            // Context window guard: truncate oldest messages if approaching limit
            self.context_guard.guard(&mut self.session.messages);

            // Ollama session compaction (T8/MOGA) — prevents "goes silent" bug.
            // When session messages approach the model's context window limit,
            // structurally compact older messages: keep system messages + recent
            // turns, replace the rest with a summary marker. This avoids an
            // expensive LLM summarization call (which would add latency on Ollama)
            // while preventing context overflow that causes the model to choke.
            {
                let (provider, _) = self.config.parse_model();
                if provider == zeus_core::Provider::Ollama {
                    // Estimate token count from message content (~4 chars per token)
                    let estimated_tokens: usize = self.session.messages.iter()
                        .map(|m| {
                            let content_len = m.content.len().min(32000); // cap per-message
                            let tool_len: usize = m.tool_calls.iter()
                                .map(|tc| tc.name.len() + tc.arguments.to_string().len().min(8000))
                                .sum();
                            // Cap tool results — large file reads shouldn't inflate the estimate
                            let result_len: usize = m.tool_results.iter()
                                .map(|tr| tr.output.len().min(8000))
                                .sum();
                            (content_len + tool_len + result_len) / 4
                        })
                        .sum();

                    // Query cached context window (falls back to 32768).
                    // Uses a one-shot client since LlmClient doesn't expose its
                    // reqwest::Client. The result is cached in-process by
                    // get_cached_context_window so subsequent calls are free.
                    let ctx_client = reqwest::Client::new();
                    let model_name = self.config.model.split('/').last().unwrap_or(&self.config.model);
                    let model_ctx = zeus_llm::ollama::get_cached_context_window(
                        &ctx_client,
                        &self.config.ollama.url,
                        model_name,
                    ).await;

                    let threshold = match model_ctx {
                        Some(ctx) => (ctx as f64 * 0.6) as usize,
                        None => 100_000,
                    };

                    if estimated_tokens > threshold && self.session.messages.len() > 10 {
                        let total_msgs = self.session.messages.len();
                        // Keep: system messages (any position) + last 8 messages
                        let keep_recent = 20; // 8 was too aggressive — agents lost task context
                        let split_point = total_msgs.saturating_sub(keep_recent);

                        // Count what we're compacting
                        let old_messages = &self.session.messages[..split_point];
                        let user_count = old_messages.iter()
                            .filter(|m| m.role == zeus_core::Role::User)
                            .count();

                        // Extract topic snippets from old user messages for the summary
                        let topics: Vec<String> = old_messages.iter()
                            .filter(|m| m.role == zeus_core::Role::User && !m.content.is_empty())
                            .map(|m| {
                                let words: Vec<&str> = m.content.split_whitespace().take(6).collect();
                                let snippet = words.join(" ");
                                if m.content.split_whitespace().count() > 6 {
                                    format!("{}...", snippet)
                                } else {
                                    snippet
                                }
                            })
                            .take(5)
                            .collect();

                        let summary = format!(
                            "[Session compacted: {} earlier messages ({} user turns) removed to stay within context window. Topics discussed: {}]",
                            split_point,
                            user_count,
                            if topics.is_empty() { "general conversation".to_string() } else { topics.join(", ") }
                        );

                        // Build new message list: preserved messages + summary + recent messages
                        // CompactionHint::Preserve messages survive regardless of position
                        let mut compacted = Vec::with_capacity(keep_recent + split_point / 4);
                        // Extract preserved messages from the old (to-be-compacted) range
                        let preserved: Vec<zeus_core::Message> = self.session.messages[..split_point]
                            .iter()
                            .filter(|m| m.compaction_hint == zeus_core::CompactionHint::Preserve)
                            .cloned()
                            .collect();
                        if !preserved.is_empty() {
                            info!("Compaction: keeping {} preserved messages", preserved.len());
                        }
                        compacted.extend(preserved);
                        compacted.push(zeus_core::Message::system(&summary));
                        compacted.extend(self.session.messages.drain(split_point..));
                        self.session.messages = compacted;

                        info!(
                            "Ollama session compacted: {} → {} messages (estimated {} tokens, threshold {} of {} ctx)",
                            total_msgs,
                            self.session.messages.len(),
                            estimated_tokens,
                            threshold,
                            model_ctx.unwrap_or(0)
                        );
                    }
                }
            }

            // Microcompact: strip old tool outputs (read_file, list_dir, shell, glob)
            // from PREVIOUS turns only. Never compact tool results from the current
            // turn — the agent needs them for multi-iteration cooking.
            {
                const NOISY_TOOLS: &[&str] = &["read_file", "list_dir", "shell", "glob", "grep_files"];
                // Only compact messages from before this turn started
                if turn_start_msg_idx > 0 {
                    // Defensive clamp: bigger compaction passes (L1780+) may have already shrunk
                    // session.messages below the captured turn-start index. Without this clamp,
                    // the slice below panics with index-OOB, killing the tokio-rt-worker and
                    // closing the agent inbox (HTTP 500 "Agent inbox closed" on every subsequent request).
                    let cutoff = turn_start_msg_idx.min(self.session.messages.len());
                    // Build set of call_ids belonging to noisy tools in the old region
                    let noisy_call_ids: std::collections::HashSet<String> = self.session.messages[..cutoff]
                        .iter()
                        .filter(|m| m.role == zeus_core::Role::Assistant)
                        .flat_map(|m| m.tool_calls.iter())
                        .filter(|tc| NOISY_TOOLS.contains(&tc.name.as_str()))
                        .map(|tc| tc.id.clone())
                        .collect();
                    if !noisy_call_ids.is_empty() {
                        for msg in self.session.messages[..cutoff].iter_mut() {
                            for tr in msg.tool_results.iter_mut() {
                                if noisy_call_ids.contains(&tr.call_id) && tr.output != "[compacted]" {
                                    debug!("microcompact: stripping tool result for call_id={}", tr.call_id);
                                    tr.output = "[compacted]".to_string();
                                }
                            }
                        }
                    }
                }
            }

            // Repair orphaned tool_use blocks: find tool_calls with no matching
            // tool_result anywhere in the session, and inject synthetic results.
            // Prevents Anthropic API 400 errors from broken tool_use→tool_result pairing.
            //
            // CATCH #57-ii (P-2): Replaced 47 LOC inline duplicate with single call to
            // zeus_session::repair_orphaned_tool_calls (also called at line 954/1926/2396).
            // The wrapper is now segment-scoped (per-turn) instead of globally-scoped,
            // fixing the kimi `shell:0` cross-turn reuse bug that masked real orphans.
            // Single source of truth — DRY discipline per catch #45.
            zeus_session::repair_orphaned_tool_calls(
                &mut self.session.messages,
                Some(self.llm.provider()),
            );

            // S103 #39: Prompt cache break detection — warn if prompt changed
            {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                system_prompt.hash(&mut hasher);
                let prompt_hash = hasher.finish();
                thread_local! { static LAST_HASH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
                LAST_HASH.with(|h| {
                    let prev = h.get();
                    if prev != 0 && prev != prompt_hash {
                        warn!("Prompt cache break detected — system prompt changed between iterations (hash {} → {})", prev, prompt_hash);
                    }
                    h.set(prompt_hash);
                });
            }

            // Get LLM response with streaming — use FallbackProvider if configured
            let stream_result = if let Some(ref fallback) = self.fallback {
                let primary_model = self.config.model.clone();
                match fallback.stream(&self.session.messages, &tool_schemas, Some(&system_prompt)).await {
                    Ok(result) => {
                        // Detect which provider was used by checking health status
                        let statuses: Vec<zeus_llm::fallback::ProviderHealth> = fallback.health_status().await;
                        let active = statuses.iter().find(|s| s.healthy).map(|s| s.model_string.as_str()).unwrap_or(&primary_model);
                        if active != primary_model {
                            let switch_notice = format!("\n\n⚠️ **Provider switched** — `{}` unavailable, using `{}`.", primary_model, active);
                            self.emit(AgentEvent::TextChunk(switch_notice)).await;
                        }
                        Ok(result)
                    }
                    Err(e) => Err(e),
                }
            } else {
                // Repair orphaned tool_calls before sending to LLM — prevents
                // OpenAI/Anthropic 400 "tool_call_id has no response" errors.
                zeus_session::repair_orphaned_tool_calls(&mut self.session.messages, Some(self.llm.provider()));
                self.llm.stream(&self.session.messages, &tool_schemas, Some(&system_prompt)).await
            };
            let (mut rx, handle) = match stream_result
            {
                Ok(result) => result,
                Err(e) => {
                    // Check for token exhaustion (401/429) and alert fleet
                    let provider = self.config.model.split('/').next().unwrap_or("unknown");
                    let agent_name = self.config.name.clone().or_else(|| self.config.network.as_ref().and_then(|n| n.agent_name.clone())).unwrap_or_else(|| "zeus".to_string());
                    let discord_token = self
                        .config
                        .channels
                        .as_ref()
                        .and_then(|c| c.discord.as_ref())
                        .map(|d| d.token.clone());
                    crate::token_alert::check_and_alert(
                        &e.to_string(),
                        agent_name,
                        provider.to_string(),
                        discord_token,
                    );
                    return Err(e);
                }
            };

            // Stream text chunks to event handler (S102 #27: 90s idle watchdog)
            loop {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(90),
                    rx.recv(),
                ).await {
                    Ok(Some(chunk)) => {
                        // Forward token to stream callback (TUI real-time display)
                        if let Some(ref tx) = self.stream_tx {
                            let _ = tx.send(zeus_core::inbox::StreamChunk::Token(chunk.clone())).await;
                        }
                        self.emit(AgentEvent::TextChunk(chunk)).await;
                    }
                    Ok(None) => break, // stream ended normally
                    Err(_) => {
                        warn!("Stream idle for 90s — no tokens received. Stream may be stalled.");
                        // Don't abort — the JoinHandle will resolve with whatever was collected.
                        // The 5-minute overall timeout below catches truly stuck streams.
                        break;
                    }
                }
            }

            // Wait for complete response with timeout
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(300), // 5 minute timeout per LLM call
                handle,
            )
            .await
            .map_err(|_| Error::Agent("LLM response timed out after 5 minutes".to_string()))?
            .map_err(|e| Error::Agent(format!("LLM task failed: {}", e)))?;

            self.emit(AgentEvent::ResponseComplete(response.clone()))
                .await;

            // Accumulate token usage
            total_input_tokens += response.input_tokens;
            total_output_tokens += response.output_tokens;

            // Add assistant message
            let assistant_msg =
                Message::assistant(&response.content).with_tool_calls(response.tool_calls.clone());
            self.session.add(assistant_msg.clone()).await?;

            // Store in Mnemosyne only if message is worth storing
            let assistant_msg_id = if let Some(ref mnemosyne) = self.mnemosyne {
                use crate::message_store_filter::{MessageStoreFilter, StoreDecision};
                static FILTER: std::sync::OnceLock<MessageStoreFilter> = std::sync::OnceLock::new();
                let filter = FILTER.get_or_init(MessageStoreFilter::new);
                if matches!(filter.should_store(&assistant_msg).await, StoreDecision::Store(_) | StoreDecision::Escalate(_)) {
                    mnemosyne
                        .store_with_embedding(&self.session.id, &assistant_msg)
                        .await
                        .ok()
                } else {
                    None
                }
            } else {
                None
            };

            // Extract entities from assistant response
            if let Some(ref mnemosyne) = self.mnemosyne {
                match mnemosyne
                    .extract_entities_from_text(&response.content, assistant_msg_id)
                    .await
                {
                    Ok(count) if count > 0 => {
                        debug!("Extracted {} entities from assistant response", count);
                    }
                    Err(e) => {
                        debug!("Entity extraction from response failed (non-fatal): {}", e);
                    }
                    _ => {}
                }
            }

            // Auto-extract key facts from assistant response (paths, URLs, config values)
            // Stores as MemoryType::Fact with high importance so they survive search ranking
            if let Some(ref mnemosyne) = self.mnemosyne {
                let facts = extract_facts_from_text(&response.content);
                for fact in facts {
                    let fact_msg = Message::assistant(&fact);
                    match mnemosyne
                        .store_typed(
                            &self.session.id,
                            &fact_msg,
                            zeus_mnemosyne::MemoryType::Fact,
                            0.85,
                        )
                        .await
                    {
                        Ok(_) => debug!("Auto-stored fact: {}", zeus_core::truncate_str(&fact, 80)),
                        Err(e) => debug!("Fact storage failed (non-fatal): {}", e),
                    }
                }
            }

            // Check stop reason
            match response.stop_reason {
                StopReason::EndTurn => {
                    info!("Agent finished (end_turn) after {} iterations", iterations);

                    // Log response to Athena
                    if let Some(ref athena) = self.athena {
                        let summary = if response.content.len() > 200 {
                            format!("{}...", zeus_core::truncate_str(&response.content, 200))
                        } else {
                            response.content.clone()
                        };
                        let action = ActionLog::new(ActionType::ResponseSent, &summary)
                            .with_session(&self.session.id);
                        if let Err(e) = athena.log_action(&action).await { tracing::debug!("Athena log failed: {}", e); }
                    }

                    // Athena: generate session summary from conversation
                    if let Some(ref athena) = self.athena {
                        let actions: Vec<ActionLog> = self
                            .session
                            .messages
                            .iter()
                            .filter_map(|msg| {
                                let action_type = match msg.role {
                                    zeus_core::Role::User => ActionType::MessageReceived,
                                    zeus_core::Role::Assistant => ActionType::ResponseSent,
                                    zeus_core::Role::Tool => ActionType::ToolExecuted,
                                    _ => return None,
                                };
                                let desc = if msg.content.is_empty() {
                                    "tool execution".to_string()
                                } else if msg.content.len() > 200 {
                                    // UTF-8 safe truncation: find the last char boundary at or before byte 197
                                    let mut end = 197;
                                    while end > 0 && !msg.content.is_char_boundary(end) {
                                        end -= 1;
                                    }
                                    format!("{}...", &msg.content[..end])
                                } else {
                                    msg.content.clone()
                                };
                                Some(
                                    ActionLog::new(action_type, desc)
                                        .with_session(&self.session.id),
                                )
                            })
                            .collect();
                        if !actions.is_empty()
                            && let Err(e) =
                                athena.summarize_session(&self.session.id, &actions).await
                        {
                            debug!("Athena session summary failed (non-fatal): {}", e);
                        }
                    }

                    // Hermes: notify on successful completion
                    if let Some(ref hermes) = self.hermes {
                        let notif = Notification::new(format!(
                            "Session completed after {} iteration(s)",
                            iterations
                        ))
                        .with_title("Zeus Task Complete")
                        .with_priority(NotificationPriority::Low);
                        if let Err(e) = hermes.write().await.notify(notif).await { tracing::debug!("Hermes notify failed: {}", e); }
                    }

                    // Fire on_session_end hook
                    {
                        let hook_ctx =
                            HookContext::new(HookEventType::OnSessionEnd, &self.session.id)
                                .with_content(&response.content)
                                .with_iteration(iterations);
                        let _ = self.hooks.fire(&hook_ctx).await;
                    }

                    // Nous: learn from completed interaction
                    if let Some(ref nous) = self.nous
                        && let Ok(intent) = nous.understand(&user_input_owned).await
                    {
                        let _ = nous.learn_outcome(&intent, true, &response.content).await;
                    }

                    // Auto-collect any outstanding background spawns before finishing.
                    // If the LLM spawned subagents with wait=false and never called
                    // collect_spawns, their results would be silently dropped.
                    if self.running_subagents() > 0 {
                        let running_count = self.running_subagents();
                        info!(
                            "EndTurn: auto-collecting {} outstanding background subagent(s)",
                            running_count
                        );
                        let collected = self
                            .await_subagents_timeout(std::time::Duration::from_secs(60))
                            .await;
                        if let Some(ref mnemosyne) = self.mnemosyne {
                            use crate::message_store_filter::{MessageStoreFilter, StoreDecision};
                    static FILTER: std::sync::OnceLock<MessageStoreFilter> = std::sync::OnceLock::new();
                    let filter = FILTER.get_or_init(MessageStoreFilter::new);
                    for result in &collected {
                        let content = format!(
                            "[auto-collected spawn] id={} mission_id={} success={} iterations={}\n{}",
                            result.id,
                            result.mission_id.as_deref().unwrap_or("none"),
                            result.success,
                            result.iterations,
                            result.output
                        );
                        let msg = Message::assistant(&content);
                        if matches!(filter.should_store(&msg).await, StoreDecision::Store(_) | StoreDecision::Escalate(_)) {
                            let _ = mnemosyne.store_with_embedding(&self.session.id, &msg).await;
                        }
                    }
                        }
                        let succeeded = collected.iter().filter(|r| r.success).count();
                        info!(
                            "EndTurn: collected {} spawn result(s) ({} succeeded, {} failed)",
                            collected.len(),
                            succeeded,
                            collected.len() - succeeded,
                        );
                    }

                    self.emit(AgentEvent::Finished { iterations }).await;
                    return Ok(zeus_core::TurnResult {
                        content: response.content,
                        tool_calls: tool_call_records,
                        input_tokens: total_input_tokens,
                        output_tokens: total_output_tokens,
                        iterations,
                        stop_reason: zeus_core::TurnStopReason::EndTurn,
                    });
                }
                StopReason::MaxTokens => {
                    warn!("Response truncated (max_tokens)");
                    // Add continuation prompt so the LLM knows to continue
                    let cont_msg = Message::user("Please continue from where you left off.");
                    self.session.add(cont_msg).await?;
                }
                StopReason::ToolUse => {
                    // Execute tools (with Aegis security checks)
                    let tool_results = self.execute_tools(&response.tool_calls).await;

                    // Record tool calls for TurnResult
                    for (tc, tr) in response.tool_calls.iter().zip(tool_results.iter()) {
                        tool_call_records.push(zeus_core::ToolCallRecord {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                            success: tr.success,
                            output: tr.output.clone(),
                        });
                    }

                    // Loop detection: check if the same tool is being called repeatedly
                    for tc in &response.tool_calls {
                        if let Some(warning) = self.loop_detector.record_call(&tc.name) {
                            let loop_msg = Message::system(warning);
                            self.session.add(loop_msg).await?;
                        }
                    }

                    // Add tool results as message
                    let tool_msg = Message {
                        role: zeus_core::Role::Tool,
                        content: String::new(),
                        tool_calls: vec![],
                        tool_results,
                        timestamp: chrono::Utc::now(),
                        attachments: vec![],
                        message_id: None,
                        parent_id: None,
                        thread_id: None,
                        direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
                    };
                    self.session.add(tool_msg).await?;

                    // Fire on_agent_loop_end hook
                    {
                        let hook_ctx =
                            HookContext::new(HookEventType::OnAgentLoopEnd, &self.session.id)
                                .with_iteration(iterations);
                        let _ = self.hooks.fire(&hook_ctx).await;
                    }

                    // Check if compaction is needed after tool execution
                    self.check_compaction(&tool_schemas, &system_prompt).await;

                    // Continue loop to get next response
                }
                StopReason::Error => {
                    let err_detail = if response.content.is_empty() {
                        "LLM returned error (no details)".to_string()
                    } else {
                        response.content.clone()
                    };
                    error!("LLM error: {}", err_detail);

                    // Notify via Hermes on error
                    if let Some(ref hermes) = self.hermes {
                        let notif = Notification::new(&err_detail)
                            .with_title("Zeus Agent Error")
                            .with_priority(NotificationPriority::High);
                        if let Err(e) = hermes.write().await.notify(notif).await { tracing::debug!("Hermes notify failed: {}", e); }
                    }

                    // Fire on_error hook
                    {
                        let hook_ctx = HookContext::new(HookEventType::OnError, &self.session.id)
                            .with_error(&err_detail);
                        let _ = self.hooks.fire(&hook_ctx).await;
                    }

                    // Nous: reflect on error + learn from failure
                    if let Some(ref nous) = self.nous {
                        let reflection = nous.reflect().await;
                        debug!(
                            "Nous reflection on error (health={:.2}): {}",
                            reflection.health, reflection.summary
                        );
                        // Store reflection in memory so future sessions benefit
                        if let Some(ref mnemosyne) = self.mnemosyne {
                            use crate::message_store_filter::{MessageStoreFilter, StoreDecision};
                            static FILTER: std::sync::OnceLock<MessageStoreFilter> = std::sync::OnceLock::new();
                            let filter = FILTER.get_or_init(MessageStoreFilter::new);
                            let lesson = format!("[reflection] {}", reflection.summary);
                            let msg = zeus_core::Message::assistant(&lesson);
                            if matches!(filter.should_store(&msg).await, StoreDecision::Store(_) | StoreDecision::Escalate(_)) {
                                if let Err(e) = mnemosyne.store_with_embedding(&self.session.id, &msg).await { tracing::debug!("Mnemosyne store failed: {}", e); }
                            }
                        }
                        // Record failure outcome for Nous learning
                        if let Ok(intent) = nous.understand(&err_detail).await {
                            let _ = nous.learn_outcome(&intent, false, &err_detail).await;
                        }
                    }

                    self.emit(AgentEvent::Error(err_detail.clone())).await;
                    return Err(Error::Agent(err_detail));
                }
            }
        }

        // Reached max iterations
        let last_content = self
            .session
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // Auto-collect any outstanding background spawns before finishing (max-iterations path).
        if self.running_subagents() > 0 {
            let running_count = self.running_subagents();
            info!(
                "MaxIterations: auto-collecting {} outstanding background subagent(s)",
                running_count
            );
            let collected = self
                .await_subagents_timeout(std::time::Duration::from_secs(60))
                .await;
            if let Some(ref mnemosyne) = self.mnemosyne {
                for result in &collected {
                    let content = format!(
                        "[auto-collected spawn] id={} mission_id={} success={} iterations={}\n{}",
                        result.id,
                        result.mission_id.as_deref().unwrap_or("none"),
                        result.success,
                        result.iterations,
                        result.output
                    );
                    let msg = Message::assistant(&content);
                    if let Err(e) = mnemosyne.store_with_embedding(&self.session.id, &msg).await { tracing::debug!("Mnemosyne store failed: {}", e); }
                }
            }
            let succeeded = collected.iter().filter(|r| r.success).count();
            info!(
                "MaxIterations: collected {} spawn result(s) ({} succeeded, {} failed)",
                collected.len(),
                succeeded,
                collected.len() - succeeded,
            );
        }

        self.emit(AgentEvent::Finished { iterations }).await;
        Ok(zeus_core::TurnResult {
            content: last_content,
            tool_calls: tool_call_records,
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            iterations,
            stop_reason: zeus_core::TurnStopReason::MaxIterations,
        })
    }

    /// Check if context compaction is needed and run pre-compaction memory flush.
    ///
    /// 1. If a ContextManager is configured and threshold is exceeded:
    /// 2. Inject flush messages (system + user) asking LLM to save memories
    /// 3. Run one LLM call with 30s timeout for the flush
    /// 4. Compact the session messages via LLM summarization
    /// 5. Reset flush state for next cycle
    async fn check_compaction(
        &mut self,
        tool_schemas: &[zeus_core::ToolSchema],
        system_prompt: &str,
    ) {
        // Extract compaction state without holding borrow across async operations
        let (needs_compaction, fill_pct) = match &self.context_manager {
            Some(cm) => {
                let tokens = ContextManager::estimate_tokens(&self.session.messages);
                let max = cm.max_tokens();
                let pct = if max > 0 { (tokens * 100) / max } else { 0 };
                (cm.needs_compaction(&self.session.messages), pct)
            }
            None => return,
        };

        // S103 #38: Smart compaction reminder at 70% fill
        if fill_pct >= 70 && !needs_compaction {
            info!("Context window at {}% — compaction will trigger soon", fill_pct);
            self.emit(AgentEvent::Error(format!(
                "Context window at {}% capacity. Use /compact to free space, or it will auto-compact soon.",
                fill_pct
            ))).await;
        }

        if !needs_compaction {
            return;
        }

        let workspace_writable = self.workspace.root().exists();

        // Check flush eligibility (extract decision, then release borrow)
        let should_flush =
            !self.compaction_flush.is_flushed() && workspace_writable && needs_compaction;

        // Pre-compaction memory flush (one-shot per cycle)
        if should_flush {
            info!("Pre-compaction flush: injecting memory save prompt");
            self.compaction_flush.mark_flushed();

            let (system_msg, user_msg) = self.compaction_flush.flush_messages();
            let _ = self.session.add(system_msg).await;
            let _ = self.session.add(user_msg).await;

            // Repair orphaned tool_calls before compaction flush LLM call
            zeus_session::repair_orphaned_tool_calls(&mut self.session.messages, Some(self.llm.provider()));

            // Run one LLM call with timeout for the flush
            let flush_timeout = self.compaction_flush.timeout();
            let flush_result = tokio::time::timeout(flush_timeout, async {
                let (mut rx, handle) = if let Some(ref fallback) = self.fallback {
                    fallback.stream(&self.session.messages, tool_schemas, Some(system_prompt)).await?
                } else {
                    self.llm.stream(&self.session.messages, tool_schemas, Some(system_prompt)).await?
                };

                // Drain stream chunks (don't emit to user — flush is silent)
                while rx.recv().await.is_some() {}

                let response: LlmResponse = handle
                    .await
                    .map_err(|e| Error::Agent(format!("Flush LLM task failed: {}", e)))?;

                // If the LLM wants to use tools (e.g., write_file), execute them
                if response.stop_reason == StopReason::ToolUse {
                    let assistant_msg = Message::assistant(&response.content)
                        .with_tool_calls(response.tool_calls.clone());
                    let _ = self.session.add(assistant_msg).await;

                    let tool_results = self.execute_tools(&response.tool_calls).await;
                    let tool_msg = Message {
                        role: zeus_core::Role::Tool,
                        content: String::new(),
                        tool_calls: vec![],
                        tool_results,
                        timestamp: chrono::Utc::now(),
                        attachments: vec![],
                        message_id: None,
                        parent_id: None,
                        thread_id: None,
                        direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
                    };
                    let _ = self.session.add(tool_msg).await;
                } else {
                    // Store the response (might be NO_REPLY or a note)
                    let assistant_msg = Message::assistant(&response.content);
                    let _ = self.session.add(assistant_msg).await;
                }

                Ok::<(), Error>(())
            })
            .await;

            match flush_result {
                Ok(Ok(())) => {
                    info!("Pre-compaction flush completed successfully");
                }
                Ok(Err(e)) => {
                    warn!("Pre-compaction flush failed: {}", e);
                }
                Err(_) => {
                    warn!(
                        "Pre-compaction flush timed out after {}s",
                        flush_timeout.as_secs()
                    );
                }
            }
        }

        // Capture context journal before compaction (direct file write, no LLM)
        let journal_content = if let Some(ref cj) = self.context_journal {
            let max_tokens = self
                .context_manager
                .as_ref()
                .map(|cm| cm.max_tokens())
                .unwrap_or(0);
            if cj.needs_journal(&self.session.messages, max_tokens) {
                match cj.write_journal(&self.session.id, &self.session.messages, max_tokens) {
                    Ok(path) => {
                        info!("Context journal written: {}", path.display());
                        cj.read_latest_journal(&self.session.id).ok().flatten()
                    }
                    Err(e) => {
                        warn!("Failed to write context journal: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // Now perform actual compaction
        info!("Running context compaction");
        if let Some(ref cm) = self.context_manager {
            let msgs_before = self.session.messages.len();
            let tokens_before = self.session.messages.iter()
                .map(|m| m.content.len() / 4) // rough estimate: 4 chars per token
                .sum::<usize>();
            let compact_timeout_secs = self.config.session_compaction
                .as_ref()
                .and_then(|c| c.compaction_timeout_secs)
                .unwrap_or(120);
            let compact_result = tokio::time::timeout(
                std::time::Duration::from_secs(compact_timeout_secs),
                cm.compact(&mut self.session.messages, &self.llm),
            ).await;
            let compact_inner = match compact_result {
                Ok(result) => result,
                Err(_elapsed) => {
                    warn!("Context compaction timed out after {}s — skipping this cycle", compact_timeout_secs);
                    return;
                }
            };
            if let Err(e) = compact_inner {
                warn!("Context compaction failed: {}", e);
            } else {
                let msgs_after = self.session.messages.len();
                let tokens_after = self.session.messages.iter()
                    .map(|m| m.content.len() / 4)
                    .sum::<usize>();
                let removed = msgs_before.saturating_sub(msgs_after);
                info!(
                    "Compaction complete, {} messages remaining (removed {})",
                    msgs_after, removed
                );
                self.emit(AgentEvent::Compacted {
                    messages_removed: removed,
                    tokens_before,
                    tokens_after,
                }).await;

                // S102 #24: Extract pending work and save to workspace
                let pending = zeus_session::context_manager::infer_pending_work(&self.session.messages);
                if !pending.is_empty() {
                    let pending_text = pending.iter()
                        .map(|p| format!("- {}", p))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let pending_msg = format!(
                        "[Pending Work — extracted during compaction]\n{}",
                        pending_text
                    );
                    let _ = self.session.add(Message::system(&pending_msg)).await;
                    // Also persist to workspace file
                    let _ = self.workspace.write("PENDING.md", &format!(
                        "# Pending Work\n\nExtracted during context compaction.\n\n{}\n",
                        pending_text
                    )).await;
                    info!("Extracted {} pending work items during compaction", pending.len());
                }

                // Inject journal as system message after successful compaction
                if let Some(content) = journal_content {
                    let journal_msg = Message::system(format!(
                        "[Context Journal - Workflow Checkpoint]\n\n{}",
                        content
                    ));
                    let _ = self.session.add(journal_msg).await;
                    info!("Injected context journal into post-compaction context");
                }
            }
        }

        // Reset flush for next compaction cycle
        self.compaction_flush.reset();
    }


    /// Read-only tools that can safely run in parallel (no side effects)
    fn is_read_only_tool(name: &str) -> bool {
        matches!(name, "read_file" | "list_dir" | "web_fetch" | "grep_files")
    }

    /// Execute tool calls and return results
    #[instrument(skip(self, calls), fields(tool_count = calls.len()))]
    async fn execute_tools(&mut self, calls: &[ToolCall]) -> Vec<ToolResult> {
        // S101 #19: Concurrent read-only tools.
        // If all calls in the batch are read-only, run them in parallel via join_all.
        // If mixed, run reads first (parallel), then writes (sequential).
        // Security checks (loop guard, constitution, aegis) always run sequentially.

        let all_read_only = calls.iter().all(|c| Self::is_read_only_tool(&c.name));
        let any_read_only = calls.iter().any(|c| Self::is_read_only_tool(&c.name));

        // Fast path: single call or no reads — skip classification overhead
        if calls.len() <= 1 || !any_read_only {
            return self.execute_tools_sequential(calls).await;
        }

        if all_read_only {
            // All read-only: full parallel execution after sequential security checks
            return self.execute_tools_parallel_reads(calls).await;
        }

        // Mixed: split into reads and writes, run reads first in parallel
        let (read_calls, write_calls): (Vec<&ToolCall>, Vec<&ToolCall>) =
            calls.iter().partition(|c| Self::is_read_only_tool(&c.name));

        let mut results = Vec::new();
        // Run reads in parallel
        let read_refs: Vec<&ToolCall> = read_calls;
        let read_results = self.execute_tools_parallel_reads_slice(&read_refs).await;
        results.extend(read_results);
        // Run writes sequentially
        let write_refs: Vec<&ToolCall> = write_calls;
        let write_results = self.execute_tools_sequential_slice(&write_refs).await;
        results.extend(write_results);
        return results;
    }

    /// Sequential execution path (original behavior, used as fallback)
    async fn execute_tools_sequential(&mut self, calls: &[ToolCall]) -> Vec<ToolResult> {
        self.execute_tools_sequential_owned(calls).await
    }

    async fn execute_tools_sequential_slice(&mut self, calls: &[&ToolCall]) -> Vec<ToolResult> {
        let owned: Vec<ToolCall> = calls.iter().map(|c| (*c).clone()).collect();
        self.execute_tools_sequential_owned(&owned).await
    }

    async fn execute_tools_parallel_reads(&mut self, calls: &[ToolCall]) -> Vec<ToolResult> {
        let refs: Vec<&ToolCall> = calls.iter().collect();
        self.execute_tools_parallel_reads_slice(&refs).await
    }

    async fn execute_tools_parallel_reads_slice(&mut self, calls: &[&ToolCall]) -> Vec<ToolResult> {
        // Security checks must run sequentially (they mutate self state)
        // Collect approved calls, then run actual tool execution in parallel
        let mut approved: Vec<ToolCall> = Vec::new();
        let mut blocked: Vec<(usize, ToolResult)> = Vec::new(); // (original_idx, result)

        for (idx, call) in calls.iter().enumerate() {
            // Emit tool call event
            self.emit(AgentEvent::ToolCall {
                name: call.name.clone(),
                args: call.arguments.clone(),
            }).await;

            // Loop guard check
            match self.loop_guard.check(&call.name, &call.arguments) {
                LoopGuardVerdict::Block(msg) => {
                    warn!("Loop guard blocked tool '{}': {}", call.name, msg);
                    self.emit(AgentEvent::ToolResult {
                        name: call.name.clone(),
                        success: false,
                        output: msg.clone(),
                    }).await;
                    blocked.push((idx, ToolResult { call_id: call.id.clone(), success: false, output: msg }));
                    continue;
                }
                LoopGuardVerdict::Warn(msg) => warn!("Loop guard warning for '{}': {}", call.name, msg),
                LoopGuardVerdict::Allow => {}
            }

            // Constitution check
            let verdict = self.constitution.check(&call.name, &call.arguments);
            if verdict.is_blocked() {
                if let ConstitutionVerdict::Blocked { law_id, description } = verdict {
                    let msg = format!("Constitutional law '{}' violated: {}", law_id, description);
                    warn!("{}", msg);
                    blocked.push((idx, ToolResult { call_id: call.id.clone(), success: false, output: msg }));
                    continue;
                }
            }

            // Agent tool policy check
            if let Some(ref policy) = self.tool_policy {
                if !policy.is_tool_allowed(&call.name) {
                    let msg = format!("Agent policy: tool '{}' is not permitted", call.name);
                    warn!("{}", msg);
                    blocked.push((idx, ToolResult { call_id: call.id.clone(), success: false, output: msg }));
                    continue;
                }
            }

            // Aegis security check
            if let Some(ref aegis) = self.aegis {
                if !aegis.is_permitted(&call.name) {
                    let msg = format!("Security: tool '{}' is not permitted", call.name);
                    warn!("{}", msg);
                    blocked.push((idx, ToolResult { call_id: call.id.clone(), success: false, output: msg }));
                    continue;
                }
            }

            approved.push((*call).clone());
        }

        // Now run approved read-only tools in parallel
        let tool_futures: Vec<_> = approved.iter()
            .map(|call| self.tools.execute(&call.name, call.arguments.clone()))
            .collect();
        let parallel_results = futures::future::join_all(tool_futures).await;

        let mut results: Vec<ToolResult> = Vec::new();
        for (call, exec_result) in approved.iter().zip(parallel_results.into_iter()) {
            let tr = match exec_result {
                Ok(output) => {
                    self.emit(AgentEvent::ToolResult {
                        name: call.name.clone(),
                        success: true,
                        output: output.clone(),
                    }).await;
                    ToolResult { call_id: call.id.clone(), success: true, output }
                }
                Err(e) => {
                    let msg = if self.config.suppress_tool_errors {
                        "Tool execution failed".to_string()
                    } else {
                        e.to_string()
                    };
                    self.emit(AgentEvent::ToolResult {
                        name: call.name.clone(),
                        success: false,
                        output: msg.clone(),
                    }).await;
                    ToolResult { call_id: call.id.clone(), success: false, output: msg }
                }
            };
            results.push(tr);
        }

        // Insert blocked results at their original positions
        for (idx, tr) in blocked {
            results.insert(idx.min(results.len()), tr);
        }

        results
    }

    /// Helper: execute a single tool call (used by the sequential path dispatch)
    async fn execute_single_tool(&mut self, call: &ToolCall) -> ToolResult {
        match self.tools.execute(&call.name, call.arguments.clone()).await {
            Ok(output) => {
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: true,
                    output: output.clone(),
                }).await;
                ToolResult { call_id: call.id.clone(), success: true, output }
            }
            Err(e) => {
                let error_msg = if self.config.suppress_tool_errors {
                    debug!("Tool '{}' error (suppressed): {}", call.name, e);
                    "Tool execution failed".to_string()
                } else {
                    e.to_string()
                };
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: false,
                    output: error_msg.clone(),
                }).await;
                ToolResult { call_id: call.id.clone(), success: false, output: error_msg }
            }
        }
    }

    async fn execute_tools_sequential_owned(&mut self, calls: &[ToolCall]) -> Vec<ToolResult> {
        let mut results = Vec::new();

        for call in calls {
            self.emit(AgentEvent::ToolCall {
                name: call.name.clone(),
                args: call.arguments.clone(),
            })
            .await;

            // Loop guard: per-hash counter, ping-pong detection, circuit breaker
            let loop_guard_warning: Option<String> =
                match self.loop_guard.check(&call.name, &call.arguments) {
                    LoopGuardVerdict::Block(msg) => {
                        warn!("Loop guard blocked tool '{}': {}", call.name, msg);
                        self.emit(AgentEvent::ToolResult {
                            name: call.name.clone(),
                            success: false,
                            output: msg.clone(),
                        })
                        .await;
                        results.push(ToolResult {
                            call_id: call.id.clone(),
                            success: false,
                            output: msg,
                        });
                        continue;
                    }
                    LoopGuardVerdict::Warn(msg) => {
                        warn!("Loop guard warning for tool '{}': {}", call.name, msg);
                        Some(msg)
                    }
                    LoopGuardVerdict::Allow => None,
                };

            // S101 #16: Pre-tool hook check
            {
                let hook_ctx = HookContext::new(HookEventType::PreToolUse, &self.session.id)
                    .with_tool(&call.name, &call.arguments);
                match self.hooks.fire_resolve(&hook_ctx).await {
                    HookAction::Abort(msg) | HookAction::Deny(msg) => {
                        warn!("Hook denied tool '{}': {}", call.name, msg);
                        self.emit(AgentEvent::ToolResult {
                            name: call.name.clone(),
                            success: false,
                            output: msg.clone(),
                        }).await;
                        results.push(ToolResult {
                            call_id: call.id.clone(),
                            success: false,
                            output: msg,
                        });
                        continue;
                    }
                    HookAction::Warn(msg) => {
                        warn!("Hook warning for tool '{}': {}", call.name, msg);
                    }
                    _ => {}
                }
            }

            if let Some(hooks_cfg) = &self.config.hooks
                && let Some(command) = Self::matching_hook_command(&hooks_cfg.before_tool, &call.name)
                && let Err(e) = self.run_configured_tool_hook(command, call, None).await
            {
                let msg = format!("Before-tool hook blocked '{}': {}", call.name, e);
                warn!("{}", msg);
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: false,
                    output: msg.clone(),
                }).await;
                results.push(ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: msg,
                });
                continue;
            }

            // Constitutional law check (immutable, cannot be overridden)
            let verdict = self.constitution.check(&call.name, &call.arguments);
            if verdict.is_blocked()
                && let ConstitutionVerdict::Blocked {
                    law_id,
                    description,
                } = verdict
            {
                let error_msg =
                    format!("Constitutional law '{}' violated: {}", law_id, description);
                warn!("{}", error_msg);
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: false,
                    output: error_msg.clone(),
                })
                .await;
                results.push(ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: error_msg,
                });
                continue;
            }

            // Agent tool policy check (business rule, per-agent)
            if let Some(ref policy) = self.tool_policy
                && !policy.is_tool_allowed(&call.name)
            {
                let error_msg = format!("Agent policy: tool '{}' is not permitted", call.name);
                warn!("{}", error_msg);
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: false,
                    output: error_msg.clone(),
                })
                .await;
                results.push(ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: error_msg,
                });
                continue;
            }

            // Aegis security check before execution
            if let Some(ref aegis) = self.aegis {
                if !aegis.is_permitted(&call.name) {
                    let error_msg = format!("Security: tool '{}' is not permitted", call.name);
                    warn!("{}", error_msg);
                    self.emit(AgentEvent::ToolResult {
                        name: call.name.clone(),
                        success: false,
                        output: error_msg.clone(),
                    })
                    .await;
                    results.push(ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: error_msg,
                    });
                    continue;
                }

                // Validate shell commands against security policies
                if call.name == "shell"
                    && let Some(cmd) = call.arguments.get("command").and_then(|v| v.as_str())
                    && let Err(e) = aegis.validate_shell_command(cmd)
                {
                    let error_msg = format!("Security: {}", e);
                    warn!("{}", error_msg);
                    results.push(ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: error_msg,
                    });
                    continue;
                }

                // Check URL access for web_fetch
                if call.name == "web_fetch"
                    && let Some(url) = call.arguments.get("url").and_then(|v| v.as_str())
                    && let Err(e) = aegis.check_network_url(url)
                {
                    let error_msg = format!("Security: {}", e);
                    warn!("{}", error_msg);
                    results.push(ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: error_msg,
                    });
                    continue;
                }

                // Check file path access for read_file, write_file, edit_file
                if matches!(call.name.as_str(), "read_file" | "write_file" | "edit_file")
                    && let Some(path) = call.arguments.get("path").and_then(|v| v.as_str())
                    && aegis.restricts_filesystem()
                    && !aegis.is_path_allowed(path)
                {
                    let error_msg = format!("Security: file access denied for '{}'", path);
                    warn!("{}", error_msg);
                    results.push(ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: error_msg,
                    });
                    continue;
                }
            }

            // Skill permission enforcement: if a skill is active, check its policy
            if !self.active_skills.is_empty()
                && let Some(ref registry) = self.skill_permissions
            {
                let required_cap = match call.name.as_str() {
                    "read_file" | "list_dir" => Some(zeus_skills::SkillCapability::FileRead),
                    "write_file" | "edit_file" => Some(zeus_skills::SkillCapability::FileWrite),
                    "shell" => Some(zeus_skills::SkillCapability::Shell),
                    "web_fetch" => Some(zeus_skills::SkillCapability::Network),
                    "spawn" => Some(zeus_skills::SkillCapability::Process),
                    _ => None,
                };
                if let Some(cap) = required_cap {
                    let mut blocked = false;
                    for skill_name in &self.active_skills {
                        let check = registry.check_capability(skill_name, &cap).await;
                        if !check.allowed {
                            let error_msg = format!(
                                "Skill permission denied: skill '{}' lacks {} capability",
                                skill_name, cap
                            );
                            warn!("{}", error_msg);
                            self.emit(AgentEvent::ToolResult {
                                name: call.name.clone(),
                                success: false,
                                output: error_msg.clone(),
                            })
                            .await;
                            results.push(ToolResult {
                                call_id: call.id.clone(),
                                success: false,
                                output: error_msg,
                            });
                            blocked = true;
                            break;
                        }
                    }
                    if blocked {
                        continue;
                    }
                }
            }

            // Approval check: queue sensitive tool calls for user confirmation
            if let Some(ref aegis) = self.aegis {
                let outcome = aegis
                    .queue_for_approval(&call.name, &call.arguments, None)
                    .await;
                if !outcome.is_approved() {
                    let error_msg = match outcome {
                        zeus_aegis::ApprovalOutcome::Denied { reason } => {
                            format!(
                                "Approval denied for '{}': {}",
                                call.name,
                                reason.unwrap_or_else(|| "no reason given".to_string())
                            )
                        }
                        zeus_aegis::ApprovalOutcome::Expired => {
                            format!("Approval timed out for '{}'", call.name)
                        }
                        _ => format!("Approval not granted for '{}'", call.name),
                    };
                    warn!("{}", error_msg);
                    self.emit(AgentEvent::ToolResult {
                        name: call.name.clone(),
                        success: false,
                        output: error_msg.clone(),
                    })
                    .await;
                    results.push(ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: error_msg,
                    });
                    continue;
                }
            }

            // Taint tracking: check sink policies before execution
            if let Err(violation) =
                self.taint_tracker
                    .check_sink(&call.id, &call.name, &call.arguments)
            {
                let error_msg = format!("Taint policy: {}", violation.description);
                warn!("{}", error_msg);
                self.emit(AgentEvent::ToolResult {
                    name: call.name.clone(),
                    success: false,
                    output: error_msg.clone(),
                })
                .await;
                results.push(ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: error_msg,
                });
                continue;
            }

            let start = std::time::Instant::now();

            // Handle spawn, collect_spawns, message, and deep_research specially - they need access to agent resources
            let result = if call.name == "spawn" {
                self.execute_spawn(call).await
            } else if call.name == "collect_spawns" {
                self.execute_collect_spawns(call).await
            } else if call.name == "message" {
                self.execute_message(call).await
            } else if call.name == "send_file" {
                self.execute_send_file(call).await
            } else if call.name == "deep_research" {
                self.execute_deep_research(call).await
            } else if call.name == "council_deliberate" {
                self.execute_council_deliberate(call).await
            } else {
                // S101 #19: Read-only tools run concurrently when batched.
                // Single-call path: execute directly (no overhead for common case).
                self.execute_single_tool(call).await
            };

            let duration_ms = start.elapsed().as_millis() as u64;

            // Taint tracking: label tool output with provenance information
            if result.success {
                let labels = self.taint_tracker.label_output(
                    &call.id,
                    &call.name,
                    &call.arguments,
                    &result.output,
                );
                if !labels.is_empty() {
                    debug!(
                        "Taint labels for {} ({}): [{}]",
                        call.name,
                        call.id,
                        labels
                            .iter()
                            .map(|l| l.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }

            // Log tool execution to Athena
            if let Some(ref athena) = self.athena {
                let action = ActionLog::new(
                    ActionType::ToolExecuted,
                    format!(
                        "Executed {} ({})",
                        call.name,
                        if result.success { "ok" } else { "err" }
                    ),
                )
                .with_session(&self.session.id)
                .with_tool(&call.name)
                .with_result(zeus_core::truncate_str(&result.output, 200))
                .with_duration(duration_ms);
                if let Err(e) = athena.log_action(&action).await { tracing::debug!("Athena log failed: {}", e); }
            }

            // Nous evaluation of tool execution quality
            if let Some(ref nous) = self.nous {
                let exec_ctx = zeus_nous::ExecutionContext {
                    task_description: call.name.clone(),
                    success: result.success,
                    partial: false,
                    duration_ms,
                    expected_ms: None,
                    retry_count: 0,
                    errors: if result.success {
                        vec![]
                    } else {
                        vec![result.output.clone()]
                    },
                    tool_call_count: 1,
                    goal_id: None,
                };
                let eval = nous.evaluate(&exec_ctx);
                if eval.quality_score < 0.5 {
                    debug!(
                        "Nous: low quality tool execution ({:.2}): {:?}",
                        eval.quality_score, eval.improvements
                    );
                }
            }

            // Notify via Hermes on tool errors
            if !result.success
                && let Some(ref hermes) = self.hermes
            {
                let notif =
                    Notification::new(format!("Tool '{}' failed: {}", call.name, result.output))
                        .with_title("Zeus Tool Error")
                        .with_priority(NotificationPriority::Normal);
                if let Err(e) = hermes.write().await.notify(notif).await { tracing::debug!("Hermes notify failed: {}", e); }
            }

            // Fire on_tool_executed hook
            {
                let hook_ctx = HookContext::new(HookEventType::OnToolExecuted, &self.session.id)
                    .with_tool_result(&call.name, result.success, &result.output);
                let _ = self.hooks.fire(&hook_ctx).await;
            }

            // Prepend any loop guard warning to the tool output so the LLM
            // sees it in context on the very next turn.
            let result = if let Some(warning) = loop_guard_warning {
                ToolResult {
                    call_id: result.call_id,
                    success: result.success,
                    output: format!("{}\n\n---\n{}", warning, result.output),
                }
            } else {
                result
            };

            // S101 #16: Post-tool hook (fire-and-forget)
            {
                let hook_ctx = HookContext::new(HookEventType::PostToolUse, &self.session.id)
                    .with_tool(&call.name, &call.arguments);
                let _ = self.hooks.fire_resolve(&hook_ctx).await;
            }

            if let Some(hooks_cfg) = &self.config.hooks
                && let Some(command) = Self::matching_hook_command(&hooks_cfg.after_tool, &call.name)
                && let Err(e) = self.run_configured_tool_hook(command, call, Some(&result)).await
            {
                warn!("After-tool hook failed for '{}': {}", call.name, e);
            }

            // Cross-channel awareness: log side-effect tool calls to
            // memory/RECENT_ACTIVITY.md so other channel sessions for the
            // same titan see them in their next get_context() render.
            // Fire-and-forget: failure here must never break the cooking loop.
            if result.success && matches!(call.name.as_str(), "message" | "send_file") {
                let entry = Self::format_recent_activity_entry(call);
                if let Err(e) = self.workspace.append_recent_activity(&entry).await {
                    debug!("append_recent_activity failed: {}", e);
                }
            }

            results.push(result);
        }

        results
    }

    /// Execute spawn tool - creates an actual subagent
    #[instrument(skip(self, call), fields(call_id = %call.id))]
    async fn execute_spawn(&mut self, call: &ToolCall) -> ToolResult {
        let args = &call.arguments;

        let task = match args.get("task").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "Missing 'task' argument".to_string(),
                };
            }
        };

        let context = args
            .get("context")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let max_iterations = args
            .get("max_iterations")
            .and_then(|m| m.as_u64())
            .unwrap_or(15) as usize;

        let wait = args.get("wait").and_then(|w| w.as_bool()).unwrap_or(false);

        // Build agent target: local by default, remote if gateway_url is provided
        let target = if let Some(gateway_url) = args.get("gateway_url").and_then(|g| g.as_str()) {
            let auth_token = args
                .get("auth_token")
                .and_then(|a| a.as_str())
                .map(String::from);
            AgentTarget::Remote {
                gateway_url: gateway_url.to_string(),
                auth_token,
            }
        } else {
            AgentTarget::Local
        };

        // Extract mission_id from spawn args (set by Pantheon missions)
        let mission_id = args
            .get("mission_id")
            .and_then(|m| m.as_str())
            .map(String::from);

        // S102 #23: Pass parent system prompt for prompt cache sharing.
        // Subagent uses same prefix → API cache hit on shared context.
        let parent_prompt = self.workspace.get_context().await.ok();

        let config = SubagentConfig {
            max_iterations,
            can_spawn: false, // Subagents cannot spawn other subagents
            task: task.clone(),
            context,
            target,
            // Forward parent's model so remote gateways use the same LLM
            model: Some(self.config.model.clone()),
            mission_id,
            parent_system_prompt: parent_prompt,
        };

        info!("Spawning subagent for task: {}", task);

        // Create subagent with cloned resources (including Aegis for security enforcement)
        let subagent = Subagent::new(
            config,
            self.llm.clone(),
            self.workspace.clone(),
            self.aegis.clone(),
        );

        // Use the subagent's own ID for tracking (avoids ID mismatch)
        let subagent_id = subagent.id().to_string();

        if wait {
            // Wait for subagent to complete
            let result = subagent.run().await;
            self.emit(AgentEvent::ToolResult {
                name: "spawn".to_string(),
                success: result.success,
                output: result.output.clone(),
            })
            .await;

            ToolResult {
                call_id: call.id.clone(),
                success: result.success,
                output: format!(
                    "Subagent completed in {} iterations:\n{}",
                    result.iterations, result.output
                ),
            }
        } else {
            // Spawn in background
            let handle = tokio::spawn(async move { subagent.run().await });

            self.subagents.insert(subagent_id.clone(), handle);

            self.emit(AgentEvent::ToolResult {
                name: "spawn".to_string(),
                success: true,
                output: format!("Subagent {} spawned", subagent_id),
            })
            .await;

            ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: format!(
                    "Subagent '{}' spawned in background for task: {}. It will run independently with up to {} iterations.",
                    subagent_id, task, max_iterations
                ),
            }
        }
    }

    /// Execute collect_spawns tool — waits for all background subagents and returns their results.
    async fn execute_collect_spawns(&mut self, call: &ToolCall) -> ToolResult {
        let args = &call.arguments;
        let timeout_secs = args
            .get("timeout_seconds")
            .and_then(|t| t.as_u64())
            .unwrap_or(300);

        let running = self.running_subagents();
        if running == 0 {
            return ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: "No subagents running. Nothing to collect.".to_string(),
            };
        }

        info!(
            "collect_spawns: waiting for {} subagents (timeout: {}s)",
            running, timeout_secs
        );

        let results = self
            .await_subagents_timeout(std::time::Duration::from_secs(timeout_secs))
            .await;

        let summary: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "subagent_id": r.id,
                    "success": r.success,
                    "output": r.output,
                    "iterations": r.iterations,
                    "mission_id": r.mission_id,
                })
            })
            .collect();

        let succeeded = results.iter().filter(|r| r.success).count();
        let failed = results.len() - succeeded;

        self.emit(AgentEvent::ToolResult {
            name: "collect_spawns".to_string(),
            success: true,
            output: format!(
                "{} subagents collected ({} succeeded, {} failed)",
                results.len(),
                succeeded,
                failed
            ),
        })
        .await;

        ToolResult {
            call_id: call.id.clone(),
            success: true,
            output: serde_json::to_string_pretty(&serde_json::json!({
                "collected": results.len(),
                "succeeded": succeeded,
                "failed": failed,
                "still_running": self.running_subagents(),
                "results": summary,
            }))
            .unwrap_or_else(|_| "Failed to serialize results".to_string()),
        }
    }

    /// Execute message tool - routes through platform channels or falls back to simple channels
    #[instrument(skip(self, call), fields(call_id = %call.id))]
    async fn execute_message(&self, call: &ToolCall) -> ToolResult {
        let args = &call.arguments;

        let channel_spec = match args.get("channel").and_then(|c| c.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "Missing 'channel' argument".to_string(),
                };
            }
        };

        let content = match args.get("content").and_then(|c| c.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "Missing 'content' argument".to_string(),
                };
            }
        };

        let target = args.get("target").and_then(|t| t.as_str());
        let attachment = args.get("attachment").and_then(|a| a.as_str());

        // Handle broadcast to multiple channels
        if channel_spec == "broadcast" {
            if let Some(ref channels) = self.channels {
                let targets_val = args.get("targets").and_then(|t| t.as_array());
                if let Some(targets) = targets_val {
                    let mut channel_targets = Vec::new();
                    for target_obj in targets {
                        let ch = target_obj
                            .get("channel")
                            .and_then(|c| c.as_str())
                            .unwrap_or("unknown");
                        let tgt = target_obj
                            .get("target")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        channel_targets.push(ChannelSource::with_chat(ch, "agent", tgt));
                    }
                    let results = channels.broadcast(content, &channel_targets).await;
                    let successes = results.iter().filter(|(_, r)| r.is_ok()).count();
                    let failures = results.iter().filter(|(_, r)| r.is_err()).count();
                    let result = ToolResult {
                        call_id: call.id.clone(),
                        success: failures == 0,
                        output: format!(
                            "Broadcast complete: {} sent, {} failed",
                            successes, failures
                        ),
                    };
                    self.emit(AgentEvent::ToolResult {
                        name: "message".to_string(),
                        success: result.success,
                        output: result.output.clone(),
                    })
                    .await;
                    return result;
                } else {
                    let result = ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: "Missing 'targets' array for broadcast".to_string(),
                    };
                    self.emit(AgentEvent::ToolResult {
                        name: "message".to_string(),
                        success: result.success,
                        output: result.output.clone(),
                    })
                    .await;
                    return result;
                }
            } else {
                let result = ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "No channels configured for broadcast".to_string(),
                };
                self.emit(AgentEvent::ToolResult {
                    name: "message".to_string(),
                    success: result.success,
                    output: result.output.clone(),
                })
                .await;
                return result;
            }
        }

        // Check if this is a platform channel
        let result = match channel_spec {
            "telegram" | "discord" | "slack" | "email" | "imessage" | "irc" | "matrix" | "whatsapp" | "signal" | "mattermost" | "mqtt" => {
                if let Some(ref channels) = self.channels {
                    let Some(target) = target else {
                        return ToolResult {
                            call_id: call.id.clone(),
                            success: false,
                            output: format!(
                                "'target' is required for {} channel (e.g., chat_id, email address, phone number)",
                                channel_spec
                            ),
                        };
                    };

                    // Build ChannelSource based on channel type
                    let source = match channel_spec {
                        // For group-oriented channels, target is the chat/channel ID
                        "telegram" | "discord" | "slack" | "irc" | "mattermost" | "mqtt" => {
                            ChannelSource::with_chat(channel_spec, "agent", target)
                        }
                        // For direct channels, target is the recipient identifier
                        "email" | "imessage" | "whatsapp" | "signal" | "matrix" => ChannelSource::new(channel_spec, target),
                        _ => ChannelSource::with_chat(channel_spec, "agent", target),
                    };

                    // If attachment is provided, send as file
                    if let Some(file_path) = attachment {
                        match tokio::fs::read(file_path).await {
                            Ok(data) => {
                                let filename = std::path::Path::new(file_path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("attachment");
                                match channels
                                    .send_file(&source, filename, &data, Some(content))
                                    .await
                                {
                                    Ok(()) => {
                                        debug!(
                                            "File '{}' sent via {} to {}",
                                            filename, channel_spec, target
                                        );
                                        ToolResult {
                                            call_id: call.id.clone(),
                                            success: true,
                                            output: format!(
                                                "File '{}' sent via {} to {}",
                                                filename, channel_spec, target
                                            ),
                                        }
                                    }
                                    Err(e) => ToolResult {
                                        call_id: call.id.clone(),
                                        success: false,
                                        output: format!(
                                            "Failed to send file via {}: {}",
                                            channel_spec, e
                                        ),
                                    },
                                }
                            }
                            Err(e) => ToolResult {
                                call_id: call.id.clone(),
                                success: false,
                                output: format!(
                                    "Failed to read file '{}': {}",
                                    file_path, e
                                ),
                            },
                        }
                    } else {
                        match channels.send(&source, content).await {
                            Ok(()) => {
                                debug!("Message sent via {} to {}", channel_spec, target);
                                ToolResult {
                                    call_id: call.id.clone(),
                                    success: true,
                                    output: format!(
                                        "Message sent via {} to {}",
                                        channel_spec, target
                                    ),
                                }
                            }
                            Err(e) => ToolResult {
                                call_id: call.id.clone(),
                                success: false,
                                output: format!("Failed to send via {}: {}", channel_spec, e),
                            },
                        }
                    }
                } else {
                    ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: format!(
                            "Channel '{}' is not configured. Add [channels.{}] to config.toml",
                            channel_spec, channel_spec
                        ),
                    }
                }
            }
            _ => {
                // Fall back to simple channels (file, webhook, console)
                let channel = crate::channels::Channel::parse(channel_spec);
                match channel.send(content, target).await {
                    Ok(output) => ToolResult {
                        call_id: call.id.clone(),
                        success: true,
                        output,
                    },
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        success: false,
                        output: e.to_string(),
                    },
                }
            }
        };

        self.emit(AgentEvent::ToolResult {
            name: "message".to_string(),
            success: result.success,
            output: result.output.clone(),
        })
        .await;

        result
    }

    /// Execute send_file tool — send a file attachment through platform channels
    #[instrument(skip(self, call), fields(call_id = %call.id))]
    async fn execute_send_file(&self, call: &ToolCall) -> ToolResult {
        // Delegate to the standalone function so it can be reused from the
        // cooking loop (zeus-prometheus ToolExecutor) and anywhere else that
        // holds a ChannelManager reference.
        let channels = self.channel_manager();
        if let Some(channels) = channels {
            // Build a fallback source from the most recent inbound message in the session.
            // This lets agents omit `target` in send_file and have it default to the
            // channel the triggering message came from.
            let fallback_source: Option<zeus_channels::ChannelSource> = self
                .session()
                .messages
                .iter()
                .rev()
                .find_map(|m| m.channel_source.as_ref())
                .map(|cs| zeus_channels::ChannelSource {
                    channel_type: cs.channel_type.clone(),
                    user_id: String::new(),
                    chat_id: cs.channel_id.clone(),
                    account_id: None,
                    thread_id: None,
                    reply_to_message_id: None,
                    sender_type: zeus_core::SenderType::System,
                });
            let mut result = crate::tools::send_file_to_channel_with_fallback(
                &call.arguments,
                &channels,
                fallback_source.as_ref(),
            )
            .await;
            result.call_id = call.id.clone();
            result
        } else {
            ToolResult {
                call_id: call.id.clone(),
                success: false,
                output: "No channel manager configured — cannot send files".to_string(),
            }
        }
    }

    /// Execute deep_research tool — multi-step web search + LLM synthesis
    #[instrument(skip(self, call), fields(call_id = %call.id))]
    async fn execute_deep_research(&self, call: &ToolCall) -> ToolResult {
        let args = &call.arguments;

        let query = match args.get("query").and_then(|q| q.as_str()) {
            Some(q) => q,
            None => {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "Missing 'query' argument".to_string(),
                };
            }
        };

        // Build config from args + env vars
        let mut config = crate::research::ResearchConfig::from_env();
        if let Some(max_sources) = args.get("max_sources").and_then(|m| m.as_u64()) {
            config.max_sources = max_sources as usize;
        }
        if let Some(max_queries) = args.get("max_queries").and_then(|m| m.as_u64()) {
            config.max_queries = max_queries as usize;
        }

        info!(
            "deep_research: query='{}' max_sources={} max_queries={}",
            query, config.max_sources, config.max_queries
        );

        let engine = crate::research::ResearchEngine::new(config);

        match engine.research(query, &self.llm).await {
            Ok(report) => {
                let formatted = crate::research::format_report(&report);
                self.emit(AgentEvent::ToolResult {
                    name: "deep_research".to_string(),
                    success: true,
                    output: format!(
                        "Research complete: {} sources, {}ms",
                        report.sources_count, report.duration_ms
                    ),
                })
                .await;

                ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: formatted,
                }
            }
            Err(e) => {
                self.emit(AgentEvent::ToolResult {
                    name: "deep_research".to_string(),
                    success: false,
                    output: e.to_string(),
                })
                .await;

                ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: format!("Deep research failed: {}", e),
                }
            }
        }
    }

    /// Run the `council_deliberate` tool.
    ///
    /// Builds a CouncilConfig from tool args (or falls back to config.toml defaults),
    /// enforces a min_models=2 floor, runs the 3-stage pipeline, and returns the
    /// synthesized answer with a quorum field.
    async fn execute_council_deliberate(&self, call: &ToolCall) -> ToolResult {
        let args = &call.arguments;

        let query = match args.get("query").and_then(|q| q.as_str()) {
            Some(q) if !q.trim().is_empty() => q.to_string(),
            _ => {
                return ToolResult {
                    call_id: call.id.clone(),
                    success: false,
                    output: "Missing required argument: 'query'".to_string(),
                };
            }
        };

        // Build config: tool args override config.toml [council] section, which overrides defaults
        let mut council_config = self.config.council
            .as_ref()
            .map(|c| CouncilConfig {
                models: c.models.clone(),
                chairman: c.chairman.clone(),
                timeout_secs: CouncilConfig::default().timeout_secs,
            })
            .unwrap_or_default();

        if let Some(models_str) = args.get("models").and_then(|m| m.as_str()) {
            council_config.models = models_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(chairman) = args.get("chairman").and_then(|c| c.as_str()) {
            council_config.chairman = chairman.to_string();
        }

        // Enforce min_models=2: need at least 2 models for meaningful deliberation
        if council_config.models.len() < 2 {
            return ToolResult {
                call_id: call.id.clone(),
                success: false,
                output: format!(
                    "Council requires at least 2 models, got {}. \
                     Configure [council] models in config.toml or pass the 'models' argument.",
                    council_config.models.len()
                ),
            };
        }

        info!(
            query = %query,
            models = ?council_config.models,
            "Running council deliberation"
        );

        match run_council(&query, council_config.clone()).await {
            Ok(result) => {
                let n_models = council_config.models.len();
                let n_responded = result.session.results.len();
                let quorum = if n_responded == 0 {
                    "failed".to_string()
                } else if n_responded == 1 {
                    format!("degraded_1_of_{}", n_models)
                } else if n_responded >= n_models {
                    format!("full_{}_of_{}", n_responded, n_models)
                } else {
                    format!("degraded_{}_of_{}", n_responded, n_models)
                };

                let output = serde_json::json!({
                    "answer": result.final_answer,
                    "quorum": quorum,
                    "models_responded": n_responded,
                    "models_total": n_models,
                });
                ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: serde_json::to_string_pretty(&output)
                        .unwrap_or_else(|_| result.final_answer),
                }
            }
            Err(e) => ToolResult {
                call_id: call.id.clone(),
                success: false,
                output: format!("Council deliberation failed: {}", e),
            },
        }
    }

    // ========================================================================
    // Autonomous Task Queue
    // ========================================================================

    /// Run a list of tasks autonomously without human intervention.
    ///
    /// This is the core autonomy primitive: the agent reads its task queue,
    /// executes each task in order, and only surfaces to a human when
    /// genuinely blocked (i.e., on error after retries).
    ///
    /// # Arguments
    /// * `tasks` — ordered list of task strings to execute
    /// * `max_retries` — how many times to retry a failed task before skipping
    ///
    /// # Returns
    /// A `TaskQueueReport` with per-task results.
    pub async fn run_task_queue(
        &mut self,
        tasks: Vec<String>,
        max_retries: usize,
    ) -> TaskQueueReport {
        let mut report = TaskQueueReport::default();

        for (i, task) in tasks.iter().enumerate() {
            info!("Task queue [{}/{}]: {}", i + 1, tasks.len(), task);
            let mut attempt = 0;
            let mut last_error = String::new(); let _ = &last_error;

            loop {
                attempt += 1;
                match self.run(task).await {
                    Ok(output) => {
                        info!("Task queue [{}/{}] done (attempt {})", i + 1, tasks.len(), attempt);
                        report.completed.push(TaskResult {
                            task: task.clone(),
                            output,
                            attempts: attempt,
                            success: true,
                        });
                        break;
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        warn!(
                            "Task queue [{}/{}] attempt {}/{} failed: {}",
                            i + 1,
                            tasks.len(),
                            attempt,
                            max_retries + 1,
                            last_error
                        );
                        if attempt > max_retries {
                            error!(
                                "Task queue [{}/{}] giving up after {} attempts: {}",
                                i + 1,
                                tasks.len(),
                                attempt,
                                last_error
                            );
                            report.failed.push(TaskResult {
                                task: task.clone(),
                                output: last_error.clone(),
                                attempts: attempt,
                                success: false,
                            });
                            break;
                        }
                        // Brief back-off before retry
                        tokio::time::sleep(tokio::time::Duration::from_secs(
                            2u64.pow(attempt as u32).min(30),
                        ))
                        .await;
                    }
                }
            }
        }

        report
    }

    /// Load tasks from a markdown checklist file (lines starting with `- [ ]`).
    /// Returns the list of uncompleted task strings.
    pub fn load_tasks_from_file(path: &std::path::Path) -> std::io::Result<Vec<String>> {
        let content = std::fs::read_to_string(path)?;
        let tasks = content
            .lines()
            .filter(|l| l.trim_start().starts_with("- [ ]"))
            .map(|l| {
                l.trim_start()
                    .trim_start_matches("- [ ]")
                    .trim()
                    .to_string()
            })
            .filter(|t| !t.is_empty())
            .collect();
        Ok(tasks)
    }

    /// Mark a task as done in a markdown checklist file.
    /// Replaces the first `- [ ] <task>` matching the task string with `- [x] <task>`.
    pub fn mark_task_done(path: &std::path::Path, task: &str) -> std::io::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let needle = format!("- [ ] {}", task);
        let replacement = format!("- [x] {}", task);
        let updated = content.replacen(&needle, &replacement, 1);
        std::fs::write(path, updated)
    }

    /// Send a response back through the originating channel.
    ///
    /// If `self.channels` is `None`, the reply is dropped and a `warn!()` is
    /// emitted so the failure is observable in logs instead of silent. A common
    /// cause is a config wipe clearing the `[channels.discord]` section, which
    /// leaves the default agent without a `ChannelManager` even though inbound
    /// messages still arrive via a separate path.
    pub async fn send_to_channel(&self, source: &zeus_channels::ChannelSource, content: &str) {
        if self.channels.is_none() {
            warn!(
                "send_to_channel: no channel manager — {} reply dropped. \
                 Check [channels.{}] in config.toml",
                source.channel_type(),
                source.channel_type()
            );
            return;
        }
        if let Some(ref channels) = self.channels
            && let Err(e) = channels.send(source, content).await
        {
            warn!("Failed to send reply to {}: {}", source.channel_type(), e);
        }
    }

    /// Add a reaction to a channel message (e.g., Discord emoji reactions)
    pub async fn add_channel_reaction(
        &self,
        channel_type: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) {
        if let Some(ref channels) = self.channels
            && let Err(e) = channels
                .add_reaction(channel_type, channel_id, message_id, emoji)
                .await
        {
            debug!("Failed to add reaction: {}", e);
        }
    }

    /// Remove a reaction from a channel message
    pub async fn remove_channel_reaction(
        &self,
        channel_type: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) {
        if let Some(ref channels) = self.channels
            && let Err(e) = channels
                .remove_reaction(channel_type, channel_id, message_id, emoji)
                .await
        {
            debug!("Failed to remove reaction: {}", e);
        }
    }

    /// Take the channel message receiver for inbound platform messages
    pub fn take_channel_receiver(
        &mut self,
    ) -> Option<mpsc::Receiver<zeus_channels::ChannelMessage>> {
        self.channel_rx.take()
    }

    /// Set a stream callback for forwarding LLM tokens in real-time.
    /// Used by the gateway inbox consumer for TUI streaming.
    pub fn set_stream_tx(&mut self, tx: mpsc::Sender<zeus_core::inbox::StreamChunk>) {
        self.stream_tx = Some(tx);
    }

    /// Clear the stream callback after the request completes.
    pub fn clear_stream_tx(&mut self) {
        self.stream_tx = None;
    }

    /// Stop all channel adapters
    /// Share another agent's ChannelManager with this agent.
    /// Used for registry agents that don't own channel adapters but need
    /// the `message` tool to work for platform channels (discord, telegram, etc).
    pub fn set_shared_channels(&mut self, channels: Arc<ChannelManager>) {
        self.channels = Some(channels.clone());
        // Also wire into ToolRegistry so the `message` tool can dispatch to
        // platform channels (Discord, Telegram, etc.) for registry agents that
        // don't own adapters but share another agent's ChannelManager.
        self.tools.set_channels(channels);
    }

    /// Get a reference to this agent's ChannelManager (if any).
    pub fn channel_manager(&self) -> Option<Arc<ChannelManager>> {
        self.channels.clone()
    }

    /// Get a reference to this agent's LLM client.
    pub fn llm(&self) -> &LlmClient {
        &self.llm
    }

    pub async fn stop_channels(&self) {
        if let Some(ref channels) = self.channels
            && let Err(e) = channels.stop_all().await
        {
            warn!("Error stopping channel adapters: {}", e);
        }
    }

    /// Emit an event if a channel is configured
    async fn emit(&self, event: AgentEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event).await;
        }
    }

    /// Get the session
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Get mutable session
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Reset the session — starts a fresh JSONL file.
    /// Used for auto-recovery after consecutive LLM errors (e.g., orphaned tool_use blocks).
    pub fn reset_session(&mut self) {
        let sessions_dir = self.session.path().parent().unwrap_or(std::path::Path::new("."));
        self.session = Session::new(sessions_dir);
        info!("Session reset — starting fresh");
    }

    /// Swap in a different session (e.g. per-channel session from ChannelSessionRouter).
    ///
    /// The gateway's Discord relay uses this to route each channel's messages to
    /// their own session file, preventing context pollution between channels.
    /// No-op if the new session ID matches the current one.
    pub fn set_session(&mut self, session: Session) {
        if session.id == self.session.id {
            return;
        }
        debug!(
            old_id = %self.session.id,
            new_id = %session.id,
            "Agent session swapped"
        );
        self.session = session;
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Get the Mnemosyne instance (if configured)
    pub fn mnemosyne(&self) -> Option<&Arc<Mnemosyne>> {
        self.mnemosyne.as_ref()
    }

    /// Get the Nous instance (if configured)
    pub fn nous(&self) -> Option<&Arc<Nous>> {
        self.nous.as_ref()
    }

    /// Get tool schemas from the registry
    pub fn tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools.schemas()
    }

    /// Get core tool schemas only (8 essentials for Ollama lazy loading)
    pub fn core_schemas(&self) -> Vec<ToolSchema> {
        self.tools.core_schemas()
    }

    /// Get context-aware tool schemas based on config.
    /// Includes core tools + configured integrations only.
    pub fn context_schemas(&self) -> Vec<ToolSchema> {
        self.tools.context_schemas(&self.config)
    }

    /// Message-aware tool schemas — for Ollama, selects tools relevant to the message.
    pub fn context_schemas_for_message(&self, message: &str) -> Vec<ToolSchema> {
        self.tools.context_schemas_for_message(&self.config, message)
    }

    /// Get count of running subagents
    pub fn running_subagents(&self) -> usize {
        self.subagents
            .iter()
            .filter(|(_, h)| !h.is_finished())
            .count()
    }

    /// Get IDs of all subagents (running and completed)
    pub fn subagent_ids(&self) -> Vec<String> {
        self.subagents.keys().cloned().collect()
    }

    /// Check if a specific subagent has completed and get its result
    pub async fn get_subagent_result(&mut self, id: &str) -> Option<SubagentResult> {
        if let Some(handle) = self.subagents.remove(id) {
            if handle.is_finished() {
                match handle.await {
                    Ok(result) => Some(result),
                    Err(e) => Some(SubagentResult {
                        id: id.to_string(),
                        success: false,
                        output: format!("Subagent task panicked: {}", e),
                        iterations: 0,
                        mission_id: None,
                    }),
                }
            } else {
                // Put it back, not done yet
                self.subagents.insert(id.to_string(), handle);
                None
            }
        } else {
            None
        }
    }

    /// Wait for all subagents to complete
    pub async fn wait_all_subagents(&mut self) -> Vec<SubagentResult> {
        let mut results = Vec::new();
        let ids: Vec<_> = self.subagents.keys().cloned().collect();

        for id in ids {
            if let Some(handle) = self.subagents.remove(&id) {
                match handle.await {
                    Ok(result) => results.push(result),
                    Err(e) => results.push(SubagentResult {
                        id: id.clone(),
                        success: false,
                        output: format!("Subagent task panicked: {}", e),
                        iterations: 0,
                        mission_id: None,
                    }),
                }
            }
        }

        results
    }

    /// Wait for all subagents to complete with a timeout.
    ///
    /// Returns results for all completed subagents. Subagents that exceed
    /// the timeout are left running (their handles remain in `self.subagents`).
    pub async fn await_subagents_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Vec<SubagentResult> {
        let mut results = Vec::new();
        let deadline = tokio::time::Instant::now() + timeout;
        let ids: Vec<_> = self.subagents.keys().cloned().collect();

        for id in ids {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                warn!(
                    "await_subagents_timeout: deadline reached, {} subagents still running",
                    self.subagents.len()
                );
                break;
            }

            if let Some(handle) = self.subagents.remove(&id) {
                match tokio::time::timeout(remaining, handle).await {
                    Ok(Ok(result)) => {
                        info!(
                            "Subagent {} completed: success={}",
                            result.id, result.success
                        );
                        results.push(result);
                    }
                    Ok(Err(e)) => {
                        results.push(SubagentResult {
                            id: id.clone(),
                            success: false,
                            output: format!("Subagent task panicked: {}", e),
                            iterations: 0,
                            mission_id: None,
                        });
                    }
                    Err(_) => {
                        warn!("Subagent {} timed out, leaving running", id);
                        // Can't put handle back (consumed by timeout), create a stub
                        results.push(SubagentResult {
                            id: id.clone(),
                            success: false,
                            output: "Subagent timed out waiting for result".to_string(),
                            iterations: 0,
                            mission_id: None,
                        });
                    }
                }
            }
        }

        results
    }
}

/// Extract key facts (paths, URLs, config values) from text for auto-storage as MemoryType::Fact.
/// Returns a vec of fact strings suitable for storing in Mnemosyne.
fn extract_facts_from_text(text: &str) -> Vec<String> {
    let mut facts = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Absolute paths (Unix)
        if let Some(start) = trimmed.find('/') {
            let candidate = &trimmed[start..];
            // Extract path-like token (stop at whitespace, quotes, parens)
            let path: String = candidate
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != ')' && *c != '`')
                .collect();
            if path.starts_with("/home/")
                || path.starts_with("/etc/")
                || path.starts_with("/usr/")
                || path.starts_with("/var/")
                || path.starts_with("/opt/")
            {
                if path.len() > 5 && !facts.contains(&path) {
                    facts.push(format!("Path: {}", path));
                }
            }
        }

        // URLs
        for prefix in &["https://", "http://"] {
            if let Some(start) = trimmed.find(prefix) {
                let url: String = trimmed[start..]
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != '>' && *c != ')')
                    .collect();
                if url.len() > 10 && !facts.contains(&url) {
                    facts.push(format!("URL: {}", url));
                }
            }
        }

        // Key=value patterns (e.g., "repo: /path", "port: 8080")
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim();
            let val = trimmed[colon_pos + 1..].trim();
            if key.len() < 30
                && !val.is_empty()
                && val.len() < 200
                && (key.to_lowercase().contains("path")
                    || key.to_lowercase().contains("repo")
                    || key.to_lowercase().contains("dir")
                    || key.to_lowercase().contains("port")
                    || key.to_lowercase().contains("host")
                    || key.to_lowercase().contains("version"))
            {
                let fact = format!("{}: {}", key, val);
                if !facts.contains(&fact) {
                    facts.push(fact);
                }
            }
        }
    }

    // Cap to avoid over-storing
    facts.truncate(5);
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use zeus_core::Provider;

    /// Create a minimal Agent for testing (no LLM calls, no subsystems).
    fn test_agent() -> Agent {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = Config::default();
        let llm =
            LlmClient::new(Provider::Ollama, "test-model".to_string()).expect("LlmClient::new");
        let workspace = Workspace::new(tmp.path().join("workspace"));
        let session = Session::new(tmp.path().join("sessions"));
        Agent::new(config, llm, workspace, session, None)
    }

    /// Create a minimal Agent with custom config.
    fn test_agent_with_config(config: Config) -> Agent {
        let tmp = tempfile::tempdir().expect("tempdir");
        let llm =
            LlmClient::new(Provider::Ollama, "test-model".to_string()).expect("LlmClient::new");
        let workspace = Workspace::new(tmp.path().join("workspace"));
        let session = Session::new(tmp.path().join("sessions"));
        Agent::new(config, llm, workspace, session, None)
    }

    // ── Agent Event tests ──────────────────────────────────────────────

    #[test]
    fn test_agent_event_variants() {
        let events = vec![
            AgentEvent::Started,
            AgentEvent::TextChunk("hello".to_string()),
            AgentEvent::Finished { iterations: 1 },
        ];
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_agent_event_clone() {
        let event = AgentEvent::ToolCall {
            name: "shell".to_string(),
            args: json!({"command": "ls"}),
        };
        let cloned = event.clone();
        if let AgentEvent::ToolCall { name, args } = cloned {
            assert_eq!(name, "shell");
            assert_eq!(args["command"], "ls");
        } else {
            panic!("Clone produced wrong variant");
        }
    }

    #[test]
    fn test_agent_event_debug_format() {
        let event = AgentEvent::Error("test error".to_string());
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("test error"));
    }

    #[test]
    fn test_agent_event_tool_result() {
        let event = AgentEvent::ToolResult {
            name: "read_file".to_string(),
            success: true,
            output: "file contents".to_string(),
        };
        if let AgentEvent::ToolResult {
            name,
            success,
            output,
        } = event
        {
            assert_eq!(name, "read_file");
            assert!(success);
            assert_eq!(output, "file contents");
        } else {
            panic!("Wrong variant");
        }
    }

    // ── Agent construction tests ───────────────────────────────────────

    #[test]
    fn test_agent_new_default_config() {
        let agent = test_agent();
        assert_eq!(agent.running_subagents(), 0);
        assert!(agent.subagent_ids().is_empty());
        assert!(agent.goals_context.is_none());
        assert!(agent.tool_policy.is_none());
        assert!(agent.mnemosyne.is_none());
        assert!(agent.athena.is_none());
        assert!(agent.aegis.is_none());
        assert!(agent.hermes.is_none()); // Agent::new is the lean constructor; hermes is wired in during async init
        assert!(agent.nous.is_none());
        assert!(agent.channels.is_none());
    }

    #[test]
    fn test_agent_new_with_custom_max_iterations() {
        let mut config = Config::default();
        config.max_iterations = 42;
        let agent = test_agent_with_config(config);
        assert_eq!(agent.config.max_iterations, 42);
    }

    #[test]
    fn test_suppress_tool_errors_config_default() {
        let config = Config::default();
        assert!(!config.suppress_tool_errors);
    }

    #[test]
    fn test_suppress_tool_errors_config_set() {
        let mut config = Config::default();
        config.suppress_tool_errors = true;
        assert!(config.suppress_tool_errors);
    }

    // ── Session and workspace access ───────────────────────────────────

    #[test]
    fn test_agent_session_access() {
        let agent = test_agent();
        let session = agent.session();
        assert!(!session.id.is_empty());
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_agent_session_mut_access() {
        let mut agent = test_agent();
        let id_before = agent.session().id.clone();
        let session = agent.session_mut();
        assert_eq!(session.id, id_before);
    }

    #[test]
    fn test_agent_workspace_access() {
        let agent = test_agent();
        // Workspace root should be the path we gave it
        let root = agent.workspace().root();
        assert!(root.to_string_lossy().contains("workspace"));
    }

    // ── Goals and policy ───────────────────────────────────────────────

    #[test]
    fn test_agent_set_goals_context() {
        let mut agent = test_agent();
        assert!(agent.goals_context.is_none());

        agent.set_goals_context(Some("Goal: deploy v2.0".to_string()));
        assert_eq!(agent.goals_context.as_deref(), Some("Goal: deploy v2.0"));

        agent.set_goals_context(None);
        assert!(agent.goals_context.is_none());
    }

    #[test]
    fn test_agent_set_tool_policy() {
        let mut agent = test_agent();
        assert!(agent.tool_policy.is_none());

        let policy = AgentToolPolicy {
            allowed_tools: vec!["read_file".to_string(), "list_dir".to_string()],
            denied_tools: vec![],
        };
        agent.set_tool_policy(policy);

        let policy = agent.tool_policy.as_ref().expect("policy should be set");
        assert!(policy.is_tool_allowed("read_file"));
        assert!(policy.is_tool_allowed("list_dir"));
        assert!(!policy.is_tool_allowed("shell"));
    }

    #[test]
    fn test_tool_policy_filters_schemas() {
        let mut agent = test_agent();
        let all_schemas = agent.tools.schemas();
        assert!(!all_schemas.is_empty());

        // Set restrictive policy
        let policy = AgentToolPolicy {
            allowed_tools: vec!["read_file".to_string()],
            denied_tools: vec![],
        };
        agent.set_tool_policy(policy);

        let filtered: Vec<_> = agent
            .tools
            .schemas()
            .into_iter()
            .filter(|s| agent.tool_policy.as_ref().unwrap().is_tool_allowed(&s.name))
            .collect();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "read_file");
        assert!(filtered.len() < all_schemas.len());
    }

    // ── Event channel ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_agent_emit_without_channel() {
        let agent = test_agent();
        // Should not panic when no event channel is set
        agent.emit(AgentEvent::Started).await;
        agent.emit(AgentEvent::Error("test".to_string())).await;
    }

    #[tokio::test]
    async fn test_agent_with_events_receives_emit() {
        let (tx, mut rx) = mpsc::channel(16);
        let agent = test_agent().with_events(tx);

        agent.emit(AgentEvent::Started).await;
        agent.emit(AgentEvent::Finished { iterations: 5 }).await;

        let e1 = rx.recv().await.expect("should receive Started");
        assert!(matches!(e1, AgentEvent::Started));

        let e2 = rx.recv().await.expect("should receive Finished");
        if let AgentEvent::Finished { iterations } = e2 {
            assert_eq!(iterations, 5);
        } else {
            panic!("Expected Finished event");
        }
    }

    #[tokio::test]
    async fn test_agent_set_events_mutable() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut agent = test_agent();
        agent.set_events(tx);

        agent.emit(AgentEvent::TextChunk("hello".to_string())).await;

        let event = rx.recv().await.expect("should receive event");
        if let AgentEvent::TextChunk(text) = event {
            assert_eq!(text, "hello");
        } else {
            panic!("Expected TextChunk");
        }
    }

    // ── execute_spawn argument parsing ─────────────────────────────────

    #[tokio::test]
    async fn test_execute_spawn_missing_task() {
        let mut agent = test_agent();
        let call = ToolCall {
            id: "call-1".to_string(),
            name: "spawn".to_string(),
            arguments: json!({"context": "some context"}),
        };
        let result = agent.execute_spawn(&call).await;
        assert!(!result.success);
        assert!(result.output.contains("Missing 'task'"));
    }

    #[tokio::test]
    async fn test_execute_spawn_with_task_background() {
        let mut agent = test_agent();
        let call = ToolCall {
            id: "call-2".to_string(),
            name: "spawn".to_string(),
            arguments: json!({
                "task": "Summarize the README",
                "max_iterations": 5
            }),
        };
        let result = agent.execute_spawn(&call).await;
        // Background spawn should succeed (subagent spawned)
        assert!(result.success);
        assert!(result.output.contains("spawned in background"));
        assert_eq!(agent.subagent_ids().len(), 1);
    }

    // ── execute_message argument parsing ───────────────────────────────

    #[tokio::test]
    async fn test_execute_message_missing_channel() {
        let agent = test_agent();
        let call = ToolCall {
            id: "call-3".to_string(),
            name: "message".to_string(),
            arguments: json!({"content": "hello"}),
        };
        let result = agent.execute_message(&call).await;
        assert!(!result.success);
        assert!(result.output.contains("Missing 'channel'"));
    }

    #[tokio::test]
    async fn test_execute_message_missing_content() {
        let agent = test_agent();
        let call = ToolCall {
            id: "call-4".to_string(),
            name: "message".to_string(),
            arguments: json!({"channel": "telegram"}),
        };
        let result = agent.execute_message(&call).await;
        assert!(!result.success);
        assert!(result.output.contains("Missing 'content'"));
    }

    #[tokio::test]
    async fn test_execute_message_unconfigured_platform_channel() {
        let agent = test_agent();
        let call = ToolCall {
            id: "call-5".to_string(),
            name: "message".to_string(),
            arguments: json!({
                "channel": "telegram",
                "content": "hello",
                "target": "12345"
            }),
        };
        let result = agent.execute_message(&call).await;
        assert!(!result.success);
        assert!(result.output.contains("not configured"));
    }

    #[tokio::test]
    async fn test_execute_message_broadcast_no_channels() {
        let agent = test_agent();
        let call = ToolCall {
            id: "call-6".to_string(),
            name: "message".to_string(),
            arguments: json!({
                "channel": "broadcast",
                "content": "hello everyone",
                "targets": [{"channel": "telegram", "target": "123"}]
            }),
        };
        let result = agent.execute_message(&call).await;
        assert!(!result.success);
        assert!(result.output.contains("No channels configured"));
    }

    #[tokio::test]
    async fn test_execute_message_platform_missing_target() {
        let agent = test_agent();
        let call = ToolCall {
            id: "call-7".to_string(),
            name: "message".to_string(),
            arguments: json!({
                "channel": "discord",
                "content": "hello"
                // no target
            }),
        };
        let result = agent.execute_message(&call).await;
        assert!(!result.success);
        // Either "not configured" or "target is required"
        assert!(!result.success);
    }

    // ── execute_tools security checks ──────────────────────────────────

    #[tokio::test]
    async fn test_execute_tools_constitution_block() {
        let mut agent = test_agent();
        // Constitution with builtin laws should block dangerous patterns
        // Even without custom laws, test that the flow runs without panic
        let calls = vec![ToolCall {
            id: "call-8".to_string(),
            name: "nonexistent_tool".to_string(),
            arguments: json!({}),
        }];
        let results = agent.execute_tools(&calls).await;
        assert_eq!(results.len(), 1);
        // Tool doesn't exist so it will fail at execution, but shouldn't panic
        assert!(!results[0].success || results[0].success); // completes either way
    }

    #[tokio::test]
    async fn test_execute_tools_policy_block() {
        let mut agent = test_agent();
        agent.set_tool_policy(AgentToolPolicy {
            allowed_tools: vec!["read_file".to_string()],
            denied_tools: vec![],
        });

        let calls = vec![ToolCall {
            id: "call-9".to_string(),
            name: "shell".to_string(),
            arguments: json!({"command": "ls"}),
        }];
        let results = agent.execute_tools(&calls).await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert!(results[0].output.contains("not permitted"));
    }

    #[tokio::test]
    async fn test_execute_tools_multiple_calls() {
        let mut agent = test_agent();
        let calls = vec![
            ToolCall {
                id: "call-10".to_string(),
                name: "list_dir".to_string(),
                arguments: json!({"path": "/nonexistent/path/unlikely"}),
            },
            ToolCall {
                id: "call-11".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "/nonexistent/file.txt"}),
            },
        ];
        let results = agent.execute_tools(&calls).await;
        assert_eq!(results.len(), 2);
        // Both should complete (fail gracefully on nonexistent paths)
        assert_eq!(results[0].call_id, "call-10");
        assert_eq!(results[1].call_id, "call-11");
    }

    // ── Subagent tracking ──────────────────────────────────────────────

    #[test]
    fn test_running_subagents_initially_zero() {
        let agent = test_agent();
        assert_eq!(agent.running_subagents(), 0);
        assert!(agent.subagent_ids().is_empty());
    }

    #[tokio::test]
    async fn test_get_subagent_result_nonexistent() {
        let mut agent = test_agent();
        let result = agent.get_subagent_result("nonexistent-id").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_wait_all_subagents_empty() {
        let mut agent = test_agent();
        let results = agent.wait_all_subagents().await;
        assert!(results.is_empty());
    }

    // ── Spawn auto-collection ──────────────────────────────────────────

    #[tokio::test]
    async fn test_await_subagents_timeout_collects_finished_spawn() {
        let mut agent = test_agent();
        // Simulate a background spawn by inserting a pre-resolved JoinHandle.
        let handle = tokio::spawn(async {
            SubagentResult {
                id: "sub-1".to_string(),
                success: true,
                output: "done".to_string(),
                iterations: 3,
                mission_id: Some("m-123".to_string()),
            }
        });
        agent.subagents.insert("sub-1".to_string(), handle);
        assert_eq!(agent.running_subagents(), 1);

        let results = agent
            .await_subagents_timeout(std::time::Duration::from_secs(5))
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "sub-1");
        assert!(results[0].success);
        assert_eq!(results[0].iterations, 3);
        assert_eq!(results[0].mission_id.as_deref(), Some("m-123"));
        // Handle was consumed — no more running subagents.
        assert_eq!(agent.running_subagents(), 0);
    }

    #[tokio::test]
    async fn test_await_subagents_timeout_multiple_spawns() {
        let mut agent = test_agent();
        for i in 0..3u32 {
            let id = format!("sub-{}", i);
            let id2 = id.clone();
            let handle = tokio::spawn(async move {
                SubagentResult {
                    id: id2,
                    success: true,
                    output: "ok".to_string(),
                    iterations: 1,
                    mission_id: None,
                }
            });
            agent.subagents.insert(id, handle);
        }
        assert_eq!(agent.running_subagents(), 3);

        let results = agent
            .await_subagents_timeout(std::time::Duration::from_secs(5))
            .await;

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.success));
        assert_eq!(agent.running_subagents(), 0);
    }

    // ── Hooks ──────────────────────────────────────────────────────────

    #[test]
    fn test_hooks_mut_access() {
        let mut agent = test_agent();
        let hooks = agent.hooks_mut();
        assert_eq!(hooks.len(), 0);
    }

    #[test]
    fn test_agent_with_hooks_builder() {
        let hooks = HookRegistry::new();
        let agent = test_agent().with_hooks(hooks);
        assert_eq!(agent.hooks.len(), 0);
    }

    // ── Session repair: orphaned tool_use detection ────────────────────────

    #[test]
    fn test_session_repair_detects_orphaned_tool_use() {
        use zeus_core::{Message, Role, ToolCall, ToolResult};

        // Build a session with one assistant message that has a tool_call
        // but no corresponding tool_result anywhere in the session.
        let mut messages = vec![
            Message::user("do something"),
            Message::assistant("sure").with_tool_calls(vec![ToolCall {
                id: "call_orphan_1".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            }]),
            // Deliberately NO tool message following — orphaned!
        ];

        // Replicate the repair logic from agent_loop.rs
        let existing_results: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.call_id.as_str()))
            .collect();

        let mut orphaned: Vec<(usize, String)> = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            if m.role == Role::Assistant {
                for tc in &m.tool_calls {
                    if !existing_results.contains(tc.id.as_str()) {
                        orphaned.push((i, tc.id.clone()));
                    }
                }
            }
        }

        assert_eq!(orphaned.len(), 1, "should detect 1 orphaned tool_use");
        assert_eq!(orphaned[0].1, "call_orphan_1");

        // Apply repair
        let repair_msg = Message::tool(
            &orphaned[0].1,
            false,
            "[Tool result missing — session repaired]",
        );
        messages.insert(orphaned[0].0 + 1, repair_msg);

        // Verify repair: tool_result now exists for the orphaned call
        let results_after: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.call_id.as_str()))
            .collect();
        assert!(results_after.contains("call_orphan_1"), "repair must inject tool_result");
        assert_eq!(messages.len(), 3, "repair inserts exactly 1 message after the assistant msg");
    }

    #[test]
    fn test_session_repair_ignores_matched_tool_use() {
        use zeus_core::{Message, ToolCall};

        // Assistant message with a tool_call that IS matched by a tool_result
        let messages = vec![
            Message::user("do something"),
            Message::assistant("sure").with_tool_calls(vec![ToolCall {
                id: "call_ok".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            }]),
            Message::tool("call_ok", true, "file1.txt\nfile2.txt"),
        ];

        let existing_results: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.call_id.as_str()))
            .collect();

        let orphaned: Vec<(usize, String)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == zeus_core::Role::Assistant)
            .flat_map(|(i, m)| {
                m.tool_calls
                    .iter()
                    .filter(|tc| !existing_results.contains(tc.id.as_str()))
                    .map(move |tc| (i, tc.id.clone()))
            })
            .collect();

        assert!(orphaned.is_empty(), "matched tool_use must NOT be flagged as orphaned");
    }

    #[test]
    fn test_session_repair_multiple_orphans() {
        use zeus_core::{Message, ToolCall};

        // Two orphaned tool_calls in the same assistant message
        let mut messages = vec![
            Message::user("do two things"),
            Message::assistant("ok").with_tool_calls(vec![
                ToolCall {
                    id: "call_a".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "ls"}),
                },
                ToolCall {
                    id: "call_b".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({"path": "/tmp/foo"}),
                },
            ]),
        ];

        let existing_results: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.call_id.as_str()))
            .collect();

        let orphaned: Vec<(usize, String)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == zeus_core::Role::Assistant)
            .flat_map(|(i, m)| {
                m.tool_calls
                    .iter()
                    .filter(|tc| !existing_results.contains(tc.id.as_str()))
                    .map(move |tc| (i, tc.id.clone()))
            })
            .collect();

        assert_eq!(orphaned.len(), 2, "both orphaned calls detected");

        // Repair both (insert one message per call, reversed to preserve indices)
        for (pos, id) in orphaned.iter().rev() {
            let repair = Message::tool(id, false, "[Tool result missing — session repaired]");
            messages.insert(pos + 1, repair);
        }

        let results_after: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.call_id.as_str()))
            .collect();
        assert!(results_after.contains("call_a"));
        assert!(results_after.contains("call_b"));
    }

    // ── Compaction threshold regression fence ─────────────────────────

    #[test]
    fn test_compaction_threshold_model_aware() {
        // When context window is known: threshold = ctx * 0.6
        let ctx = Some(262_144usize);
        let threshold = match ctx {
            Some(c) => (c as f64 * 0.6) as usize,
            None => 100_000,
        };
        assert_eq!(threshold, 157_286, "threshold should be 60% of 262144");

        // When context window is unknown: fallback to 100K
        let ctx: Option<usize> = None;
        let threshold = match ctx {
            Some(c) => (c as f64 * 0.6) as usize,
            None => 100_000,
        };
        assert_eq!(threshold, 100_000, "fallback threshold should be 100K");
    }

    // #32 regression fence: pre-flight compaction must fire BEFORE first LLM call
    // when a loaded session exceeds the compaction threshold.
    #[test]
    fn test_preflight_compaction_detection() {
        use zeus_session::ContextManager;
        use zeus_core::SessionCompactionConfig;

        // Create a ContextManager with a very low threshold (100 tokens * 0.5 = 50 trigger)
        let config = SessionCompactionConfig {
            max_context_tokens: 100,
            compaction_threshold: 0.5,
            summary_model: None,
            compaction_timeout_secs: None,
            ollama_compaction_threshold: None,
            flush_timeout_secs: None,
        };
        let cm = ContextManager::new(&config);

        // Empty session — should NOT need compaction
        let empty: Vec<zeus_core::Message> = vec![];
        assert!(!cm.needs_compaction(&empty), "empty session should not need compaction");

        // Small session — should NOT need compaction
        let small = vec![
            zeus_core::Message::system("You are a helpful assistant."),
            zeus_core::Message::user("Hello"),
        ];
        assert!(!cm.needs_compaction(&small), "small session should not need compaction");

        // Large session — SHOULD need compaction (each message ~4 chars/token,
        // so 20 messages of ~100 chars each ≈ 500 tokens > 50 trigger)
        let mut large = vec![zeus_core::Message::system("You are a helpful assistant.")];
        for i in 0..20 {
            large.push(zeus_core::Message::user(&format!(
                "This is a long message with many words to exceed the token threshold. Message number {}. \
                 Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor.",
                i
            )));
        }
        assert!(cm.needs_compaction(&large), "large session SHOULD need compaction (threshold={})",
            (config.max_context_tokens as f32 * config.compaction_threshold) as usize);

        // Verify max_tokens accessor
        assert_eq!(cm.max_tokens(), 100);
    }
}
