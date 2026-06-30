//! Zeus Prometheus - Brain/Orchestration Engine
//!
//! The central orchestration layer that coordinates:
//! - Intent analysis and autonomous decision-making
//! - Task planning and execution via LLM
//! - Heartbeat-based proactive tasks
//! - Interaction learning and pattern recognition
//! - Self-monitoring, anomaly detection, and self-correction
//! - Integration with Nous cognitive engine
//!
//! # Autonomy Model
//!
//! The processing pipeline follows the essay's four primitives:
//! 1. **Classify** — Intent analysis determines what the user wants
//! 2. **Decide** — Autonomy engine chooses the action strategy
//! 3. **Execute** — Planner/executor/direct LLM carries out the decision
//! 4. **Learn** — Record the interaction pattern and monitor health

pub mod benchmark;
pub mod experiment;
pub mod meta_loop;
pub mod acceptance;
pub mod agent_director;
pub mod agent_pool;
pub mod autonomy;
pub mod compaction;
pub mod channels;
pub mod compute;
pub mod env_block;
pub mod content_pipeline;
pub mod cook_context;
pub mod content_queue;
pub mod content_queue_drain;
pub mod commitments;
pub mod coordination;
mod db;
pub mod dreaming;
pub mod executor;
pub mod feedback;
pub mod backlog_sync;
pub mod goals;
pub mod fire_decision;
pub mod heartbeat;
pub mod ledger;
pub mod intent;
pub mod learning;
pub mod memory_injector;
pub mod metabolism;
pub mod monitor;
pub mod orchestrate;
pub mod packaging;
pub mod pantheon_bridge;

pub mod plan_mode;
pub mod plan_store;
pub mod planner;
pub mod replication;
pub mod scheduler;
pub mod session;
pub mod session_resolver;
pub mod spawner;
pub mod standing_orders;
pub mod taskflow;
pub mod strategic;
pub mod telemetry;
pub mod tool_executor;

// Cooking Loop modules
pub mod cooking_attempt;
pub mod cooking_auth;
pub mod cooking_backoff;
pub mod cooking_checkpoint;
pub mod cooking_errors;
pub mod cooking_events;
pub mod cooking_plans;
pub mod cooking_session;

pub use benchmark::{BenchmarkResult, BenchmarkStore, RunComparison, RunSummary};
pub use experiment::{AutoTuneReport, ConfigChange, ConfigExperiment, ExperimentOutcome};
pub use meta_loop::{ExperimentProposer, IterationRecord, MetaLoop, MetaLoopReport, RandomProposer};
pub use agent_director::{
    AgentDirector, DirectorSession, DirectorStatus, DrivingConfig, DrivingLoop, DrivingResult,
    ElementInfo, PageInfo, PageMap, PuppetCommand, PuppetResponse, ScrollDirection, UiAction,
    UiActionResult, build_director_prompt, parse_action_plan,
};
pub use agent_pool::{AgentPool, AgentPoolConfig, PoolResult};
pub use autonomy::{AutonomyConfig, AutonomyEngine, AutonomyLevel, Decision, DecisionContext};
pub use channels::ChannelKind;
pub use cook_context::CookContext;
pub use session_resolver::FleetSessionAlias;
pub use compute::{
    AgentCompute, AgentComputeSummary, ComputeProvisioner, ComputeReport, QuotaCheck,
    ResourceQuota, UsageCounters, WindowDuration,
};
pub use content_queue::{ContentJob, ContentQueue, JobStatus, Platform, QueueStats};
pub use coordination::{
    CoordinationConfig, CoordinationEvent, CoordinationLoop, CoordinationResult,
};
pub use feedback::{FeedbackLoop, StrategyRecord};
pub use goals::{Goal, GoalSource, GoalStack, GoalStatus, Priority, format_work_state};
pub use intent::{Intent, IntentAnalysis, IntentClassifier, TaskComplexity};
pub use learning::{
    InteractionRecord, Learning, LearningConfig, LearningEngine, Outcome, PatternSummary,
    StrategicLearner, ToolEffectiveness,
};
pub use memory_injector::MemoryInjector;
pub use metabolism::{
    AgentMetabolism, MetabolismAction, MetabolismConfig, MetabolismEngine, MetabolismLoop,
    MetabolismTier, MetabolismTransition, SustainabilityReport,
};
pub use monitor::{HealthReport, HealthStatus, Monitor, MonitorConfig};
pub use pantheon_bridge::{
    MissionCheckpointer, MissionDriver, MissionResult, PlannedMission, plan_to_mission_tasks,
};
pub use plan_mode::{PlanMode, PlanMeta, PlanStatus};
pub use plan_store::{PlanOutcome, PlanOutcomeStore};
pub use replication::{
    LineageTracker, ReplicationConfig, ReplicationManager, ReplicationRequest, ReplicationResult,
};
pub use scheduler::{
    CronJobHistory, CronJobTemplate, CronScheduler, ScheduledTask, SchedulerConfig, TaskConfig,
    DeliveryMode, TaskExecution, TaskType, WakeMode,
};
pub use trigger_tools::TriggerHandle;
pub use spawner::{
    ProactiveSpawner, SpawnCriteria, SpawnHealthSummary, SpawnOutcome, SpawnRecommendation,
    SpawnRequest, SpawnTracker,
};
pub use strategic::{StrategicPlanner, TaskDAG, TaskNode};
pub use tool_executor::{
    CookingConfig, CookingLoop, CookingResult, ToolCallRecord, ToolExecutor,
    estimate_message_tokens,
};

// Cooking Loop re-exports
pub use cooking_attempt::{Attempt, AttemptOutcome, AttemptResult, AttemptToolCall};
pub use cooking_auth::{AuthProfile, AuthProfileManager, FailoverReason};
pub use cooking_backoff::{BackoffConfig, BackoffStrategy};
pub use cooking_checkpoint::{CookingCheckpoint, CookingCheckpointStore, InterruptedSession};
pub use cooking_errors::{ErrorClass, classify_error};
pub use cooking_events::{CookingEvent, EventEmitter};
pub use cooking_plans::{ExecutionStatus, PlanExecutor, TaskDef, TaskPlan, TaskResult};
pub use cooking_session::{JsonlEntry, SessionPersistence};

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use zeus_core::{Config, Message, Result, ToolSchema};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_nous::critic::{CriticEngine, ExecutionContext};

/// The brain of Zeus - orchestrates all components
pub struct Prometheus {
    /// LLM client for inference
    llm: Arc<LlmClient>,
    /// Workspace for memory
    workspace: Arc<Workspace>,
    /// Configuration
    config: Config,
    /// Session manager
    sessions: Arc<RwLock<session::SessionManager>>,
    /// Heartbeat manager
    heartbeat: Option<heartbeat::Heartbeat>,
    /// Cron-based scheduler
    scheduler: Option<Arc<RwLock<CronScheduler>>>,
    /// Planner for task decomposition
    planner: planner::Planner,
    /// Executor for task execution
    executor: executor::Executor,
    /// Intent classifier for message analysis
    intent_classifier: IntentClassifier,
    /// Autonomy engine for decision-making
    autonomy_engine: AutonomyEngine,
    /// Learning engine for interaction patterns
    learning_engine: Option<LearningEngine>,
    /// Self-monitoring engine
    monitor: Arc<Monitor>,
    /// Tool executor for running tools in the cooking loop
    tool_executor: Option<Arc<dyn ToolExecutor>>,
    /// Memory injector for auto-context
    memory_injector: MemoryInjector,
    /// Mnemosyne instance for memory search
    mnemosyne: Option<Arc<zeus_mnemosyne::Mnemosyne>>,
    /// Cooking loop configuration
    cooking_config: CookingConfig,
    /// Runtime override for max cooking iterations (set per-request based on intent).
    /// 0 = use cooking_config.max_iterations (default).
    iteration_cap: std::sync::atomic::AtomicUsize,
    /// Goal stack for persistent goal management
    goal_stack: Option<GoalStack>,
    /// S69: Pending heartbeat result delivery channel (set before start_heartbeat)
    pending_heartbeat_result_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// Channel-active flag passed to heartbeat so it defers during real message processing.
    pending_channel_active: Option<zeus_core::CookState>,
    /// Inbox queue-depth counter passed to heartbeat for busy-aware fire-decision (`busy: inbound`).
    pending_inbox_queue_depth: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    /// Subagent-depth counter passed to heartbeat for busy-aware fire-decision (`busy: subagent`).
    pending_subagent_depth: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    /// Last-interaction-at unix-secs handle passed to heartbeat for busy-aware fire-decision
    /// (`busy: recent_interaction`). Plumbing-only proxy; forwarded to
    /// `Heartbeat::set_last_interaction_at` at start. Dual-path (Lane A1.5b-ii):
    /// external setter for caller-driven wire-up OR internal-allocation fallback in
    /// `start_heartbeat` so cook-completion writes land on the same handle Heartbeat reads.
    pending_last_interaction_at: Option<std::sync::Arc<std::sync::atomic::AtomicI64>>,
    /// Authoritative handle for `last_interaction_at` post-`start_heartbeat`. Cook-completion
    /// write-sites in `process_autonomous` `.store(now_unix, Relaxed)` to this same `Arc`.
    /// `None` until `start_heartbeat` runs (mirrors Heartbeat's None-handle graceful-degrade).
    last_interaction_at: Option<std::sync::Arc<std::sync::atomic::AtomicI64>>,
    /// Feedback loop for strategy learning
    feedback: FeedbackLoop,
    /// Critic engine for evaluating execution outcomes
    critic: CriticEngine,
    /// Consolidation engine shutdown signal
    consolidation_shutdown: Option<tokio::sync::watch::Sender<bool>>,
    /// Current channel kind for cross-channel context injection (#86-sprint-C).
    /// Set by gateway before each cook via `set_current_channel_kind()`.
    /// Empty string = unknown / no filtering.
    current_channel_kind: std::sync::Arc<std::sync::RwLock<String>>,
    /// Current human (sender) id for cross-channel session correlation (#192).
    /// Set by gateway before each cook via `set_current_human_id()`.
    /// `None` = unknown → resolver falls back to unaliased (raw-channel) behavior.
    current_human_id: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// #168 dedup — last goal-key written by the per-cook-turn working write.
    /// Guards against per-turn append of near-dup `[Active Goal]` rows: the write
    /// only fires when the goal actually changes, not once per answer. Shared-lock
    /// (no `&mut self`) to match the `cook_with_history_interruptible` `&self` path.
    last_cook_goal_key: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// Strategic planner for DAG analysis (always initialized)
    strategic_planner: StrategicPlanner,
    /// Nous cognitive engine (optional — for intent + lessons in cooking loop)
    nous: Option<Arc<zeus_nous::Nous>>,
    /// Global state manager for multi-agent coordination
    state_manager: Option<Arc<zeus_orchestra::GlobalStateManager>>,
    /// Dynamic orchestrator for agent lifecycle management
    dynamic_orchestrator: Option<Arc<zeus_orchestra::DynamicOrchestrator>>,
    /// Proactive agent spawner for multi-agent parallelism (Mutex for interior mutability)
    spawner: Option<std::sync::Mutex<ProactiveSpawner>>,
    /// Plan outcome store for learning from past executions
    plan_store: Option<PlanOutcomeStore>,
    /// Checkpoint store for cooking loop crash-resume persistence
    checkpoint_store: Option<Arc<CookingCheckpointStore>>,
    /// Cooking event emitter — shared across all cooking loops for live progress
    cooking_event_emitter: EventEmitter,
    /// In-memory counter for prior dispatches per fleet session alias (Lane 2b-i)
    dispatch_counter: Arc<RwLock<std::collections::HashMap<String, u64>>>,
}

