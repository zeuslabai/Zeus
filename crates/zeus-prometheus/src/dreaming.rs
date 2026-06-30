//! Dreaming — a background self-reflection loop (#130).
//!
//! Zeus consolidates memory only when something else triggers it. Dreaming adds
//! *autonomous* reflection: on a cron tick, with no human in the loop, the agent
//! "wakes," reviews recent recall, consolidates it into durable lessons, and
//! writes a short narrative of what it learned.
//!
//! Two phases, mirroring OpenClaw's `dreaming.ts`:
//!
//! - **Light** — frequent, shallow, **free**. Tidies recent recall through the
//!   existing [`ConsolidationEngine`]. Makes **no LLM call** (asserted in tests).
//! - **REM** — infrequent, deep, **one LLM call**. Synthesizes a reflection
//!   across a longer lookback, promotes high-confidence lessons, and appends a
//!   dated narrative to `MEMORY.md`.
//!
//! ## Guardrails (carried from Phase 1)
//! - **Off-by-default.** [`DreamingConfig::enabled`] defaults to `false`. When
//!   off, [`DreamingEngine::scheduler_tasks`] returns **zero** tasks — no cron
//!   jobs registered, no behavior change, no surprise LLM spend.
//! - **One scheduler.** Cadence is expressed as [`TaskConfig`]s handed to the
//!   shared `CronScheduler`. This module never forks its own loop.
//! - **Persona-latch wins.** Reflection narrative is advisory; it never
//!   overrides correctness/safety/permissions.

use crate::scheduler::{TaskConfig, TaskType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use zeus_core::Result;
use zeus_nous::consolidation::{ConsolidationEngine, ConsolidationResult};

// ============================================================================
// Config
// ============================================================================

/// Default cron for the light phase: every 6 hours. Free, so it can be frequent.
const DEFAULT_LIGHT_CRON: &str = "0 0 */6 * * * *";
/// Default cron for the REM phase: daily at 03:00. Costs one LLM call, so rare.
const DEFAULT_REM_CRON: &str = "0 0 3 * * * *";
/// Default lookback for the light phase (hours of recent recall to tidy).
const DEFAULT_LIGHT_LOOKBACK_HOURS: i64 = 12;
/// Default lookback for the REM phase (hours of recall to synthesize over).
const DEFAULT_REM_LOOKBACK_HOURS: i64 = 24;
/// Default cap on items pulled into a single REM pass — bounds LLM cost.
const DEFAULT_MAX_REM_ITEMS: usize = 200;

fn default_light_cron() -> String {
    DEFAULT_LIGHT_CRON.to_string()
}
fn default_rem_cron() -> String {
    DEFAULT_REM_CRON.to_string()
}
fn default_light_lookback_hours() -> i64 {
    DEFAULT_LIGHT_LOOKBACK_HOURS
}
fn default_rem_lookback_hours() -> i64 {
    DEFAULT_REM_LOOKBACK_HOURS
}
fn default_max_rem_items() -> usize {
    DEFAULT_MAX_REM_ITEMS
}

/// Configuration for the dreaming loop. **Off by default.**
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingConfig {
    /// Master switch. When `false` (default) no cron jobs are registered and
    /// the loop has zero effect. Off-by-default is load-bearing — turning this
    /// on is the only way to incur autonomous LLM spend.
    #[serde(default)]
    pub enabled: bool,

    /// Cron for the free light phase (recall tidy). 7-field.
    #[serde(default = "default_light_cron")]
    pub light_cron: String,

    /// Cron for the costed REM phase (deep synthesis). 7-field, keep infrequent.
    #[serde(default = "default_rem_cron")]
    pub rem_cron: String,

    /// How many hours of recent recall the light phase tidies.
    #[serde(default = "default_light_lookback_hours")]
    pub light_lookback_hours: i64,

    /// How many hours of recall the REM phase synthesizes over.
    #[serde(default = "default_rem_lookback_hours")]
    pub rem_lookback_hours: i64,

    /// Upper bound on items pulled into one REM pass. Bounds cost.
    #[serde(default = "default_max_rem_items")]
    pub max_rem_items: usize,
}

impl Default for DreamingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            light_cron: default_light_cron(),
            rem_cron: default_rem_cron(),
            light_lookback_hours: default_light_lookback_hours(),
            rem_lookback_hours: default_rem_lookback_hours(),
            max_rem_items: default_max_rem_items(),
        }
    }
}