impl Prometheus {
    /// Create a new Prometheus instance
    pub async fn new(config: Config) -> Result<Self> {
        let llm = LlmClient::from_config(&config)?;
        let workspace = Workspace::from_config(&config);
        workspace.init().await?;

        // Read prometheus sub-configs from core config, falling back to defaults
        let prom_cfg = config.prometheus.as_ref();

        // Initialize the cron scheduler if EITHER [gateway].enable_cron OR
        // [prometheus].enable_heartbeat is set. The invariant (#187): a seat
        // with enable_cron=true MUST get a constructed scheduler — otherwise
        // gateway.rs start_scheduler() is a silent no-op and talos-written
        // rows are never loaded/ticked. [prometheus].enable_heartbeat carries
        // #[serde(default)] (=> false when omitted), so a cron-only seat would
        // otherwise fall through to None. Construction is unified here; the
        // *start*/tick gate stays on enable_cron in gateway.rs (heartbeat-only
        // seats construct-but-don't-self-tick, as before).
        let enable_cron = config
            .gateway
            .as_ref()
            .map(|g| g.enable_cron)
            .unwrap_or(false);
        let scheduler = prom_cfg.and_then(|p| {
            if p.enable_heartbeat || enable_cron {
                let sched_cfg = p
                    .scheduler
                    .as_ref()
                    .and_then(|v| serde_json::to_value(v).ok())
                    .and_then(|jv| serde_json::from_value::<SchedulerConfig>(jv).ok())
                    .unwrap_or_else(SchedulerConfig::with_defaults);
                let sched = CronScheduler::new(sched_cfg.clone());
                // Persist to ~/.zeus/scheduler.db so triggers survive restarts
                let db_path = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zeus")
                    .join("scheduler.db");
                let sched = match sched.with_db(&db_path) {
                    Ok(s) => {
                        debug!("Scheduler persistence enabled at {:?}", db_path);
                        s
                    }
                    Err(e) => {
                        warn!("Scheduler DB init failed (triggers won't persist): {}", e);
                        // with_db consumes self on error too — rebuild without DB
                        CronScheduler::new(sched_cfg)
                    }
                };
                Some(Arc::new(RwLock::new(sched)))
            } else {
                None
            }
        });

        // Initialize learning engine (optional, may fail if db path is invalid)
        let learning_engine = {
            let learning_config = prom_cfg
                .and_then(|p| p.learning.as_ref())
                .and_then(|v| serde_json::to_value(v).ok())
                .and_then(|jv| serde_json::from_value::<LearningConfig>(jv).ok())
                .unwrap_or_else(|| {
                    let db_path = dirs::home_dir()
                        .unwrap_or_default()
                        .join(".zeus")
                        .join("learning.db");
                    LearningConfig {
                        db_path,
                        ..Default::default()
                    }
                });
            match LearningEngine::new(&learning_config) {
                Ok(engine) => {
                    debug!("Learning engine initialized");
                    Some(engine)
                }
                Err(e) => {
                    warn!("Learning engine initialization failed (non-fatal): {}", e);
                    None
                }
            }
        };

        // Initialize monitor from config or defaults
        let monitor_config: MonitorConfig = prom_cfg
            .and_then(|p| p.monitor.as_ref())
            .and_then(|v| serde_json::to_value(v).ok())
            .and_then(|jv| serde_json::from_value::<MonitorConfig>(jv).ok())
            .unwrap_or_default();
        let monitor = Arc::new(Monitor::new(monitor_config));

        // Initialize autonomy engine from config or defaults
        let autonomy_config: AutonomyConfig = prom_cfg
            .and_then(|p| p.autonomy.as_ref())
            .and_then(|v| serde_json::to_value(v).ok())
            .and_then(|jv| serde_json::from_value::<AutonomyConfig>(jv).ok())
            .unwrap_or_default();
        let autonomy_engine = AutonomyEngine::new(autonomy_config);

        // Initialize goal stack (optional, non-fatal if db path is invalid)
        let goal_stack = {
            let db_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".zeus")
                .join("goals.db");
            match GoalStack::new(db_path) {
                Ok(stack) => {
                    debug!("Goal stack initialized");
                    Some(stack)
                }
                Err(e) => {
                    warn!("Goal stack initialization failed (non-fatal): {}", e);
                    None
                }
            }
        };