// ============================================================================
// Phases
// ============================================================================

/// Which phase a wake tick is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DreamPhase {
    /// Frequent, shallow, free. No LLM call.
    Light,
    /// Infrequent, deep, one LLM call.
    Rem,
}

impl DreamPhase {
    /// Task name used to register / route this phase on the shared scheduler.
    pub fn task_name(self) -> &'static str {
        match self {
            DreamPhase::Light => "dreaming:light",
            DreamPhase::Rem => "dreaming:rem",
        }
    }

    /// Whether this phase is permitted to make an LLM call.
    pub fn uses_llm(self) -> bool {
        matches!(self, DreamPhase::Rem)
    }
}

/// Outcome of one wake tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamResult {
    /// Which phase produced this result.
    pub phase: DreamPhase,
    /// How many recall items were reviewed.
    pub items_reviewed: usize,
    /// Result of the consolidation pass (decay/promote/cleanup tallies).
    pub consolidation: ConsolidationResult,
    /// Lessons promoted to the semantic tier this tick.
    pub lessons_promoted: usize,
    /// The narrative written to `MEMORY.md` (REM only; `None` for light).
    pub narrative: Option<String>,
    /// Whether an LLM call was actually made.
    pub llm_called: bool,
    /// When the tick ran.
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Provider — injectable I/O so the engine is testable without a live LLM
// ============================================================================

/// One recalled memory item under review during a dream tick.
#[derive(Debug, Clone)]
pub struct RecallItem {
    /// Free-text content of the memory.
    pub content: String,
    /// Current importance score (drives decay).
    pub importance: f32,
    /// Age in days (drives cleanup eligibility).
    pub age_days: i64,
    /// If this item carries a lesson, its confidence (drives promotion).
    pub lesson_confidence: Option<f32>,
}

/// The side-effecting surface a dream tick needs. Implemented over
/// `zeus-mnemosyne` recall + `MEMORY.md` writes in production; mocked in tests.
///
/// Keeping this a trait is what lets the light phase prove (in a unit test) that
/// it never invokes [`synthesize_narrative`](DreamProvider::synthesize_narrative).
#[allow(async_fn_in_trait)]
pub trait DreamProvider: Send + Sync {
    /// Pull recall items from the last `lookback_hours`, newest first, capped
    /// at `limit`.
    async fn recall(&self, lookback_hours: i64, limit: usize) -> Result<Vec<RecallItem>>;

    /// Promote a lesson to the durable semantic tier. Returns `Ok(())` on store.
    async fn promote_lesson(&self, content: &str, confidence: f32) -> Result<()>;

    /// **The single LLM call.** Synthesize a short narrative from the reviewed
    /// items + the consolidation tallies. Only the REM phase calls this.
    async fn synthesize_narrative(
        &self,
        items: &[RecallItem],
        consolidation: &ConsolidationResult,
    ) -> Result<String>;

    /// Append a dated narrative entry to `MEMORY.md`.
    async fn append_narrative(&self, narrative: &str) -> Result<()>;
}

// ============================================================================
// Engine
// ============================================================================

/// Drives the two-phase dreaming loop over a [`DreamProvider`], reusing the
/// existing [`ConsolidationEngine`] for the actual memory math.
pub struct DreamingEngine {
    config: DreamingConfig,
    consolidation: ConsolidationEngine,
}

impl DreamingEngine {
    /// Build an engine from config. Reuses `ConsolidationEngine` — no new
    /// consolidation logic.
    pub fn new(config: DreamingConfig) -> Self {
        Self {
            consolidation: ConsolidationEngine::default(),
            config,
        }
    }

    /// Read-only view of the config.
    pub fn config(&self) -> &DreamingConfig {
        &self.config
    }

    /// Cron tasks to hand to the shared `CronScheduler`.
    ///
    /// Returns **empty** when disabled — this is the off-by-default guarantee
    /// at the scheduler boundary. When enabled, registers exactly two jobs
    /// (light + REM) as `Shell` no-op markers routed by task name; the gateway
    /// wake handler dispatches to [`tick`](Self::tick) by phase.
    pub fn scheduler_tasks(&self) -> Vec<TaskConfig> {
        if !self.config.enabled {
            return Vec::new();
        }
        vec![
            TaskConfig {
                name: DreamPhase::Light.task_name().to_string(),
                cron: self.config.light_cron.clone(),
                task_type: TaskType::SubsystemTick {
                    kind: DreamPhase::Light.task_name().to_string(),
                },
                enabled: true,
                run_at: None,
                run_once: false,
                wake_mode: crate::scheduler::WakeMode::Now,
            delivery_mode: crate::scheduler::DeliveryMode::Channel,
            },
            TaskConfig {
                name: DreamPhase::Rem.task_name().to_string(),
                cron: self.config.rem_cron.clone(),
                task_type: TaskType::SubsystemTick {
                    kind: DreamPhase::Rem.task_name().to_string(),
                },
                enabled: true,
                run_at: None,
                run_once: false,
                wake_mode: crate::scheduler::WakeMode::Now,
            delivery_mode: crate::scheduler::DeliveryMode::Channel,
            },
        ]
    }

    /// Run one wake tick for the given phase.
    ///
    /// - **Light**: tidy recent recall via consolidation. No LLM call, no
    ///   narrative. `llm_called == false` always.
    /// - **REM**: synthesize a narrative (one LLM call), promote high-confidence
    ///   lessons, append the narrative to `MEMORY.md`.
    pub async fn tick<P: DreamProvider>(&self, phase: DreamPhase, provider: &P) -> Result<DreamResult> {
        let (lookback, limit) = match phase {
            DreamPhase::Light => (self.config.light_lookback_hours, self.config.max_rem_items),
            DreamPhase::Rem => (self.config.rem_lookback_hours, self.config.max_rem_items),
        };

        let items = provider.recall(lookback, limit).await?;
        debug!(phase = ?phase, items = items.len(), "dream tick: recalled");

        // Shared consolidation math (decay/promote/cleanup) — reused, not rebuilt.
        let contents: Vec<String> = items.iter().map(|i| i.content.clone()).collect();
        let importances: Vec<f32> = items.iter().map(|i| i.importance).collect();
        let ages: Vec<i64> = items.iter().map(|i| i.age_days).collect();
        let confidences: Vec<f32> = items
            .iter()
            .filter_map(|i| i.lesson_confidence)
            .collect();

        let consolidation =
            self.consolidation
                .consolidate(&contents, &importances, &ages, &confidences);

        match phase {
            DreamPhase::Light => {
                // Free phase: tidy only. No LLM, no narrative, no promotion.
                info!(
                    patterns = consolidation.patterns_found,
                    "dreaming:light complete (free)"
                );
                Ok(DreamResult {
                    phase,
                    items_reviewed: items.len(),
                    consolidation,
                    lessons_promoted: 0,
                    narrative: None,
                    llm_called: false,
                    timestamp: Utc::now(),
                })
            }
            DreamPhase::Rem => {
                // Promote high-confidence lessons (the consolidation engine
                // selected which by index).
                let promote_idx = self
                    .consolidation
                    .select_for_promotion(&confidences);
                let mut promoted = 0usize;
                for &idx in &promote_idx {
                    // Map promotion index back to the originating recall item.
                    if let Some(item) = items
                        .iter()
                        .filter(|i| i.lesson_confidence.is_some())
                        .nth(idx)
                    {
                        provider
                            .promote_lesson(
                                &item.content,
                                item.lesson_confidence.unwrap_or(0.0),
                            )
                            .await?;
                        promoted += 1;
                    }
                }

                // The single LLM call.
                let narrative = provider
                    .synthesize_narrative(&items, &consolidation)
                    .await?;
                provider.append_narrative(&narrative).await?;

                info!(
                    promoted,
                    patterns = consolidation.patterns_found,
                    "dreaming:rem complete (1 LLM call)"
                );

                Ok(DreamResult {
                    phase,
                    items_reviewed: items.len(),
                    consolidation,
                    lessons_promoted: promoted,
                    narrative: Some(narrative),
                    llm_called: true,
                    timestamp: Utc::now(),
                })
            }
        }
    }
}