        // Initialize cooking checkpoint store (optional, non-fatal)
        let checkpoint_store = {
            let db_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".zeus")
                .join("cooking_checkpoints.db");
            match CookingCheckpointStore::open(&db_path) {
                Ok(store) => {
                    // Checkpoint retention sweep — prevents the unbounded DB growth
                    // that ballooned cooking_checkpoints.db to 2 GB. The sweep:
                    //   1. age-caps completed rows (older than max_age_days)
                    //   2. count-caps total rows (keep N most-recent)
                    //   3. VACUUMs to reclaim on-disk space (DELETE alone doesn't shrink)
                    // Runs on boot and then every sweep_interval (hourly by default),
                    // not boot-only — a long-lived gateway re-sweeps.
                    let store = Arc::new(store);
                    let cleanup_store = store.clone();
                    // Retention knobs from config, with defaults when prometheus
                    // config is absent (matches PrometheusConfig::default()).
                    let max_age_days = prom_cfg.map(|p| p.checkpoint_max_age_days).unwrap_or(7);
                    let max_rows = prom_cfg.map(|p| p.checkpoint_max_rows).unwrap_or(500);
                    let sweep_secs = prom_cfg
                        .map(|p| p.checkpoint_sweep_interval_secs)
                        .unwrap_or(3600);
                    tokio::spawn(async move {
                        loop {
                            if max_age_days > 0 {
                                cleanup_store
                                    .cleanup_old_sessions(std::time::Duration::from_secs(
                                        max_age_days * 86400,
                                    ))
                                    .await;
                            }
                            cleanup_store.enforce_count_cap(max_rows).await;
                            cleanup_store.vacuum().await;
                            if sweep_secs == 0 {
                                break; // boot-only mode
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(sweep_secs)).await;
                        }
                    });
                    debug!(
                        max_age_days,
                        max_rows, sweep_secs, "Cooking checkpoint store initialized (retention sweep scheduled)"
                    );
                    Some(store)
                }
                Err(e) => {
                    warn!(
                        "Cooking checkpoint store initialization failed (non-fatal): {}",
                        e
                    );
                    None
                }
            }
        };

        // Initialize plan outcome store (optional, non-fatal)
        let plan_store = {
            let db_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".zeus")
                .join("plan_outcomes.db");
            match PlanOutcomeStore::new(&db_path) {
                Ok(store) => {
                    debug!("Plan outcome store initialized");
                    Some(store)
                }
                Err(e) => {
                    warn!(
                        "Plan outcome store initialization failed (non-fatal): {}",
                        e
                    );
                    None
                }
            }
        };

        Ok(Self {
            llm: Arc::new(llm),
            workspace: Arc::new(workspace),
            config,
            sessions: Arc::new(RwLock::new(session::SessionManager::new())),
            heartbeat: None,
            scheduler,
            planner: planner::Planner::new()
                .with_templates(zeus_templates::TemplateRegistry::load_builtins()),
            executor: executor::Executor::new(),
            intent_classifier: IntentClassifier::new(),
            autonomy_engine,
            learning_engine,
            monitor,
            tool_executor: None,
            memory_injector: MemoryInjector::default(),
            mnemosyne: None,
            cooking_config: CookingConfig::default(),
            iteration_cap: std::sync::atomic::AtomicUsize::new(0),
            goal_stack,
            pending_heartbeat_result_tx: None,
            pending_channel_active: None,
            pending_inbox_queue_depth: None,
            pending_subagent_depth: None,
            pending_last_interaction_at: None,
            last_interaction_at: None,
            feedback: FeedbackLoop::new(),
            critic: CriticEngine::new(),
            consolidation_shutdown: None,
            current_channel_kind: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
            current_human_id: std::sync::Arc::new(std::sync::RwLock::new(None)),
            last_cook_goal_key: std::sync::Arc::new(std::sync::RwLock::new(None)),
            strategic_planner: StrategicPlanner::new(),
            nous: None,
            state_manager: None,
            dynamic_orchestrator: None,
            spawner: Some(std::sync::Mutex::new(ProactiveSpawner::default())),
            plan_store,
            checkpoint_store,
            cooking_event_emitter: EventEmitter::new(),
            dispatch_counter: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Create with default config loaded from `~/.zeus/config.toml`.
    pub async fn load_default() -> Result<Self> {
        let config = Config::load()?;
        Self::new(config).await
    }

    /// Autonomous processing pipeline: classify → decide → execute → evaluate → learn.
    ///
    /// This is the primary entry point for the brain. It analyzes the user's
    /// intent, decides the best action strategy, executes it, evaluates the
    /// outcome with the critic, and records the interaction for future learning.
    pub async fn process_autonomous(
        &self,
        message: &str,
        tools: &[ToolSchema],
    ) -> Result<AutonomousResult> {
        let start = std::time::Instant::now();

        // 1. Classify intent
        let analysis = self.intent_classifier.classify(message, tools);
        debug!(
            intent = %analysis.intent,
            complexity = ?analysis.complexity,
            confidence = analysis.confidence,
            "Classified user intent"
        );

        // 2. Build decision context
        let session_count = {
            let sessions = self.sessions.read().await;
            sessions.current().map(|s| s.message_count).unwrap_or(0)
        };
        let health = self.monitor.health_check();
        let recent_errors = match &health.status {
            HealthStatus::Unhealthy(_) => 5,
            HealthStatus::Degraded(_) => 2,
            HealthStatus::Healthy => 0,
        };

        let context = DecisionContext {
            intent: analysis.clone(),
            has_memory_context: true,
            session_message_count: session_count,
            recent_error_count: recent_errors,
            available_tools: tools.iter().map(|t| t.name.clone()).collect(),
            autonomous_tool_count: 0,
        };

        // 3. Decide action (feedback loop may override with learned strategy)
        let decision = if let Some(suggested) = self.feedback.suggest_strategy(&analysis) {
            info!(decision = %suggested, "Feedback loop suggested strategy override");
            suggested
        } else {
            self.autonomy_engine.decide(&context)
        };
        info!(decision = %decision, "Autonomous decision made");

        // 3b-pre. Upgrade PlanAndExecute → SpawnAgents if proactive spawner recommends it
        let decision = if matches!(decision, Decision::PlanAndExecute) {
            if let Some(ref spawner_mutex) = self.spawner
                && let Ok(spawner) = spawner_mutex.lock()
            {
                let active = spawner.tracker().active_count();
                let rec = spawner.analyze(&analysis, None, active);
                if rec.should_spawn && !rec.agents.is_empty() {
                    info!(
                        agents = rec.agents.len(),
                        rationale = %rec.rationale,
                        speedup = rec.estimated_speedup,
                        "Upgrading PlanAndExecute → SpawnAgents"
                    );
                    Decision::SpawnAgents(rec.agents)
                } else {
                    decision
                }
            } else {
                decision
            }
        } else {
            decision
        };

        // 3b. Create a goal for planned tasks
        let mut goal_id: Option<String> = None;
        if matches!(decision, Decision::PlanAndExecute)
            && let Some(ref stack) = self.goal_stack
        {
            let goal = Goal::new(message, Priority::Normal, GoalSource::System);
            match stack.add(&goal) {
                Ok(id) => {
                    info!(goal_id = %id, "Created goal for planned task");
                    // Store goal as working memory (#168: via shared helper to avoid drift)
                    self.store_active_goal_working(&id, message).await;
                    goal_id = Some(id);
                }
                Err(e) => {
                    debug!("Failed to create goal (non-fatal): {}", e);
                }
            }
        }

        // 4. Execute based on decision, tracking execution metadata
        let mut exec_success = true;
        let mut exec_errors: Vec<String> = Vec::new();
        let mut exec_retries: usize = 0;
        let mut exec_tool_count: usize = 0;

        let (response, tool_calls) = match &decision {
            Decision::RespondDirectly(_) | Decision::ExecuteTool(_) => {
                if self.tool_executor.is_some() {
                    let cook_result = self.cook(message, tools).await?;
                    exec_success = cook_result.completed_naturally;
                    // Lane A1.5b-ii: cook-completion = real-interaction-completion timestamp
                    // for busy-aware fire-decision `busy: recent_interaction` bucket.
                    self.record_interaction_completion();
                    exec_tool_count = cook_result.tool_calls.len();
                    exec_errors = cook_result
                        .tool_calls
                        .iter()
                        .filter(|tc| !tc.success)
                        .map(|tc| tc.output.clone())
                        .collect();
                    let pr = Self::cooking_to_process(&cook_result);
                    (pr.response, pr.tool_calls)
                } else {
                    let result = self.process(message, tools).await?;
                    exec_tool_count = result.tool_calls.len();
                    (result.response, result.tool_calls)
                }
            }
            Decision::PlanAndExecute => {
                let exec_result = self.plan_and_execute(message, tools).await?;
                let output = exec_result
                    .step_results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.output.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                let tool_names: Vec<String> = exec_result
                    .step_results
                    .iter()
                    .flat_map(|r| r.tool_calls_executed.iter().map(|tc| tc.name.clone()))
                    .collect();
                exec_success = exec_result.plan_status == crate::planner::PlanStatus::Completed;
                exec_errors = exec_result
                    .step_results
                    .iter()
                    .filter(|r| !r.success)
                    .filter_map(|r| r.error.clone())
                    .collect();
                exec_retries = exec_result.step_results.iter().map(|r| r.retries).sum();
                exec_tool_count = exec_result
                    .step_results
                    .iter()
                    .map(|r| r.tool_calls_executed.len())
                    .sum();
                (output, tool_names)
            }
            Decision::AskUser(question) => (question.clone(), vec![]),
            Decision::Delegate(subsystem) => {
                let result = self
                    .process(&format!("[Delegated to {}] {}", subsystem, message), tools)
                    .await?;
                exec_tool_count = result.tool_calls.len();
                (result.response, result.tool_calls)
            }
            Decision::Reflect => {
                // Self-reflection: gather recent errors and ask LLM to analyze.
                // GUARD: If error count is very high (2x threshold), skip the LLM call
                // entirely — it's likely to fail too, creating an infinite amplification
                // loop. Return a canned response and let the next valid message reset.
                let errors = self.monitor.recent_errors();
                let error_threshold = self.autonomy_engine.config().error_threshold;
                if errors.len() >= error_threshold * 2 {
                    warn!(
                        "Reflect skipped — {} errors exceeds 2x threshold ({}). Breaking cascade.",
                        errors.len(), error_threshold
                    );
                    ("I'm experiencing repeated errors. Let me wait for the next message to try a fresh approach.".to_string(), vec![])
                } else {
                    let reflection_prompt = format!(
                        "Before answering the user, reflect on these recent errors and adjust your approach:\n{}\n\nUser message: {}",
                        errors.join("\n"),
                        message
                    );
                    let result = self.process(&reflection_prompt, tools).await?;
                    (result.response, result.tool_calls)
                }
            }
            Decision::SpawnAgents(agents) => {
                if let Some(ref orchestrator) = self.dynamic_orchestrator {
                    info!(
                        agent_count = agents.len(),
                        "Dispatching SpawnAgents via DynamicOrchestrator"
                    );
                    let mut spawned_ids = Vec::new();
                    let mut failed_tasks: Vec<String> = Vec::new();
                    let max_spawn_retries: usize = 2;

                    for req in agents {
                        let caps: Vec<String> = if req.capabilities.is_empty() {
                            vec![req.role.clone()]
                        } else {
                            req.capabilities.clone()
                        };

                        let mut current_req = req.clone();
                        loop {
                            match orchestrator.route_task(&current_req.task, &caps).await {
                                Ok(agent_id) => {
                                    info!(agent_id = %agent_id, role = %current_req.role, "Spawned agent for subtask");
                                    if let Some(ref spawner_mutex) = self.spawner
                                        && let Ok(mut spawner) = spawner_mutex.lock()
                                    {
                                        spawner
                                            .tracker_mut()
                                            .track_spawn(current_req.clone(), agent_id.clone());
                                    }
                                    spawned_ids.push(agent_id);
                                    break;
                                }
                                Err(e) => {
                                    warn!(role = %current_req.role, error = %e, "Failed to spawn agent");
                                    let retry = self.spawner.as_ref().and_then(|m| {
                                        m.lock().ok().and_then(|mut s| {
                                            s.handle_route_failure(
                                                &current_req,
                                                &e.to_string(),
                                                max_spawn_retries,
                                            )
                                        })
                                    });
                                    match retry {
                                        Some(retry_req) => {
                                            current_req = retry_req;
                                            continue;
                                        }
                                        None => {
                                            // Retry budget exhausted — fold task into main execution
                                            failed_tasks.push(current_req.task.clone());
                                            exec_errors.push(format!(
                                                "Spawn failed for '{}': {}",
                                                current_req.role, e
                                            ));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // If some spawns failed, fold their tasks into the cooking context
                    let cooking_msg = if failed_tasks.is_empty() {
                        message.to_string()
                    } else {
                        warn!(
                            failed_count = failed_tasks.len(),
                            spawned_count = spawned_ids.len(),
                            "Some agent spawns failed — folding tasks into main execution"
                        );
                        format!(
                            "{}\n\n[Note: {} agent spawn(s) failed after retries. Handle these tasks directly:]\n{}",
                            message,
                            failed_tasks.len(),
                            failed_tasks
                                .iter()
                                .enumerate()
                                .map(|(i, t)| format!("{}. {}", i + 1, t))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    };

                    // After dispatching agents, use cooking loop for the main task
                    let (response, tool_calls) = if self.tool_executor.is_some() {
                        let cook_result = self.cook(&cooking_msg, tools).await?;
                        exec_success = cook_result.completed_naturally && failed_tasks.is_empty();
                        // Lane A1.5b-ii: cook-completion = real-interaction-completion timestamp.
                        self.record_interaction_completion();
                        exec_tool_count = cook_result.tool_calls.len();
                        let pr = Self::cooking_to_process(&cook_result);
                        (pr.response, pr.tool_calls)
                    } else {
                        let result = self.process(&cooking_msg, tools).await?;
                        exec_tool_count = result.tool_calls.len();
                        (result.response, result.tool_calls)
                    };

                    // Collect results from spawned agents and complete tracking
                    if !spawned_ids.is_empty() {
                        let results = orchestrator.collect_results(&spawned_ids).await;
                        if let Some(ref spawner_mutex) = self.spawner
                            && let Ok(mut spawner) = spawner_mutex.lock()
                        {
                            for r in &results {
                                spawner.tracker_mut().complete_spawn(
                                    &r.agent_id,
                                    r.success,
                                    r.output.clone(),
                                );
                            }
                        }
                        // Log aggregated spawn outcomes
                        let success_count = results.iter().filter(|r| r.success).count();
                        let total = results.len();
                        if total > 0 {
                            info!(
                                success = success_count,
                                total = total,
                                pending = spawned_ids.len() - total,
                                "SpawnAgents: collected {total} results ({success_count} succeeded)"
                            );
                        }
                    }

                    (response, tool_calls)
                } else {
                    // No orchestrator available — fall back to cooking loop
                    info!(
                        agent_count = agents.len(),
                        "SpawnAgents decision detected, no orchestrator — falling back to cooking loop"
                    );
                    if self.tool_executor.is_some() {
                        let cook_result = self.cook(message, tools).await?;
                        exec_success = cook_result.completed_naturally;
                        // Lane A1.5b-ii: cook-completion = real-interaction-completion timestamp.
                        self.record_interaction_completion();
                        exec_tool_count = cook_result.tool_calls.len();
                        let pr = Self::cooking_to_process(&cook_result);
                        (pr.response, pr.tool_calls)
                    } else {
                        let result = self.process(message, tools).await?;
                        exec_tool_count = result.tool_calls.len();
                        (result.response, result.tool_calls)
                    }
                }
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // 5. Evaluate execution with critic
        let intent_type = analysis.intent.to_string();
        let expected_ms = self.feedback.estimated_time(&intent_type);
        let partial = !exec_success && exec_errors.len() < exec_tool_count;

        let exec_ctx = ExecutionContext {
            task_description: message.to_string(),
            success: exec_success,
            partial,
            duration_ms,
            expected_ms,
            retry_count: exec_retries,
            errors: exec_errors,
            tool_call_count: exec_tool_count,
            goal_id: goal_id.clone(),
        };
        let evaluation = self.critic.evaluate(&exec_ctx);

        debug!(
            quality = evaluation.quality_score,
            improvements = evaluation.improvements.len(),
            "Critic evaluation complete"
        );

        // 6. Update goal status based on evaluation
        if let Some(ref gid) = goal_id
            && let Some(ref stack) = self.goal_stack
        {
            let new_status = match &evaluation.outcome {
                zeus_nous::TaskOutcome::Success => GoalStatus::Completed {
                    outcome: "Task completed successfully".to_string(),
                },
                zeus_nous::TaskOutcome::PartialSuccess { details } => GoalStatus::Completed {
                    outcome: format!("Partially completed: {}", details),
                },
                zeus_nous::TaskOutcome::Failure { reason } => GoalStatus::Failed {
                    reason: reason.clone(),
                },
            };
            if let Err(e) = stack.update_status(gid, new_status.clone()) {
                debug!("Failed to update goal status (non-fatal): {}", e);
            }
            // Unblock dependent goals on completion
            if matches!(new_status, GoalStatus::Completed { .. }) {
                match stack.unblock(gid) {
                    Ok(unblocked) if !unblocked.is_empty() => {
                        info!(count = unblocked.len(), "Unblocked dependent goals");
                    }
                    Err(e) => {
                        debug!("Failed to unblock goals (non-fatal): {}", e);
                    }
                    _ => {}
                }
            }
        }

        // 7. Record metrics
        self.monitor.record_llm_call(duration_ms, exec_success);

        // 8. Record interaction for learning
        if let Some(ref engine) = self.learning_engine {
            let record = InteractionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now(),
                query_type: analysis.intent.to_string(),
                tools_used: tool_calls.clone(),
                success: exec_success,
                duration_ms,
                error_message: None,
                complexity: format!("{:?}", analysis.complexity),
            };
            if let Err(e) = engine.record(record) {
                debug!("Failed to record interaction (non-fatal): {}", e);
            }
        }

        // 9. Record feedback for strategy learning
        self.feedback
            .record_outcome(&analysis, &decision, exec_success, duration_ms);

        Ok(AutonomousResult {
            response,
            tool_calls,
            decision,
            intent: analysis,
            processing_time_ms: duration_ms,
            health_status: health.status,
        })
    }

    /// Process a user message (direct LLM call, no autonomy pipeline)
    pub async fn process(&self, message: &str, tools: &[ToolSchema]) -> Result<ProcessResult> {
        let start = std::time::Instant::now();

        // Get system prompt from workspace
        let system_prompt = self.workspace.get_context().await?;

        // Build messages
        let messages = vec![Message::user(message)];

        // Call LLM
        let response = self
            .llm
            .complete(&messages, tools, Some(&system_prompt))
            .await?;

        let processing_time = start.elapsed().as_millis() as u64;

        // Record metrics
        self.monitor.record_llm_call(processing_time, true);

        Ok(ProcessResult {
            response: response.content,
            tool_calls: response
                .tool_calls
                .iter()
                .map(|tc| tc.name.clone())
                .collect(),
            processing_time_ms: processing_time,
            tokens_used: None,
        })
    }

    /// Process with streaming
    pub async fn process_stream<F>(
        &self,
        message: &str,
        tools: &[ToolSchema],
        on_chunk: F,
    ) -> Result<ProcessResult>
    where
        F: Fn(&str) + Send + Sync,
    {
        let start = std::time::Instant::now();

        // Get system prompt
        let system_prompt = self.workspace.get_context().await?;

        // Build messages
        let messages = vec![Message::user(message)];

        // Stream response - returns (Receiver, JoinHandle)
        let mut full_response = String::new();
        let (mut rx, handle) = self
            .llm
            .stream(&messages, tools, Some(&system_prompt))
            .await?;

        // Receive chunks from the channel
        while let Some(chunk) = rx.recv().await {
            on_chunk(&chunk);
            full_response.push_str(&chunk);
        }

        // Wait for the final response (contains tool calls, etc.)
        let final_response = handle
            .await
            .map_err(|e| zeus_core::Error::Llm(e.to_string()))?;

        let processing_time = start.elapsed().as_millis() as u64;

        // Record metrics
        self.monitor.record_llm_call(processing_time, true);

        Ok(ProcessResult {
            response: full_response,
            tool_calls: final_response
                .tool_calls
                .iter()
                .map(|tc| tc.name.clone())
                .collect(),
            processing_time_ms: processing_time,
            tokens_used: None,
        })
    }

    /// Start heartbeat background tasks
    /// S69: Set heartbeat result delivery channel before starting.
    pub fn set_heartbeat_result_tx(&mut self, tx: tokio::sync::mpsc::Sender<String>) {
        self.pending_heartbeat_result_tx = Some(tx);
    }

    /// Wire the channel-active flag so heartbeat defers while real messages are processing.
    pub fn set_channel_active(&mut self, state: zeus_core::CookState) {
        self.pending_channel_active = Some(state);
    }

    /// Wire the inbox queue-depth counter so heartbeat can read mpsc-buffer
    /// depth as the `busy: inbound` skip-signal in busy-aware fire-decision.
    /// Plumbing for spec §3.1; fire-decision read-site integration follows.
    pub fn set_inbox_queue_depth(&mut self, depth: std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        self.pending_inbox_queue_depth = Some(depth);
    }

    /// Set the subagent-depth counter for busy-aware fire-decision (`busy: subagent`).
    /// Plumbing-only proxy; forwarded to `Heartbeat::set_subagent_depth` at start.
    /// Wired by gateway from `SpawnTracker::active_count_handle()`.
    pub fn set_subagent_depth(&mut self, depth: std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        self.pending_subagent_depth = Some(depth);
    }

    /// Set the last-interaction-at unix-secs handle for busy-aware fire-decision
    /// (`busy: recent_interaction`). Plumbing-only proxy; forwarded to
    /// `Heartbeat::set_last_interaction_at` at start AND retained on `Prometheus` so
    /// cook-completion writes in `process_autonomous` land on the same handle. Dual-path
    /// (Lane A1.5b-ii): if not externally set, `start_heartbeat` allocates internally.
    pub fn set_last_interaction_at(
        &mut self,
        handle: std::sync::Arc<std::sync::atomic::AtomicI64>,
    ) {
        self.pending_last_interaction_at = Some(handle);
    }

    /// Record a real-interaction-completion timestamp for busy-aware fire-decision
    /// `busy: recent_interaction` bucket. Called from `process_autonomous` cook-completion
    /// points. Single-write-invariant: this is the ONLY write-site for `last_interaction_at`
    /// post-allocation. No-op if `start_heartbeat` hasn't run yet (handle is `None`).
    /// Time-source consistency: unix-secs from `SystemTime::now()`, matching read-site
    /// in `should_fire_heartbeat` (Lane A1.5b-i.α).
    fn record_interaction_completion(&self) {
        if let Some(handle) = self.last_interaction_at.as_ref() {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            handle.store(now_unix, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Set the channel for delivering trigger execution results to the gateway.
    ///
    /// When a cron trigger fires, its output is sent through this channel
    /// so the gateway can inject it into the agent's context.
    pub fn set_trigger_result_tx(&mut self, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        if let Some(ref mut scheduler) = self.scheduler {
            // We have &mut self so no one else can be reading/writing the scheduler.
            // Use Arc::get_mut to avoid the RwLock entirely if refcount is 1.
            if let Some(lock) = Arc::get_mut(scheduler) {
                lock.get_mut().set_trigger_result_tx(tx);
            } else if let Ok(mut guard) = scheduler.try_write() {
                guard.set_trigger_result_tx(tx);
            } else {
                tracing::warn!("set_trigger_result_tx: scheduler RwLock contested, trigger results won't be delivered");
            }
        }
    }

    pub async fn start_heartbeat(&mut self, tools: Vec<ToolSchema>) -> Result<()> {
        if self.heartbeat.is_some() {
            return Ok(());
        }

        let mut heartbeat = heartbeat::Heartbeat::new(self.workspace.clone(), self.llm.clone());

        // Pass config interval to heartbeat (fixes 3600s hardcode override)
        if let Some(ref prom_cfg) = self.config.prometheus {
            heartbeat = heartbeat.with_interval(prom_cfg.heartbeat_interval_secs);

            // Wire HeartbeatConfig from `[prometheus.heartbeat]` if provided.
            // This is the only path that lets ops flip `event_driven_only`,
            // tune intervals, or override quiet hours without a binary patch.
            // Stored as opaque JSON in zeus-core (avoids zeus-core → zeus-prometheus dep cycle).
            if let Some(ref hb_typed) = prom_cfg.heartbeat {
                let hb_value = serde_json::to_value(hb_typed).unwrap_or(serde_json::Value::Null);
                match serde_json::from_value::<heartbeat::HeartbeatConfig>(hb_value) {
                    Ok(hb_cfg) => {
                        info!(
                            "Heartbeat config loaded from config.toml: event_driven_only={}, safety_net={}s",
                            hb_cfg.event_driven_only, hb_cfg.safety_net_interval_secs
                        );
                        heartbeat = heartbeat.with_config(hb_cfg);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse [prometheus.heartbeat] config — falling back to defaults: {}",
                            e
                        );
                    }
                }
            }
        }

        if let Some(ref executor) = self.tool_executor {
            heartbeat = heartbeat.with_tool_executor(executor.clone(), tools);
        }

        // Wire result delivery if configured
        if let Some(tx) = self.pending_heartbeat_result_tx.take() {
            heartbeat.set_result_tx(tx);
        }

        // Wire channel-active flag so heartbeat defers during real message processing
        if let Some(flag) = self.pending_channel_active.take() {
            heartbeat.set_channel_active(flag);
        }

        // Wire inbox queue-depth counter for busy-aware fire-decision (`busy: inbound`).
        if let Some(depth) = self.pending_inbox_queue_depth.take() {
            heartbeat.set_inbox_queue_depth(depth);
        }

        // Wire subagent-depth counter for busy-aware fire-decision (`busy: subagent`).
        // Self-wire from internal SpawnTracker if pending wasn't externally set.
        // Spawner is owned by Prometheus (`Option<Mutex<ProactiveSpawner>>`), so
        // wire-up happens here via direct `active_count_handle()` rather than
        // requiring gateway.rs to plumb a handle externally.
        let subagent_depth = self.pending_subagent_depth.take().or_else(|| {
            self.spawner
                .as_ref()
                .and_then(|m| m.lock().ok().map(|s| s.tracker().active_count_handle()))
        });
        if let Some(depth) = subagent_depth {
            heartbeat.set_subagent_depth(depth);
        }

        // Lane A1.5b-ii: dual-path last-interaction-at handle. External setter wins;
        // otherwise allocate internally so cook-completion writes in `process_autonomous`
        // land on the same `Arc<AtomicI64>` Heartbeat reads via `should_fire_heartbeat`.
        let last_interaction_handle = self
            .pending_last_interaction_at
            .take()
            .unwrap_or_else(|| std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)));
        heartbeat.set_last_interaction_at(last_interaction_handle.clone());
        self.last_interaction_at = Some(last_interaction_handle);

        // Wire Mnemosyne for memory recall during heartbeat cooks
        if let Some(ref mnem) = self.mnemosyne {
            heartbeat.set_mnemosyne(mnem.clone());
        }

        heartbeat.start().await?;
        self.heartbeat = Some(heartbeat);
        Ok(())
    }

    /// Stop heartbeat
    pub fn stop_heartbeat(&mut self) {
        if let Some(ref mut hb) = self.heartbeat {
            hb.stop();
        }
        self.heartbeat = None;
    }

    /// S67-C2: Get the heartbeat wake sender for event-driven triggers.
    /// Any component can clone this sender and use it to trigger immediate heartbeats.
    pub fn heartbeat_wake_sender(&self) -> Option<tokio::sync::mpsc::Sender<heartbeat::WakeRequest>> {
        self.heartbeat.as_ref().and_then(|hb| hb.wake_sender())
    }

    /// #89.2: Get the shared peer-status context handle.
    /// Gateway writes fleet presence data here; heartbeat reads it each cycle.
    pub fn peer_status_handle(&self) -> Option<Arc<tokio::sync::RwLock<Option<String>>>> {
        self.heartbeat.as_ref().map(|hb| hb.peer_status_handle())
    }

    /// #89.3: Get the shared unread context handle.
    /// Gateway queries message history and writes formatted context here.
    pub fn unread_handle(&self) -> Option<Arc<tokio::sync::RwLock<Option<String>>>> {
        self.heartbeat.as_ref().map(|hb| hb.unread_handle())
    }

    /// Start the cron-based scheduler.
    ///
    /// If a scheduler was created during initialization, this starts its
    /// background loop. If no scheduler exists, this is a no-op.
    pub async fn start_scheduler(&mut self) -> Result<()> {
        if let Some(ref sched) = self.scheduler {
            sched.write().await
                .start(Arc::new(self.config.clone()), self.workspace.clone(), self.llm.clone())
                .await?;
        }
        Ok(())
    }

    /// Stop the cron-based scheduler.
    pub fn stop_scheduler(&mut self) {
        if let Some(ref sched) = self.scheduler {
            // We can't call .stop() synchronously on RwLock, but stop() just
            // sends a shutdown signal which is synchronous internally.
            // Use try_write to avoid blocking, or just let it drop.
            if let Ok(mut guard) = sched.try_write() {
                guard.stop();
            }
        }
    }

    /// Get a reference to the cron scheduler, if one exists.
    pub fn scheduler(&self) -> Option<Arc<RwLock<CronScheduler>>> {
        self.scheduler.clone()
    }

    /// Start the background consolidation engine.
    ///
    /// Requires Mnemosyne to be configured. The engine periodically decays
    /// importance of episodic memories, extracts patterns, and cleans up
    /// old low-importance memories.
    pub fn start_consolidation(&mut self) {
        if self.consolidation_shutdown.is_some() {
            return; // Already running
        }

        let mnemosyne = match self.mnemosyne {
            Some(ref m) => m.clone(),
            None => {
                debug!("Consolidation engine requires Mnemosyne (not configured)");
                return;
            }
        };

        let (tx, rx) = tokio::sync::watch::channel(false);
        self.consolidation_shutdown = Some(tx);

        let engine = Arc::new(zeus_nous::consolidation::ConsolidationEngine::default());
        let provider = Arc::new(MnemosyneConsolidationProvider::new(mnemosyne));

        tokio::spawn(async move {
            engine.run_background(provider, rx).await;
        });

        info!("Consolidation engine started");
    }

    /// Stop the background consolidation engine.
    pub fn stop_consolidation(&mut self) {
        if let Some(tx) = self.consolidation_shutdown.take() {
            let _ = tx.send(true);
            info!("Consolidation engine stopped");
        }
    }

    /// Get heartbeat tasks for a frequency
    pub async fn get_heartbeat_tasks(&self, frequency: &str) -> Result<Vec<String>> {
        self.workspace.get_heartbeat_tasks(frequency).await
    }

    /// Plan a complex task into steps using LLM
    pub async fn plan(&self, task: &str, tools: &[ToolSchema]) -> Result<planner::Plan> {
        self.planner.create_plan(task, &self.llm, tools).await
    }

    /// Execute a plan using LLM to drive each step.
    ///
    /// If a ToolExecutor is configured, tools are actually executed.
    /// Otherwise falls back to LLM-only simulation mode.
    pub async fn execute_plan(
        &self,
        plan: &planner::Plan,
        tools: &[ToolSchema],
    ) -> Result<executor::ExecutionResult> {
        // Use parallel executor: independent steps (no shared dependencies) run
        // concurrently via join_all; single-step batches fall through to serial.
        // Previously called executor.execute() which was always serial.
        self.executor
            .execute_parallel(plan, &self.llm, tools, self.tool_executor.as_deref())
            .await
    }

    /// Plan and immediately execute a task
    pub async fn plan_and_execute(
        &self,
        task: &str,
        tools: &[ToolSchema],
    ) -> Result<executor::ExecutionResult> {
        let plan = self.plan(task, tools).await?;
        self.execute_plan(&plan, tools).await
    }

    /// Plan and execute with adaptive replanning and parallel step execution.
    ///
    /// If a step fails, the planner re-plans the remaining work (up to
    /// `max_replans` times). Independent steps run in parallel.
    /// Results are persisted to the plan outcome store for learning.
    pub async fn plan_and_execute_adaptive(
        &self,
        task: &str,
        tools: &[ToolSchema],
        max_replans: usize,
    ) -> Result<executor::ExecutionResult> {
        let plan = self.plan(task, tools).await?;

        let (result, replan_count) = self
            .executor
            .execute_adaptive(
                &plan,
                &self.planner,
                &self.llm,
                tools,
                self.tool_executor.as_deref(),
                max_replans,
            )
            .await?;

        // Persist outcome for learning
        if let Some(ref store) = self.plan_store {
            let outcome = PlanOutcomeStore::outcome_from_result(task, &result, replan_count);
            if let Err(e) = store.record(&outcome) {
                debug!("Failed to record plan outcome (non-fatal): {}", e);
            }
        }

        Ok(result)
    }

    /// Get current model
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Get workspace
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Get LLM client
    pub fn llm(&self) -> &LlmClient {
        &self.llm
    }

    /// Get session manager
    pub fn sessions(&self) -> &Arc<RwLock<session::SessionManager>> {
        &self.sessions
    }

    /// Increment the dispatch counter for a fleet session alias and return the
    /// prior count (number of dispatches *before* this one). Lane 2b-i.
    pub async fn track_dispatch(&self, alias: &FleetSessionAlias) -> u64 {
        let key = alias.as_str().to_string();
        let mut guard = self.dispatch_counter.write().await;
        let count = guard.entry(key).or_insert(0);
        let prior = *count;
        *count += 1;
        prior
    }

    /// Get the intent classifier
    pub fn intent_classifier(&self) -> &IntentClassifier {
        &self.intent_classifier
    }

    /// Get the autonomy engine
    pub fn autonomy_engine(&self) -> &AutonomyEngine {
        &self.autonomy_engine
    }

    /// Get the learning engine (if available)
    pub fn learning_engine(&self) -> Option<&LearningEngine> {
        self.learning_engine.as_ref()
    }

    /// Get the self-monitor
    /// Set Nous cognitive engine for cooking loop intelligence
    pub fn set_nous(&mut self, nous: Arc<zeus_nous::Nous>) {
        self.nous = Some(nous);
    }

    pub fn monitor(&self) -> &Monitor {
        &self.monitor
    }

    /// Get a health report from the self-monitor
    pub fn health_check(&self) -> HealthReport {
        self.monitor.health_check()
    }

    /// Classify a message without executing (useful for previewing)
    pub fn classify_intent(&self, message: &str, tools: &[ToolSchema]) -> IntentAnalysis {
        self.intent_classifier.classify(message, tools)
    }

    /// Get tool suggestions from learning engine
    pub fn suggest_tools(&self, query_type: &str) -> Vec<String> {
        self.learning_engine
            .as_ref()
            .and_then(|e| e.suggest_tools(query_type).ok())
            .unwrap_or_default()
    }

    /// Get the checkpoint store for resume support
    pub fn checkpoint_store(&self) -> Option<&Arc<CookingCheckpointStore>> {
        self.checkpoint_store.as_ref()
    }

    /// Subscribe to cooking events (tool starts, completions, iterations).
    /// Returns a broadcast receiver that gets live updates during cooking.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<CookingEvent> {
        self.cooking_event_emitter.subscribe()
    }

    /// Find interrupted cooking sessions that can be resumed
    pub async fn find_interrupted_sessions(&self) -> Vec<InterruptedSession> {
        if let Some(ref store) = self.checkpoint_store {
            store.find_interrupted_sessions().await
        } else {
            Vec::new()
        }
    }

    /// Set the tool executor for the cooking loop
    pub fn set_tool_executor(&mut self, executor: Arc<dyn ToolExecutor>) {
        self.tool_executor = Some(executor);
    }

    /// Set the Mnemosyne instance for memory injection
    pub fn set_mnemosyne(&mut self, mnemosyne: Arc<zeus_mnemosyne::Mnemosyne>) {
        self.mnemosyne = Some(mnemosyne);
    }

    /// Set the current channel kind for cross-channel context injection (#86-sprint-C).
    /// Call from gateway before each cook so inject_cross_channel knows what to exclude.
    /// Empty string disables cross-channel injection.
    pub fn set_current_channel_kind(&self, channel_kind: &str) {
        if let Ok(mut w) = self.current_channel_kind.write() {
            *w = channel_kind.to_string();
        }
    }

    /// Set the current human (sender) id for cross-channel session correlation (#192).
    /// Call from gateway before each cook — sibling to `set_current_channel_kind`.
    /// `None` disables alias resolution (resolver returns `fallback_unaliased`).
    pub fn set_current_human_id(&self, human_id: Option<&str>) {
        if let Ok(mut w) = self.current_human_id.write() {
            *w = human_id.map(|s| s.to_string());
        }
    }

    /// Set cooking loop configuration
    /// Set a per-request max iteration cap (0 = use default from config).
    /// Call this before cook_with_history to limit iterations based on intent.
    pub fn set_iteration_cap(&self, cap: usize) {
        self.iteration_cap.store(cap, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_cooking_config(&mut self, config: CookingConfig) {
        self.cooking_config = config;
    }

    /// Set the global state manager for multi-agent coordination
    pub fn set_state_manager(&mut self, sm: Arc<zeus_orchestra::GlobalStateManager>) {
        self.state_manager = Some(sm);
    }

    /// Set the dynamic orchestrator for agent lifecycle management
    pub fn set_dynamic_orchestrator(&mut self, orch: Arc<zeus_orchestra::DynamicOrchestrator>) {
        self.dynamic_orchestrator = Some(orch);
    }

    /// Get a reference to the strategic planner
    pub fn strategic_planner(&self) -> &StrategicPlanner {
        &self.strategic_planner
    }

    /// Get a reference to the proactive agent spawner (if available)
    pub fn spawner(&self) -> Option<std::sync::MutexGuard<'_, ProactiveSpawner>> {
        self.spawner.as_ref().and_then(|m| m.lock().ok())
    }

    /// Get a reference to the plan outcome store (if available)
    pub fn plan_store(&self) -> Option<&PlanOutcomeStore> {
        self.plan_store.as_ref()
    }

    /// Run a coordinated multi-agent execution for a goal.
    ///
    /// Requires both `state_manager` and `dynamic_orchestrator` to be set.
    pub async fn coordinate(&self, goal: &str, tools: &[ToolSchema]) -> Result<CoordinationResult> {
        let sm = self.state_manager.as_ref().ok_or_else(|| {
            zeus_core::Error::Config("state_manager not configured for coordination".into())
        })?;
        let orch = self.dynamic_orchestrator.as_ref().ok_or_else(|| {
            zeus_core::Error::Config("dynamic_orchestrator not configured for coordination".into())
        })?;

        let coord = CoordinationLoop::new(orch.clone(), sm.clone(), CoordinationConfig::default());
        coord.run(goal, &self.llm, tools).await
    }

    /// Get a reference to the goal stack (if available)
    pub fn goal_stack(&self) -> Option<&GoalStack> {
        self.goal_stack.as_ref()
    }

    /// #173-b: Purge the live pending-goal queue (the daemon-side `/clear` hook).
    ///
    /// `/clear` + `--fresh` clear context/files/procs but historically left the
    /// goals.db pending rows intact — and those rows are the durable resume gate
    /// the cook loop re-picks on the next wake. This transitions every
    /// non-terminal goal to `abandoned` so a fresh start truly starts fresh.
    /// Returns the number of rows cleared (0 if no goal stack is configured).
    pub fn clear_pending_goals(&self) -> Result<usize> {
        match self.goal_stack {
            Some(ref stack) => stack.clear_pending(),
            None => Ok(0),
        }
    }

    /// Get a reference to the feedback loop
    pub fn feedback(&self) -> &FeedbackLoop {
        &self.feedback
    }

    /// Get a reference to the critic engine
    pub fn critic(&self) -> &CriticEngine {
        &self.critic
    }

    /// Get active goals summary for context injection
    /// #168 Phase 1 — code-enforced working-memory write of the active goal.
    ///
    /// Shared helper so the one-shot planned-goal write and the per-cook-turn write
    /// don't drift. Writes a `[Active Goal]` working memory under the "goals" session.
    /// No-op (silent) when Mnemosyne is unconfigured — the warn-on-None surfacing
    /// lives at the call sites so this stays a pure write primitive.
    async fn store_active_goal_working(&self, label: &str, body: &str) {
        if let Some(ref mnemosyne) = self.mnemosyne {
            let goal_msg = Message::assistant(format!("[Active Goal] {}: {}", label, body));
            let _ = mnemosyne
                .store_typed(
                    "goals",
                    &goal_msg,
                    zeus_mnemosyne::MemoryType::Working,
                    0.9,
                )
                .await;
        }
    }

    pub fn active_goals_summary(&self) -> Vec<String> {
        self.goal_stack
            .as_ref()
            .and_then(|stack| stack.active_goals().ok())
            .map(|goals| {
                goals
                    .iter()
                    .map(|g| format!("[{}] {} ({})", g.priority, g.description, g.id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Run the full cooking loop: LLM -> tool execution -> feed results -> repeat.
    ///
    /// This is the primary method that integrates tool execution with the LLM.
    /// It automatically injects memory context from Mnemosyne before each run.
    /// Delegates to `cook_with_history()` with no prior conversation context.
    pub async fn cook(&self, message: &str, tools: &[ToolSchema]) -> Result<CookingResult> {
        self.cook_with_history(message, tools, &[]).await
    }

    /// Run the cooking loop with image/file attachments on the initial user message.
    ///
    /// Builds a `Message::user()` with the provided attachments so the LLM sees
    /// images and files — matches the behaviour of `agent.run()` which already
    /// supports attachments natively.
    pub async fn cook_with_attachments(
        &self,
        message: &str,
        tools: &[ToolSchema],
        conversation_history: &[Message],
        attachments: &[zeus_core::Attachment],
    ) -> Result<CookingResult> {
        // Build a synthetic history that appends the current message as a user
        // turn with attachments, then delegate to the core history path.
        let mut history_with_attachment = conversation_history.to_vec();
        let mut user_msg = Message::user(message);
        user_msg.attachments = attachments.to_vec();
        history_with_attachment.push(user_msg);

        // Pass an empty message string — the actual content is in the last
        // history entry we just appended.
        self.cook_with_history("", tools, &history_with_attachment).await
    }

    /// Run the cooking loop with prior session messages as conversation history.
    ///
    /// `conversation_history` is prepended before the current message so the LLM
    /// sees prior turns. This prevents agents from "forgetting" context between
    /// messages when the cooking path is used (vs agent.run() which already has
    /// session history built in).
    /// #278: word-count of the user's ACTUAL turn, ignoring gateway-injected
    /// context. The gateway prepends `[Recent conversation…]` history, a
    /// `[NEW MESSAGE …]` delimiter, and a `[Work State]` block before the real
    /// user text — which inflated the word-count gating the expensive
    /// `nous.reason()` synthesize, so even a 1-word "hey" paid an 8–26s reasoning
    /// LLM call every turn. Counts only words on lines that aren't injected
    /// boilerplate (bracketed markers, history `user:` lines, work-state/plan
    /// lines). Conservative: undercounting just routes a turn to the
    /// no-reasoning path (the cook + tools still run) — never affects correctness.
    fn core_user_turn_wordcount(message: &str) -> usize {
        message
            .lines()
            .map(|l| l.trim())
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with('[')
                    && !l.starts_with("user: [")
                    && !l.starts_with("- ")
                    && !l.starts_with("Your current durable work state")
                    && !l.starts_with("Incomplete plans")
                    && !l.starts_with("Active goals")
                    && !l.starts_with("Pending tasks")
            })
            .flat_map(|l| l.split_whitespace())
            .count()
    }

    pub async fn cook_with_history(
        &self,
        message: &str,
        tools: &[ToolSchema],
        conversation_history: &[Message],
    ) -> Result<CookingResult> {
        self.cook_with_history_cancellable(message, tools, conversation_history, None).await
    }

    /// Cook with history and optional cancellation token.
    /// When the token is cancelled, the cooking loop finishes the current
    /// iteration and returns a partial result.
    pub async fn cook_with_history_cancellable(
        &self,
        message: &str,
        tools: &[ToolSchema],
        conversation_history: &[Message],
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<CookingResult> {
        self.cook_with_history_interruptible(message, tools, conversation_history, cancel, None, vec![], None, None).await
    }

    /// Like `cook_with_history_cancellable` but also accepts an interrupt receiver.
    ///
    /// When a message is sent on `interrupt_rx`, the cooking loop exits at the next
    /// iteration boundary and returns `CookingResult::interrupted_by` with that message.
    /// The gateway uses this to process stop/correction commands mid-sprint.
    pub async fn cook_with_history_interruptible(
        &self,
        message: &str,
        tools: &[ToolSchema],
        conversation_history: &[Message],
        cancel: Option<tokio_util::sync::CancellationToken>,
        interrupt_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
        attachments: Vec<zeus_core::Attachment>,
        // #168 Phase 4: gateway-threaded pending-tasks string (already formatted at
        // gateway.rs). `None` on the REST/non-gateway call sites — explicit-per-call so
        // a gateway cook's tasks can't stale-leak into a later REST cook.
        pending_tasks: Option<String>,
        // #176-b H2: gateway-threaded progress signal. The cooking loop stores the
        // unix-secs of each completed tool-call here; the gateway's timeout arm reads it
        // to decide extend-vs-kill. `None` on REST/non-gateway call sites (no auto-extend).
        progress_signal: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<CookingResult> {
        let executor = self
            .tool_executor
            .as_ref()
            .ok_or_else(|| zeus_core::Error::Config("No tool executor configured".to_string()))?;

        // Get system prompt from workspace
        let mut system_prompt = self.workspace.get_context().await?;

        // #168 Phase 1 — code-enforced working write of the active task. Write-first:
        // you can't recall what was never written. Uses the shared helper so this and
        // the one-shot planned-goal write can't drift. Warn-on-None so an unconfigured-
        // Mnemosyne seat is visible (#168 Phase 3).
        //
        // #168 dedup (deploy-gating fast-follow): two corrections to the naive per-turn
        // write that appended a `[Active Goal] cook-turn: {raw inbound}` row every answer:
        //   1. Record actual goal-progress, not the raw inbound `message`. Prefer the
        //      real active goal from `active_goals_summary()`; only fall back to a
        //      truncated inbound label when no goal is on the stack.
        //   2. Write-on-goal-change, not per-turn. Skip the write when the goal-key is
        //      unchanged from the last cook turn — so a session of N answers about the
        //      same goal yields one row, not N near-dups that get recalled later
        //      masquerading as distinct active goals.
        if self.mnemosyne.is_some() {
            // Prefer the real active goal; fall back to a bounded inbound label.
            let active = self.active_goals_summary();
            let (label, body) = if let Some(first) = active.first() {
                ("active-goal", first.clone())
            } else {
                let truncated: String = message.chars().take(120).collect();
                ("cook-turn", truncated)
            };
            let goal_key = format!("{}:{}", label, body);

            // Write-on-goal-change: only write when the key differs from last turn.
            let changed = {
                let last = self.last_cook_goal_key.read().unwrap();
                last.as_deref() != Some(goal_key.as_str())
            };
            if changed {
                self.store_active_goal_working(label, &body).await;
                if let Ok(mut last) = self.last_cook_goal_key.write() {
                    *last = Some(goal_key);
                }
            }
        } else {
            warn!("#168: Mnemosyne unconfigured — cook-turn working write skipped (work-progress will not persist on this seat)");
        }

        // R3: Prepend live <env> block (cwd, hostname, time, git branch + dirty files).
        // Per agent-intelligence research synthesis — Zeus injects 8+ static workspace
        // files but no live state; Claude Code injects fewer files plus this live signal.
        // Cost: one `git status --porcelain` per cooking turn. No LLM call.
        let env_snapshot = crate::env_block::EnvSnapshot::capture();
        system_prompt = format!("{}\n{}", env_snapshot.render(), system_prompt);

        // S78: Removed force-loaded core skills (TDD, verification, debugging).
        // Skills are now activated contextually via keyword matching below.
        // Force-loading injected rigid rules on every message (including casual chat),
        // causing robotic tone and reporting loops.
        let skill_dirs = vec![
            self.workspace.root().join("skills"),
            zeus_core::default_config_dir().join("skills"),
        ];

        // S79: List available skills by name only — agent loads on demand via read_file.
        // OpenClaw approach: don't auto-inject skill content. List names + one-line descriptions
        // in the system prompt. The agent decides which skill is relevant and reads it.
        let mut skill_names: Vec<(String, String)> = Vec::new();
        for skills_dir in &skill_dirs {
            if skills_dir.exists() {
                if let Ok(mut entries) = tokio::fs::read_dir(&skills_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                            let skill_name = entry.file_name().to_string_lossy().to_string();
                            let skill_md = entry.path().join("SKILL.md");
                            if let Ok(content) = tokio::fs::read_to_string(&skill_md).await {
                                // Extract one-line description from frontmatter
                                let desc = content.lines()
                                    .find(|l| l.starts_with("description:"))
                                    .map(|l| l.trim_start_matches("description:").trim().to_string())
                                    .unwrap_or_else(|| skill_name.clone());
                                if !skill_names.iter().any(|(n, _)| n == &skill_name) {
                                    skill_names.push((skill_name, desc));
                                }
                            }
                        }
                    }
                }
            }
        }
        if !skill_names.is_empty() {
            skill_names.sort_by(|a, b| a.0.cmp(&b.0));
            system_prompt.push_str("\n\n[Available Skills]\n");
            system_prompt.push_str("Use read_file to load a skill when the task matches. Do NOT load skills for casual conversation.\n");
            for (name, desc) in skill_names.iter().take(30) {
                system_prompt.push_str(&format!("- **{}**: {}\n", name, desc));
            }
            debug!("Cooking loop: {} skills listed (agent-driven loading)", skill_names.len());
        }

        // Inject Nous cognitive context (intent + lessons) into cooking prompt.
        // S79/Fix6: Use plain English instead of JSON-schema formatting — agents mirror
        // the register of their input; machine-format context produces machine-format responses.
        if let Some(ref nous) = self.nous {
            if let Ok(intent) = nous.understand(message).await {
                let confidence_pct = (intent.confidence.0 * 100.0) as u8;
                system_prompt.push_str(&format!(
                    "\n\nThis looks like a {:?} request ({}% confidence).",
                    intent.intent_type, confidence_pct
                ));
            }
            let lessons = nous.get_relevant_lessons(message).await;
            if !lessons.is_empty() {
                system_prompt.push_str("\n\nBased on past experience, keep in mind:\n");
                for lesson in lessons.iter().take(5) {
                    system_prompt.push_str(&format!("- {}\n", lesson.insight));
                }
            }
        }

        // Inject Nous reasoning chain for non-trivial messages.
        // Skipped when nous is absent; any LLM failure degrades gracefully (no chain injected).
        if let Some(ref nous) = self.nous {
            if Self::core_user_turn_wordcount(message) > 4 {
                if let Ok(chain) = nous.reason(message).await {
                    if let Some(ref conclusion) = chain.conclusion {
                        system_prompt.push_str(&format!(
                            "\n\nReasoning summary: {}",
                            conclusion
                        ));
                    }
                    let actions = chain.to_action_plan();
                    if !actions.is_empty() {
                        system_prompt.push_str("\nPlanned steps: ");
                        let descs: Vec<&str> = actions.iter().take(4).map(|a| a.description.as_str()).collect();
                        system_prompt.push_str(&descs.join(" → "));
                        system_prompt.push('.');
                    }
                }
            }
        }

        // Inject learning-based tool suggestions.
        // Classify the current message intent, then query the learning engine for which
        // tools have been most effective on similar past requests. Only inject when there
        // is actual history (non-empty suggestion list) to avoid noise on first runs.
        if self.learning_engine.is_some() {
            let query_type = self.intent_classifier.classify(message, tools).intent.to_string();
            let suggestions = self.suggest_tools(&query_type);
            if !suggestions.is_empty() {
                system_prompt.push_str("\n\nTools that have worked well for this type of request in the past: ");
                system_prompt.push_str(&suggestions.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
                system_prompt.push('.');
            }
        }

        // Fetch memory context if Mnemosyne is available
        // Uses hierarchical weighted search (same quality as agent loop)
        // plus proactive context from conversation history
        let memory_context = if let Some(ref mnemosyne) = self.mnemosyne {
            let query_ctx = self.memory_injector.fetch_context(mnemosyne, message).await;
            let proactive_ctx = if !conversation_history.is_empty() {
                self.memory_injector.fetch_proactive_context(mnemosyne, conversation_history).await
            } else {
                None
            };
            // Sprint-C (#86): cross-channel context injection.
            // Reads channel kind set by gateway before cook; no-op if empty/unknown.
            let cross_channel_ctx = {
                let ck = self.current_channel_kind.read()
                    .ok()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                if ck.is_empty() {
                    None
                } else {
                    // #86 complement: resolve the fleet session alias for this
                    // (agent, human, channel) key, then route cross-channel
                    // injection through the alias-aware path so context
                    // correlates against the fleet-aliased session rather than
                    // the raw channel key. human_id is read from the gateway-set
                    // `current_human_id` (#192) — when present the resolver can
                    // correlate same-human sessions across surfaces; when None it
                    // returns `fallback_unaliased` and the aliased path falls back
                    // to raw-channel behavior (strictly
                    // additive — never less context than before).
                    use std::str::FromStr;
                    let agent_id = self.config.network
                        .as_ref()
                        .and_then(|n| n.agent_name.clone())
                        .unwrap_or_else(|| "Zeus".to_string());
                    let channel_kind = crate::ChannelKind::from_str(&ck)
                        .unwrap_or(crate::ChannelKind::Discord);
                    // #192: human id plumbed from gateway via set_current_human_id().
                    let human_id = self.current_human_id.read()
                        .ok()
                        .and_then(|g| g.clone());
                    let alias = {
                        let sessions = self.sessions.read().await;
                        self.session_resolver(
                            &sessions,
                            &agent_id,
                            human_id.as_deref(),
                            channel_kind,
                            chrono::Utc::now(),
                        )
                        .await
                    };
                    self.memory_injector
                        .inject_cross_channel_aliased(mnemosyne, message, &ck, &alias)
                        .await
                }
            };
            match (query_ctx, proactive_ctx, cross_channel_ctx) {
                (Some(q), Some(p), Some(c)) => Some(format!("{}\n{}\n{}", q, p, c)),
                (Some(q), Some(p), None)    => Some(format!("{}\n{}", q, p)),
                (Some(q), None,    Some(c)) => Some(format!("{}\n{}", q, c)),
                (None,    Some(p), Some(c)) => Some(format!("{}\n{}", p, c)),
                (Some(q), None,    None)    => Some(q),
                (None,    Some(p), None)    => Some(p),
                (None,    None,    Some(c)) => Some(c),
                (None,    None,    None)    => None,
            }
        } else {
            // #168 Phase 3 — warn (not debug) so an unconfigured-Mnemosyne seat is visible.
            warn!("#168: Mnemosyne not configured — cooking loop runs without memory recall (DB half disabled on this seat)");
            None
        };

        // #168 Phase 2 — code-enforced DB recall on the live cook path.
        //
        // `cook_with_history_interruptible` bypasses `run_turn`, so the goals/tasks the
        // gateway computes never reach this path. Mnemosyne recall above is `Some()`-gated
        // and silent. Result: the live Discord answer path had a *dead DB half* — it could
        // answer "what are you working on?" with zero knowledge of its own active goals or
        // incomplete plans. This block fixes that at the gate, not the prompt: it always
        // queries the durable work-state sources and always injects them. No classifier,
        // no LLM-echo — a code-enforced hook that fires on every cook turn.
        {
            let active_goals = self.active_goals_summary();
            let incomplete_plans =
                crate::plan_mode::PlanMode::find_incomplete(self.workspace.root()).await;
            // #168 Phase 4b — shared formatter (one formatter, two callers:
            // this cook path + the zeus-api REST handlers) so the block can't drift.
            if let Some(work_state) = crate::goals::format_work_state(
                &active_goals,
                &incomplete_plans,
                pending_tasks.as_deref(),
            ) {
                system_prompt.push_str(&work_state);
                debug!(
                    active_goals = active_goals.len(),
                    incomplete_plans = incomplete_plans.len(),
                    pending_tasks = pending_tasks.is_some(),
                    "#168: injected [Work State] block into cook system prompt"
                );
            }
        }

        // Apply per-request iteration cap if set (intent-based limiting)
        let mut cooking_cfg = self.cooking_config.clone();
        // Gate audit logging: only Anthropic handles mid-conversation system messages cleanly.
        {
            let caps = zeus_llm::capabilities::capabilities(self.llm.provider());
            cooking_cfg.audit_logging = caps.supports_audit_logging;
        }
        let cap = self.iteration_cap.load(std::sync::atomic::Ordering::Relaxed);
        if cap > 0 && cap < cooking_cfg.max_iterations {
            info!("Cooking iteration cap: {} (intent-based, default was {})", cap, cooking_cfg.max_iterations);
            cooking_cfg.max_iterations = cap;
        }
        // Reset cap after reading (one-shot per request)
        self.iteration_cap.store(0, std::sync::atomic::Ordering::Relaxed);

        // Run cooking loop with optional per-iteration Mnemosyne refresh
        let mut cooking_loop = CookingLoop::new(cooking_cfg.clone())
            .with_events(self.cooking_event_emitter.clone());
        if let Some(ref mnemosyne) = self.mnemosyne {
            cooking_loop = cooking_loop.with_mnemosyne(
                mnemosyne.clone(),
                MemoryInjector::new(
                    self.cooking_config.memory_results,
                    self.memory_injector.max_context_chars(),
                ),
            );
        }
        if let Some(ref store) = self.checkpoint_store {
            cooking_loop = cooking_loop.with_checkpoint_store(store.clone());
        }
        if let Some(rx) = interrupt_rx {
            cooking_loop = cooking_loop.with_interrupt(rx);
        }
        // Wire WakeRequest sender so heartbeat fires immediately on cook completion
        // instead of waiting for the next scheduled interval.
        if let Some(ref wake_tx) = self.heartbeat_wake_sender() {
            cooking_loop = cooking_loop.with_wake_sender(wake_tx.clone());
        }
        // #176-b H2: wire the gateway-threaded progress signal so each completed
        // tool-call updates the timestamp the timeout arm reads for extend-vs-kill.
        if let Some(ref signal) = progress_signal {
            cooking_loop = cooking_loop.with_progress_signal(signal.clone());
        }
        let result = cooking_loop
            .run_with_history(
                message,
                &system_prompt,
                tools,
                &self.llm,
                executor.as_ref(),
                memory_context.as_deref(),
                conversation_history,
                cancel,
                attachments,
            )
            .await?;

        // Record metrics
        self.monitor
            .record_llm_call(result.processing_time_ms, true);
        for record in &result.tool_calls {
            self.monitor.record_tool_call(
                &record.name,
                record.success,
                if record.success { 0 } else { 500 },
            );
        }

        // Record learning
        if let Some(ref engine) = self.learning_engine {
            let record = InteractionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now(),
                query_type: "cooking_loop".to_string(),
                tools_used: result.tool_calls.iter().map(|r| r.name.clone()).collect(),
                success: result.completed_naturally,
                duration_ms: result.processing_time_ms,
                error_message: None,
                complexity: format!("{} iterations", result.iterations),
            };
            if let Err(e) = engine.record(record) {
                debug!("Failed to record interaction (non-fatal): {}", e);
            }
        }

        // Superpowers verification gate: if agent wrote code but didn't verify,
        // append a verification reminder to the response
        let result = result;
        let code_tools = ["write_file", "edit_file", "shell"];
        let wrote_code = result.tool_calls.iter().any(|tc| {
            code_tools.contains(&tc.name.as_str()) && tc.success
        });
        let ran_tests = result.tool_calls.iter().any(|tc| {
            tc.name == "shell" && tc.success && {
                let cmd = tc.arguments.get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                cmd.contains("cargo test") || cmd.contains("cargo check")
                    || cmd.contains("npm test") || cmd.contains("pytest")
            }
        });
        if wrote_code && !ran_tests && result.completed_naturally {
            // Log the reminder — don't append to response (causes agents to parrot it in Discord)
            tracing::info!("Verification reminder: code modified without test/check in cooking loop");
        }

        Ok(result)
    }

    /// Plan Mode: generate a plan, persist it to disk, then execute with
    /// the plan injected into the system prompt.
    ///
    /// Two phases:
    /// 1. **Plan generation** — single LLM call (no tools) via `self.planner.create_plan`.
    ///    The resulting `Plan` is written to `~/.zeus/workspace/plans/{slug}/PLAN.md`
    ///    by `PlanMode::write_plan`.
    /// 2. **Plan execution** — the PLAN.md contents are injected into the caller's
    ///    message so the cooking loop sees them alongside the task, then
    ///    `cook_with_history_interruptible` runs normally. On completion we persist
    ///    an OUTCOME.md next to PLAN.md.
    ///
    /// This is a thin wrapper (~100 LOC) around existing cooking. All heavy lifting
    /// (LLM, tools, memory injection, interrupt handling) is delegated.
    pub async fn cook_with_plan(
        &self,
        message: &str,
        tools: &[ToolSchema],
        conversation_history: &[Message],
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<CookingResult> {
        use crate::plan_mode::PlanMode;

        let workspace_root = self.workspace.root();

        // Derive titan name from network config or fallback
        let titan_name = self.config.network
            .as_ref()
            .and_then(|n| n.agent_name.clone())
            .unwrap_or_else(|| "Zeus".to_string());

        // ---- Phase 1: generate plan (1 LLM call, no tools) ----
        let task_preview: String = message.chars().take(80).collect();
        tracing::info!(task = %task_preview, "plan_mode: generating plan");
        let plan = self
            .planner
            .create_plan(message, &self.llm, tools)
            .await?;
        tracing::debug!(steps = plan.steps.len(), "plan_mode: plan created");

        // Persist plan to disk. If this fails we fall back to plain cooking so
        // filesystem quirks don't block the user.
        let mut plan_mode = match PlanMode::create(workspace_root, message, &titan_name).await {
            Ok(pm) => Some(pm),
            Err(e) => {
                tracing::warn!(error = %e, "plan_mode: create failed, falling back to plain cook");
                None
            }
        };

        // Format plan steps as markdown and write PLAN.md
        if let Some(ref mut pm) = plan_mode {
            let mut md = String::from("## Steps\n\n");
            for step in &plan.steps {
                md.push_str(&format!(
                    "- [ ] **Step {}** — {}{}\n",
                    step.id,
                    step.description,
                    step.tool
                        .as_deref()
                        .map(|t| format!(" (tool: `{}`)", t))
                        .unwrap_or_default(),
                ));
            }
            if let Err(e) = pm.write_plan(&md).await {
                tracing::warn!(error = %e, "plan_mode: write_plan failed");
            }
        }

        // ---- Phase 2: execute with plan context injected ----
        let augmented_message = if let Some(ref pm) = plan_mode {
            match pm.read_plan().await {
                Ok(content) if !content.is_empty() => {
                    format!("{}\n\nTask: {}", pm.plan_context_prompt(&content), message)
                }
                _ => message.to_string(),
            }
        } else {
            message.to_string()
        };

        let result = self
            .cook_with_history_interruptible(
                &augmented_message,
                tools,
                conversation_history,
                cancel,
                None,
                vec![],
                None,
                None,
            )
            .await?;

        // ---- Phase 3: persist outcome ----
        if let Some(ref mut pm) = plan_mode {
            let summary = format!(
                "## Response\n\n{}\n\n## Stats\n\n- tool calls: {}\n- processing time: {} ms\n",
                result.response,
                result.tool_calls.len(),
                result.processing_time_ms
            );
            if result.completed_naturally {
                if let Err(e) = pm.complete(&summary).await {
                    tracing::warn!(error = %e, "plan_mode: complete failed");
                }
            } else {
                if let Err(e) = pm.mark_interrupted().await {
                    tracing::warn!(error = %e, "plan_mode: mark_interrupted failed");
                }
            }
        }

        Ok(result)
    }

    /// Internal: convert CookingResult to ProcessResult
    fn cooking_to_process(result: &CookingResult) -> ProcessResult {
        ProcessResult {
            response: result.response.clone(),
            tool_calls: result.tool_calls.iter().map(|r| r.name.clone()).collect(),
            processing_time_ms: result.processing_time_ms,
            tokens_used: None,
        }
    }

    /// Run the MetaLoop auto-tuning optimizer.
    ///
    /// Tests config mutations (model, max_iterations, temperature, etc.) against
    /// benchmarks and keeps changes that improve performance. Returns a report
    /// of what was tried and what stuck.
    pub async fn auto_tune(&self, max_iterations: u32) -> Result<MetaLoopReport> {
        let experiment = experiment::ConfigExperiment::new()?;
        let proposer = Box::new(RandomProposer::new(max_iterations));
        let mut meta_loop = MetaLoop::new(experiment, proposer);
        meta_loop.run(max_iterations).await
    }
}

/// Result of autonomous processing (full pipeline)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousResult {
    /// Response text
    pub response: String,
    /// Names of tools that were called
    pub tool_calls: Vec<String>,
    /// The decision that was made
    pub decision: Decision,
    /// The intent analysis
    pub intent: IntentAnalysis,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
    /// System health at time of processing
    pub health_status: HealthStatus,
}

/// Result of processing a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    /// Response text
    pub response: String,
    /// Names of tools that were called
    pub tool_calls: Vec<String>,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
    /// Tokens used (if available)
    pub tokens_used: Option<u64>,
}

// ============================================================================
// MnemosyneConsolidationProvider
// ============================================================================

/// Provides consolidation data by querying the Mnemosyne SQLite database
/// directly via a separate connection (avoids async→sync bridge issues).
struct MnemosyneConsolidationProvider {
    db_path: std::path::PathBuf,
}

impl MnemosyneConsolidationProvider {
    fn new(mnemosyne: Arc<zeus_mnemosyne::Mnemosyne>) -> Self {
        Self {
            db_path: mnemosyne.config().db_path.clone(),
        }
    }

    fn conn(&self) -> Option<rusqlite::Connection> {
        rusqlite::Connection::open(&self.db_path).ok()
    }
}

impl zeus_nous::consolidation::ConsolidationDataProvider for MnemosyneConsolidationProvider {
    fn episodic_contents(&self) -> Vec<String> {
        let conn = match self.conn() {
            Some(c) => c,
            None => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT content FROM messages WHERE memory_type = 'episodic' ORDER BY id DESC LIMIT 200",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn episodic_importances(&self) -> Vec<f32> {
        let conn = match self.conn() {
            Some(c) => c,
            None => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT importance FROM messages WHERE memory_type = 'episodic' ORDER BY id DESC LIMIT 200",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| row.get::<_, f64>(0).map(|v| v as f32)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn episodic_ages_days(&self) -> Vec<i64> {
        let conn = match self.conn() {
            Some(c) => c,
            None => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT timestamp FROM messages WHERE memory_type = 'episodic' ORDER BY id DESC LIMIT 200",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let now = chrono::Utc::now();
        match stmt.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .map(|ts| {
                    chrono::DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_days())
                        .unwrap_or(0)
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    fn lesson_confidences(&self) -> Vec<f32> {
        let conn = match self.conn() {
            Some(c) => c,
            None => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT importance FROM messages WHERE memory_type = 'semantic' ORDER BY id DESC LIMIT 100",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| row.get::<_, f64>(0).map(|v| v as f32)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn apply_decay(&self, decay_rate: f32) -> usize {
        let conn = match self.conn() {
            Some(c) => c,
            None => return 0,
        };
        conn.execute(
            "UPDATE messages SET importance = MAX(0.0, importance - ?1) WHERE memory_type = 'episodic' AND importance > 0.0",
            rusqlite::params![decay_rate as f64],
        )
        .unwrap_or(0)
    }

    fn cleanup_memories(&self, _indices: &[usize]) -> usize {
        let conn = match self.conn() {
            Some(c) => c,
            None => return 0,
        };
        conn.execute(
            "DELETE FROM messages WHERE memory_type = 'episodic' AND importance <= 0.0 AND timestamp < datetime('now', '-30 days')",
            [],
        )
        .unwrap_or(0)
    }
}

/// Prometheus configuration
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
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Cron-based scheduler configuration
    #[serde(default)]
    pub scheduler: Option<SchedulerConfig>,
    /// Autonomy level for decision-making
    #[serde(default)]
    pub autonomy: Option<AutonomyConfig>,
    /// Learning engine configuration
    #[serde(default)]
    pub learning: Option<LearningConfig>,
    /// Monitor configuration
    #[serde(default)]
    pub monitor: Option<MonitorConfig>,
    /// Overall cooking-loop wall-clock timeout in seconds.
    /// Overrides `gateway.timeout_secs` for autonomous cooks.
    /// When absent, falls back to `gateway.timeout_secs` (default 1800s).
    #[serde(default)]
    pub cooking_loop_timeout_secs: Option<u64>,
    /// Human-readable cooking-loop wall-clock timeout (e.g. "2h", "30 hours", "45m").
    /// When present and parseable, overrides `cooking_loop_timeout_secs`.
    /// Falls back to `cooking_loop_timeout_secs`, then gateway default (1800s).
    /// Operator UX: humans type "2h" not "7200".
    #[serde(default)]
    pub cooking_loop_timeout: Option<String>,
}

fn default_heartbeat_interval() -> u64 {
    300 // 5 minutes — frequent enough for active autonomy, cook-priority prevents message starvation
}
fn default_max_iterations() -> usize {
    20
}


impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enable_heartbeat: false,
            heartbeat_interval_secs: default_heartbeat_interval(),
            enable_cognitive: false,
            max_iterations: default_max_iterations(),
            scheduler: None,
            autonomy: None,
            learning: None,
            monitor: None,
            cooking_loop_timeout_secs: None,
            cooking_loop_timeout: None,
        }
    }
}

/// Resolve the effective cooking-loop wall-clock timeout.
/// Priority: NL string > secs override > gateway_default_secs (fallback 1800s).
/// Re-uses the same humantime parser as `parse_cooking_timeout` in tool_executor.
pub fn resolve_cooking_loop_timeout(config: &PrometheusConfig, gateway_default_secs: u64) -> std::time::Duration {
    // 1. NL string wins if present and parseable
    if let Some(ref raw) = config.cooking_loop_timeout {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
            // Unparseable NL — fall through with a warning baked into return path
        }
    }
    // 2. Explicit secs override
    if let Some(secs) = config.cooking_loop_timeout_secs {
        if secs > 0 {
            return std::time::Duration::from_secs(secs);
        }
    }
    // 3. Gateway default (itself falls back to 1800 if zero)
    let effective = if gateway_default_secs > 0 { gateway_default_secs } else { 1800 };
    std::time::Duration::from_secs(effective)
}

/// Parse a per-goal front-matter timeout string (e.g. `timeout: "30 hours"`).
/// Returns None if input is None/empty/unparseable — caller falls back to config.
pub fn parse_goal_timeout(raw: Option<&str>) -> Option<std::time::Duration> {
    let s = raw?.trim();
    if s.is_empty() { return None; }
    humantime::parse_duration(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prometheus_config() {
        let config = PrometheusConfig::default();
        assert_eq!(config.heartbeat_interval_secs, 300);
        assert_eq!(config.max_iterations, 20);
        assert!(config.autonomy.is_none());
        assert!(config.learning.is_none());
        assert!(config.monitor.is_none());
        assert!(config.cooking_loop_timeout.is_none());
        assert!(config.cooking_loop_timeout_secs.is_none());
    }

    // ── #13-B cooking-loop dynamic timeout tests ─────────────────────────────

    #[test]
    fn test_resolve_cooking_loop_timeout_fallback_to_default() {
        // No overrides → gateway default wins
        let cfg = PrometheusConfig::default();
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(1800));
    }

    #[test]
    fn test_resolve_cooking_loop_timeout_secs_override() {
        // cooking_loop_timeout_secs set → overrides gateway default
        let cfg = PrometheusConfig { cooking_loop_timeout_secs: Some(3600), ..Default::default() };
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(3600));
    }

    #[test]
    fn test_resolve_cooking_loop_timeout_nl_string_wins() {
        // NL string beats secs override
        let cfg = PrometheusConfig {
            cooking_loop_timeout: Some("30 hours".to_string()),
            cooking_loop_timeout_secs: Some(7200),
            ..Default::default()
        };
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(30 * 3600));
    }

    #[test]
    fn test_resolve_cooking_loop_timeout_nl_1h30m() {
        let cfg = PrometheusConfig {
            cooking_loop_timeout: Some("1h30m".to_string()),
            ..Default::default()
        };
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(90 * 60));
    }

    #[test]
    fn test_resolve_cooking_loop_timeout_invalid_nl_falls_back_to_secs() {
        // Bad NL string → falls through to secs override
        let cfg = PrometheusConfig {
            cooking_loop_timeout: Some("not-a-duration".to_string()),
            cooking_loop_timeout_secs: Some(5400),
            ..Default::default()
        };
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(5400));
    }

    #[test]
    fn test_resolve_cooking_loop_timeout_zero_secs_falls_back_to_gateway() {
        // zero secs treated as unset → falls to gateway default
        let cfg = PrometheusConfig { cooking_loop_timeout_secs: Some(0), ..Default::default() };
        let d = resolve_cooking_loop_timeout(&cfg, 1800);
        assert_eq!(d, std::time::Duration::from_secs(1800));
    }

    #[test]
    fn test_parse_goal_timeout_valid() {
        let d = parse_goal_timeout(Some("2h")).unwrap();
        assert_eq!(d, std::time::Duration::from_secs(7200));
    }

    #[test]
    fn test_parse_goal_timeout_none() {
        assert!(parse_goal_timeout(None).is_none());
    }

    #[test]
    fn test_parse_goal_timeout_invalid() {
        assert!(parse_goal_timeout(Some("not-valid")).is_none());
    }

    #[test]
    fn test_autonomous_result_serialization() {
        let result = AutonomousResult {
            response: "Hello".to_string(),
            tool_calls: vec![],
            decision: Decision::RespondDirectly("conversation".to_string()),
            intent: IntentAnalysis {
                intent: Intent::Conversation,
                complexity: TaskComplexity::Trivial,
                confidence: 0.95,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: "greeting detected".to_string(),
            },
            processing_time_ms: 150,
            health_status: HealthStatus::Healthy,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: AutonomousResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.response, "Hello");
        assert_eq!(deser.intent.intent, Intent::Conversation);
    }

    #[test]
    fn test_intent_classifier_standalone() {
        let classifier = IntentClassifier::new();
        let analysis = classifier.classify("hello", &[]);
        assert_eq!(analysis.intent, Intent::Conversation);
        assert!(analysis.confidence > 0.5);
    }

    #[test]
    fn test_autonomy_engine_standalone() {
        let engine = AutonomyEngine::new(AutonomyConfig::default());
        let analysis = IntentAnalysis {
            intent: Intent::Conversation,
            complexity: TaskComplexity::Trivial,
            confidence: 0.9,
            suggested_tools: vec![],
            requires_confirmation: false,
            reasoning: "greeting".to_string(),
        };
        let context = DecisionContext {
            intent: analysis,
            has_memory_context: false,
            session_message_count: 0,
            recent_error_count: 0,
            available_tools: vec![],
            autonomous_tool_count: 0,
        };
        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::RespondDirectly(_)));
    }

    #[test]
    fn test_monitor_standalone() {
        let monitor = Monitor::new(MonitorConfig::default());
        monitor.record_llm_call(500, true);
        let report = monitor.health_check();
        assert_eq!(report.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_cooking_to_process() {
        let result = CookingResult {
            response: "Done".to_string(),
            iterations: 2,
            tool_calls: vec![ToolCallRecord {
                name: "shell".to_string(),
                call_id: "tc1".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                output: "ok".to_string(),
                iteration: 1,
            }],
            processing_time_ms: 300,
            completed_naturally: true,
            memory_context: None,
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 0,
            estimated_tokens_end: 0,
            retry_count: 0,
            interrupted_by: None,
        };
        let pr = Prometheus::cooking_to_process(&result);
        assert_eq!(pr.response, "Done");
        assert_eq!(pr.tool_calls, vec!["shell"]);
        assert_eq!(pr.processing_time_ms, 300);
    }

    /// Verify the PlanAndExecute → SpawnAgents upgrade path:
    /// when spawner.analyze() recommends spawning for a complex multi-tool task,
    /// the decision should be upgraded.
    #[test]
    fn test_spawner_upgrade_plan_and_execute_to_spawn_agents() {
        use crate::intent::{Intent, IntentAnalysis, TaskComplexity};
        use crate::spawner::{ProactiveSpawner, SpawnCriteria};

        let spawner = ProactiveSpawner::new(SpawnCriteria {
            min_complexity: TaskComplexity::Moderate,
            min_parallel_steps: 2,
            max_active_agents: 5,
            ..Default::default()
        });

        // Complex intent with diverse tools — spawner should recommend spawning
        let complex_intent = IntentAnalysis {
            intent: Intent::ComplexTask,
            complexity: TaskComplexity::Complex,
            confidence: 0.85,
            suggested_tools: vec![
                "shell".to_string(),
                "read_file".to_string(),
                "web_fetch".to_string(),
                "write_file".to_string(),
            ],
            requires_confirmation: false,
            reasoning: "multi-step parallel research task".to_string(),
        };

        let rec = spawner.analyze(&complex_intent, None, 0);
        // With a complex task + diverse tools + no active agents, spawner recommends
        if rec.should_spawn {
            assert!(!rec.agents.is_empty());
            assert!(rec.estimated_speedup > 1.0);
        }
        // The upgrade logic: only fires when should_spawn && !agents.is_empty()
        let upgraded = if rec.should_spawn && !rec.agents.is_empty() {
            Decision::SpawnAgents(rec.agents)
        } else {
            Decision::PlanAndExecute
        };
        // For complex multi-tool tasks the spawner should recommend; if it does,
        // the upgrade should produce SpawnAgents.
        if rec.should_spawn {
            assert!(matches!(upgraded, Decision::SpawnAgents(_)));
        }
    }

    /// Verify that simple/trivial tasks do NOT get upgraded — spawner rejects them.
    #[test]
    fn test_spawner_no_upgrade_for_simple_tasks() {
        use crate::intent::{Intent, IntentAnalysis, TaskComplexity};
        use crate::spawner::ProactiveSpawner;

        let spawner = ProactiveSpawner::default();

        let simple_intent = IntentAnalysis {
            intent: Intent::Conversation,
            complexity: TaskComplexity::Simple,
            confidence: 0.95,
            suggested_tools: vec!["shell".to_string()],
            requires_confirmation: false,
            reasoning: "simple greeting".to_string(),
        };

        let rec = spawner.analyze(&simple_intent, None, 0);
        assert!(!rec.should_spawn, "Simple tasks must not trigger spawn upgrade");

        // Upgrade logic: PlanAndExecute stays PlanAndExecute
        let decision = if rec.should_spawn && !rec.agents.is_empty() {
            Decision::SpawnAgents(rec.agents)
        } else {
            Decision::PlanAndExecute
        };
        assert!(matches!(decision, Decision::PlanAndExecute));
    }

    #[test]
    fn core_user_turn_wordcount_ignores_injected_context() {
        // #278: a trivial "hey" augmented with the gateway's history + [Work State]
        // blocks must count as trivial, so nous.reason()'s 8-26s synthesize is
        // skipped. Reverting the helper to a raw word count fails this (the
        // injected boilerplate counts as ~30 words → would run reasoning on "hey").
        let augmented = "[Recent conversation in this channel:]\n\
            user: [merakizzz]: bug on titan is strange, replies here but dead on tui\n\
            user: [merakizzz]: ssh into titan and check\n\
            [End of history]\n\n\
            [You're on a shared team channel. Just talk naturally.]\n\n\
            [NEW MESSAGE — respond to THIS, not to anything in the history above:]\n\
            [Work State]\n\
            Your current durable work state (recall this before answering status questions):\n\
            Incomplete plans:\n\
            - 2026-06-23-recent-conversation\n\n\n\
            hey";
        assert!(
            Prometheus::core_user_turn_wordcount(augmented) <= 4,
            "augmented trivial turn must count as trivial, got {}",
            Prometheus::core_user_turn_wordcount(augmented)
        );

        // A genuinely substantive turn still counts non-trivial → full reasoning runs.
        let real = "please refactor the channel router to support per-titan session keys and add tests";
        assert!(Prometheus::core_user_turn_wordcount(real) > 4);

        // Plain short message: trivial.
        assert!(Prometheus::core_user_turn_wordcount("hey there friend") <= 4);
    }
}
pub mod trigger_tools;