// ============================================================================
// Production provider — recall via Mnemosyne, narrative via the LLM,
// lesson/narrative writes via the Workspace (#143)
// ============================================================================

/// Production [`DreamProvider`] wired over the live substrate the scheduler
/// already holds:
///
/// - **`recall`** — pulls recent memories from [`zeus_mnemosyne::Mnemosyne`]
///   via a recency-ordered FTS sweep, mapping each hit into a [`RecallItem`].
/// - **`promote_lesson`** — appends the lesson to `MEMORY.md` through the
///   [`Workspace`] (the durable semantic tier the agent reads on boot).
/// - **`synthesize_narrative`** — the single REM-phase LLM call, via
///   [`LlmClient::complete`].
/// - **`append_narrative`** — writes the dated narrative back into `MEMORY.md`.
///
/// All four borrow shared state, so the provider is cheap to construct per tick
/// inside `execute_subsystem_tick` and never owns a second scheduler/loop.
pub struct WorkspaceDreamProvider<'a> {
    mnemosyne: &'a zeus_mnemosyne::Mnemosyne,
    workspace: &'a zeus_memory::Workspace,
    llm: &'a zeus_llm::LlmClient,
}

impl<'a> WorkspaceDreamProvider<'a> {
    /// Build a provider over already-open shared state.
    pub fn new(
        mnemosyne: &'a zeus_mnemosyne::Mnemosyne,
        workspace: &'a zeus_memory::Workspace,
        llm: &'a zeus_llm::LlmClient,
    ) -> Self {
        Self {
            mnemosyne,
            workspace,
            llm,
        }
    }
}

impl<'a> DreamProvider for WorkspaceDreamProvider<'a> {
    async fn recall(&self, _lookback_hours: i64, limit: usize) -> Result<Vec<RecallItem>> {
        // "Recent reflection material" = Mnemosyne's recency-ordered current
        // memories (ORDER BY importance DESC, timestamp DESC), capped at `limit`.
        // This is the closest live recall surface to a lookback window without
        // a time-range API that doesn't exist yet; the per-item `age_days`
        // derived below still lets the consolidation engine apply decay/cleanup.
        // Empty recall is a valid (no-op) tick.
        let results = self
            .mnemosyne
            .get_current_memories(limit)
            .await
            .unwrap_or_default();

        Ok(results
            .into_iter()
            .map(|r| RecallItem {
                content: r.content,
                importance: r.importance,
                // Mnemosyne timestamps are RFC3339 strings; age is best-effort.
                age_days: r
                    .timestamp
                    .parse::<DateTime<Utc>>()
                    .map(|t| (Utc::now() - t).num_days())
                    .unwrap_or(0),
                // FTS hits don't carry an explicit lesson confidence; importance
                // doubles as the promotion signal (consolidation thresholds it).
                lesson_confidence: Some(r.importance),
            })
            .collect())
    }

    async fn promote_lesson(&self, content: &str, confidence: f32) -> Result<()> {
        // Promotion = persist to the durable semantic tier the agent reads on
        // boot. `remember` appends a dated entry to MEMORY.md.
        let fact = format!("[dream:lesson conf={:.2}] {}", confidence, content);
        self.workspace.remember(&fact).await?;
        Ok(())
    }

    async fn synthesize_narrative(
        &self,
        items: &[RecallItem],
        consolidation: &ConsolidationResult,
    ) -> Result<String> {
        // The single LLM call. Bound the prompt so a large recall set can't blow
        // the context — the engine already caps `items` at `max_rem_items`.
        let mut review = String::new();
        for (i, item) in items.iter().take(20).enumerate() {
            review.push_str(&format!("{}. {}\n", i + 1, item.content));
        }

        let system = "You are the agent's dreaming reflection. In 2-4 sentences, \
            synthesize what was learned from the recent memories below into a \
            short, durable narrative written in the first person. No preamble, \
            no lists — just the reflection.";

        let prompt = format!(
            "Recent memories under review ({} items):\n{}\n\n\
             Consolidation this cycle: {} patterns found, {} promoted, \
             {} decayed.\n\nWrite the reflection.",
            items.len(),
            review,
            consolidation.patterns_found,
            consolidation.memories_promoted,
            consolidation.memories_decayed,
        );

        let messages = vec![zeus_core::Message::user(prompt)];
        let response = self
            .llm
            .complete(&messages, &[], Some(system))
            .await
            .map_err(|e| zeus_core::Error::Llm(format!("dream narrative LLM call: {}", e)))?;

        Ok(response.content.trim().to_string())
    }

    async fn append_narrative(&self, narrative: &str) -> Result<()> {
        let dated = format!("[dream:narrative] {}", narrative);
        self.workspace.remember(&dated).await?;
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Mock provider that counts LLM calls so the light-phase guarantee is
    /// directly assertable.
    #[derive(Default)]
    struct MockProvider {
        items: Vec<RecallItem>,
        llm_calls: Arc<AtomicUsize>,
        promotions: Arc<AtomicUsize>,
        narratives_written: Arc<AtomicUsize>,
    }

    impl DreamProvider for MockProvider {
        async fn recall(&self, _lookback_hours: i64, limit: usize) -> Result<Vec<RecallItem>> {
            Ok(self.items.iter().take(limit).cloned().collect())
        }
        async fn promote_lesson(&self, _content: &str, _confidence: f32) -> Result<()> {
            self.promotions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn synthesize_narrative(
            &self,
            _items: &[RecallItem],
            _c: &ConsolidationResult,
        ) -> Result<String> {
            self.llm_calls.fetch_add(1, Ordering::SeqCst);
            Ok("Today I noticed a recurring focus on the scheduler.".to_string())
        }
        async fn append_narrative(&self, _narrative: &str) -> Result<()> {
            self.narratives_written.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn sample_items() -> Vec<RecallItem> {
        vec![
            RecallItem {
                content: "Working on the scheduler today".to_string(),
                importance: 0.8,
                age_days: 1,
                lesson_confidence: Some(0.9),
            },
            RecallItem {
                content: "The scheduler cron parsing was tricky".to_string(),
                importance: 0.6,
                age_days: 2,
                lesson_confidence: Some(0.85),
            },
            RecallItem {
                content: "Random unrelated note".to_string(),
                importance: 0.2,
                age_days: 40,
                lesson_confidence: None,
            },
        ]
    }

    #[test]
    fn disabled_by_default_registers_zero_tasks() {
        // Off-by-default is load-bearing: no cron jobs, no LLM spend.
        let engine = DreamingEngine::new(DreamingConfig::default());
        assert!(!engine.config().enabled);
        assert!(engine.scheduler_tasks().is_empty());
    }

    #[test]
    fn enabled_registers_exactly_two_phases() {
        let engine = DreamingEngine::new(DreamingConfig {
            enabled: true,
            ..Default::default()
        });
        let tasks = engine.scheduler_tasks();
        assert_eq!(tasks.len(), 2);
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"dreaming:light"));
        assert!(names.contains(&"dreaming:rem"));

        // Wire-in: emitters must produce dispatchable SubsystemTicks, not
        // placeholder WorkspaceNotes. The kind carries the phase so the
        // scheduler's execute_subsystem_tick arm can route it.
        for t in &tasks {
            match &t.task_type {
                TaskType::SubsystemTick { kind } => {
                    assert!(
                        kind == "dreaming:light" || kind == "dreaming:rem",
                        "unexpected dreaming tick kind: {kind}"
                    );
                    assert_eq!(kind, &t.name, "tick kind must match task name");
                }
                other => panic!("dreaming must emit SubsystemTick, got {:?}", other),
            }
        }
    }

    #[tokio::test]
    async fn light_phase_makes_no_llm_call() {
        // The core free-phase guarantee.
        let provider = MockProvider {
            items: sample_items(),
            ..Default::default()
        };
        let engine = DreamingEngine::new(DreamingConfig {
            enabled: true,
            ..Default::default()
        });
        let result = engine.tick(DreamPhase::Light, &provider).await.unwrap();

        assert_eq!(result.phase, DreamPhase::Light);
        assert!(!result.llm_called);
        assert_eq!(provider.llm_calls.load(Ordering::SeqCst), 0);
        assert!(result.narrative.is_none());
        assert_eq!(result.lessons_promoted, 0);
        assert_eq!(provider.narratives_written.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rem_phase_writes_narrative_and_promotes() {
        let provider = MockProvider {
            items: sample_items(),
            ..Default::default()
        };
        let engine = DreamingEngine::new(DreamingConfig {
            enabled: true,
            ..Default::default()
        });
        let result = engine.tick(DreamPhase::Rem, &provider).await.unwrap();

        assert_eq!(result.phase, DreamPhase::Rem);
        assert!(result.llm_called);
        assert_eq!(provider.llm_calls.load(Ordering::SeqCst), 1);
        assert!(result.narrative.is_some());
        // Two promotable lessons in the sample (conf 0.9, 0.85).
        assert!(result.lessons_promoted >= 1);
        assert_eq!(provider.narratives_written.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn phase_metadata() {
        assert!(!DreamPhase::Light.uses_llm());
        assert!(DreamPhase::Rem.uses_llm());
        assert_eq!(DreamPhase::Light.task_name(), "dreaming:light");
        assert_eq!(DreamPhase::Rem.task_name(), "dreaming:rem");
    }

    /// #143: the production provider's recall reads live from Mnemosyne, and its
    /// promote/narrate writes land in the workspace `MEMORY.md`. Exercises every
    /// non-LLM seam end-to-end against real stores (no mock).
    #[tokio::test]
    async fn production_provider_recalls_and_writes_to_real_stores() {
        let dir = tempfile::tempdir().unwrap();

        // Real Mnemosyne over a temp db, seeded with one memory to recall.
        let mn = zeus_mnemosyne::Mnemosyne::new(zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("mn.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        })
        .await
        .unwrap();
        mn.store(
            "sess-1",
            &zeus_core::Message::user("Shipped the dreaming provider wire-in"),
        )
        .await
        .unwrap();

        // Real Workspace over a temp dir.
        let ws = zeus_memory::Workspace::new(dir.path().join("ws"));

        // We never call synthesize_narrative here, so the LLM client is unused;
        // build a throwaway just to satisfy the borrow.
        let llm =
            zeus_llm::LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();
        let provider = WorkspaceDreamProvider::new(&mn, &ws, &llm);

        // recall() pulls the seeded memory back out of the live store.
        let items = provider.recall(24, 10).await.unwrap();
        assert!(
            items.iter().any(|i| i.content.contains("dreaming provider")),
            "recall should surface the seeded memory, got: {:?}",
            items.iter().map(|i| &i.content).collect::<Vec<_>>()
        );

        // promote_lesson() + append_narrative() persist to MEMORY.md.
        provider
            .promote_lesson("Off-by-default holds at the enabled gate", 0.95)
            .await
            .unwrap();
        provider
            .append_narrative("I learned the scheduler routes one tick per phase.")
            .await
            .unwrap();

        let memory = ws.get_memory().await.unwrap();
        assert!(
            memory.contains("dream:lesson") && memory.contains("enabled gate"),
            "promoted lesson should be in MEMORY.md"
        );
        assert!(
            memory.contains("dream:narrative") && memory.contains("one tick per phase"),
            "narrative should be in MEMORY.md"
        );
    }
}
