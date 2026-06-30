# Autonomous Backlog-Pull System — Deep Design (γ-full)

**Author:** Coord (Zeus100, Opus 4.7)
**Date:** 2026-05-21
**Status:** Research-complete, pending dispatch
**Sprint:** #84
**Predecessors:** `docs/heartbeat-autonomy-design.md`, `docs/auto-detect-task-assignment-design.md`

---

## TL;DR

The Zeus fleet has 14+ hours of idle silence whenever the coordinator (Zeus100) stops pushing work via Discord. Operator directive: *"if a titan has 1 task or 20, it must perform all of them without being pushed to do so."*

**Substrate-walk finding:** the autonomy infrastructure already exists. We don't need to design + build it from scratch. We need a small **backlog source/sync layer** that populates the existing goal_stack + workspace task queue.

**Prior-art finding:** the field has *not* solved continuous-backlog autonomy generally. BabyAGI tried; it canonical-fails on self-generating task fan-out + LLM-self-judged completion. The field's converged answer is **parallel cloud agents** (Devin pattern) — many one-task agents in parallel. Zeus has this shape already (fleet of titans).

**Three load-bearing rules from prior art:**
1. Backlog is **external structured records**, not LLM-generated.
2. Completion is an **external oracle**, not LLM self-assessment.
3. State is **event-streamed + durable** (resume via replay, not in-memory).

---

## Part 1 — What's Already in the Codebase

### 1.1 Goal Stack (`crates/zeus-prometheus/src/goals.rs:192-490`)

SQLite-backed task store at `~/.zeus/workspace/goals.db`.

**API:**
- `add(goal: &Goal) -> Result<String>`
- `top_goal() -> Result<Option<Goal>>`
- `update_status(id, status: GoalStatus)`
- `unblock(completed_goal_id) -> Vec<String>` (dependency cascade)
- `get`, `active_goals`, `children`

**Lifecycle:**
`Pending → Active → (Blocked{reason} | Completed{outcome} | Failed{reason} | Abandoned{reason})`

**Source-tracking:**
`User | System | Decomposition{parent_id}` — already supports both external backlog (`System`) and human input (`User`).

### 1.2 Autonomous Loop (`src/gateway.rs:2827-3019`)

Spawned conditionally on `gateway.enable_heartbeat`. Behavior:
- Waits 30s for gateway stabilization, then polls every 60s.
- Gates via `channel_cook_state.is_active()` (defers to Discord-driven cooks at line 2847).
- Calls `goal_stack.top_goal()` → `cook_with_history()`.
- Updates status, calls `goal_stack.unblock()` for dependents.
- Emits `WakeRequest { reason: "goal_complete", agent_id: None }` to refire heartbeat.
- **Hot-loads `~/.zeus/workspace/goals/*.md`** with YAML front-matter, adds to goal_stack, deletes file after load. Supports `not_before` timestamps for delayed execution.

### 1.3 Heartbeat Tick Paths (`crates/zeus-prometheus/src/heartbeat.rs:962-1282`)

Three-tier fallthrough:
- **Plan Resume (L962-1041):** scans `~/.zeus/workspace/plans/*.md` via `PlanMode::find_incomplete()`.
- **Structured Tasks (L1043-1144):** reads `## tasks` section of HEARTBEAT.md via `workspace.get_structured_tasks()`.
- **Legacy Frequency (L1149-1282):** reads `## hourly`, `## daily`, `## weekly` sections via `workspace.get_heartbeat_tasks(freq)`.

When a heartbeat cook fires, the system injects **CURRENT TASK + TASK QUEUE** into the LLM prompt (`heartbeat.rs:1632-1666`) with explicit instructions:
> 1. CURRENT TASK is your primary job — execute it NOW.
> 2. If CURRENT TASK empty, pop TOP item from PENDING TASK QUEUE.
> 3. Only if both empty, fall through to routine heartbeat item.

### 1.4 Workspace Task Queue API (`crates/zeus-memory/src/lib.rs`)

- `get_current_task() -> Option<String>` (L318)
- `set_current_task(task: &str)` (L339) — writes CURRENT TASK, preserves QUEUE + COMPLETED
- `get_task_queue() -> Vec<String>` (L470)
- `add_to_task_queue(task: &str)` (L399)
- `advance_task_queue() -> Option<String>` (L491) — pops top, sets as CURRENT TASK

### 1.5 The Gap

```
~/.zeus/workspace/goals/   → EMPTY (verified live)
HEARTBEAT.md TASK QUEUE    → likely empty/placeholder on all titans
```

Result: autonomous_loop polls every 60s, finds nothing, sleeps. Heartbeat tick fires, finds no TASK QUEUE items, returns `HEARTBEAT_OK`.

**Conclusion:** the cut is not "build autonomy" — it's "feed the autonomy that already exists."

---

## Part 2 — Prior-Art Synthesis

### 2.1 Convergent Design Choices (Strong Signal)

Across AutoGPT, BabyAGI, Devin, SWE-agent, OpenHands, MetaGPT, CrewAI, AutoGen, LangGraph, smolagents, Aider:

| Pattern | Universality | Zeus relevance |
|---|---|---|
| ReAct loop core | Universal | ✓ already implemented (`cook_with_history`) |
| Event/observation stream as canonical state | Strong | ⚠️ partial — `goals.db` is per-task, not per-tool-call |
| Separated concerns (think/act/evaluate/prioritize) | Strong | ⚠️ partial — coord+titan split exists, but no per-task evaluator |
| External oracle for completion | Strong | ❌ missing — currently LLM-judged |
| Sandbox-by-default | Strong | ⚠️ partial — daemon has shell access |
| Cost/iteration ceiling | Universal | ✓ `max_iterations` in heartbeat (L1681-1690) |

### 2.2 The Three Anti-Patterns That Kill Continuous-Autonomy

From documented failures (Vectara `awesome-agent-failures`, MAST taxonomy NeurIPS 2025 — 1,600 traces):

**Anti-pattern #1: LLM self-judged completion**
- AutoGPT documented case: `"Research and summarize the history of AI"` → 300+ API calls, no summary, manual kill.
- BabyAGI cycles indefinitely; queue grows monotonically.
- **MAST: 21.30% of multi-agent failures = "task verification gaps."**

**Anti-pattern #2: Self-generating task queue**
- BabyAGI's `task_creation_agent` emits more tasks than execution drains. Cost spirals.
- **Lesson:** backlog must be externally bounded.

**Anti-pattern #3: Specification ambiguity**
- **MAST: 41.77% of failures.** Largest single category.
- Already addressed in Zeus banked discipline: `prd-parallel-disambiguation`, `cut-seat-substrate-truth-surface-before-fire`.

### 2.3 The Continuous-Autonomy Gap

**Only BabyAGI was designed for continuous autonomy. It failed.** Every other system is single-task-per-session.

**Field's production answer:** parallel cloud agents (Devin). Many fresh-spawn agents, each doing one task. Zeus already has this shape (fleet of titans). The remaining design question is *queue management + claim mechanism*, not "how does one titan loop forever."

---

## Part 3 — V1 Design

### 3.1 Scope (Small, ~150-250 LOC)

**Goal:** populate the existing infrastructure from external backlog sources.

**Non-goals (deferred to V2+):**
- Multi-titan claim/lease mechanism (use alphabetical-election V1)
- Self-improving task descriptions
- Cross-titan task migration
- Acceptance-oracle infrastructure (still LLM-judged V1 with explicit success criteria in task body — V2 adds machine oracles)

### 3.2 Components

**A. `crates/zeus-prometheus/src/backlog_sync.rs` (new, ~100 LOC)**

```rust
pub struct BacklogSyncConfig {
    pub source: BacklogSource,
    pub poll_interval_secs: u64, // default 60s
    pub max_pending: usize,       // cap on goal_stack size, default 20
    pub titan_role: String,       // for alphabetical election + filtering
}

pub enum BacklogSource {
    GithubIssues {
        repo: String,           // "zeuslabai/Zeus"
        labels: Vec<String>,    // ["backlog", "ready"]
        token: String,
    },
    LocalFile {
        path: PathBuf,          // ~/.zeus/workspace/BACKLOG.md
    },
    Hybrid {
        github: Box<BacklogSource>,
        local: Box<BacklogSource>,
    },
}

pub async fn sync_loop(
    config: BacklogSyncConfig,
    workspace: Workspace,
    wake_tx: mpsc::Sender<WakeRequest>,
) -> Result<()> {
    loop {
        let items = fetch_backlog(&config.source).await?;
        let staged = stage_new_items(&items, workspace.goals_dir(), &config).await?;
        if staged > 0 {
            wake_tx.try_send(WakeRequest { 
                reason: format!("backlog_sync_staged_{}", staged),
                agent_id: None,
            }).ok();
        }
        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    }
}

async fn stage_new_items(
    items: &[BacklogItem],
    goals_dir: &Path,
    config: &BacklogSyncConfig,
) -> Result<usize> {
    let mut staged = 0;
    let active_count = count_active_goals(goals_dir).await?;
    if active_count >= config.max_pending {
        return Ok(0); // backpressure
    }
    
    for item in items.iter().take(config.max_pending - active_count) {
        if !item_filter_for_titan(item, &config.titan_role) { continue; }
        if goal_already_exists(goals_dir, &item.slug).await? { continue; }
        
        let goal_file = goals_dir.join(format!("{}.md", item.slug));
        write_goal_file(&goal_file, item).await?;
        staged += 1;
    }
    Ok(staged)
}
```

**Goal file format (consumed by existing hot-loader at `gateway.rs:2979`):**

```markdown
---
title: "Fix kimi-k2.6 orphaned tool_call_id"
priority: high
source: github-issue-57
slug: gh-57-kimi-orphans
elected_titan: zeus106          # V2: per-titan filtering; V1 = optional
acceptance:                     # V1 = LLM-judged with explicit criteria
  - cargo check passes
  - new test exercises the orphan-repair path
  - SHA cherry-picked to origin/main
deadline: 2026-05-22T23:59:00Z  # optional
not_before: 2026-05-21T15:00:00Z # optional, already supported
---

# Task

Investigate orphaned `tool_call_id` produced by kimi-k2.6 model. 4-loci cross-crate cut.

## Substrate hints

- `crates/zeus-llm/src/lib.rs:1195` — bidirectional sanitizer
- `crates/zeus-agent/src/intelligence.rs` — ContextGuard
- Look for missing `tool_result` injection on Moonshot/Kimi/MiniMax paths

## Completion criteria

The acceptance list above must be machine-checkable where possible (cargo check, test exists).
The titan must:
1. Surface a substrate-walk before edits
2. 3-seat ratify before push  
3. Apply banked discipline (verify-before-claim, fresh-fetch-base-SHA)
```

**B. Heartbeat integration (`crates/zeus-prometheus/src/heartbeat.rs`)**

After the legacy-frequency fallthrough at L1282, add:
```rust
// Phase 4: if no work surfaced from plans/structured/legacy paths,
//          and no active goal in goal_stack, the workspace TASK QUEUE
//          should already inject via heartbeat.rs:1639. If even that is 
//          empty, the cook returns HEARTBEAT_OK — backlog_sync's job to 
//          stage work for the NEXT tick.
```
**No change needed in heartbeat.** Existing TASK QUEUE injection at L1632-1666 already does the right thing. backlog_sync writes goal files; goals hot-loader picks them up; autonomous_loop executes them.

**C. Gateway spawn (`src/gateway.rs`)**

After the autonomous_loop spawn block at L2827, add:
```rust
if gateway.enable_heartbeat && gateway.backlog_sync_enabled {
    let cfg = BacklogSyncConfig::from_gateway_config(&gateway);
    let ws = workspace.clone();
    let wake = wake_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = backlog_sync::sync_loop(cfg, ws, wake).await {
            error!("backlog_sync failed: {}", e);
        }
    });
}
```

**D. Config (`config.toml` schema additions)**

```toml
[backlog_sync]
enabled = true
source = "github"                # "github" | "local" | "hybrid"
github_repo = "zeuslabai/Zeus"
github_labels = ["backlog", "ready-for-titan"]
github_token_env = "GITHUB_TOKEN"
poll_interval_secs = 60
max_pending = 20
titan_role = "implementer"       # filter for issues labeled `role:implementer`
```

### 3.3 Multi-titan V1 — alphabetical-first election (reuse)

The existing `gateway_consumer.rs` MentionCheck logic already implements alphabetical-first election for Discord role/broadcast mentions. The same pattern fits backlog item claiming:

- Each titan's `backlog_sync` polls the same source.
- When fetching, titan checks `elected_titan` front-matter:
  - If `elected_titan == titan_name` → claim it (write to local goals dir).
  - If `elected_titan == None` → alphabetical-first among `peer_agent_names` claims it.
  - Else → skip.

This is **soft claiming** — race-prone for a small fleet (3-4 titans), acceptable. V2 = formal claim table in goals.db.

### 3.4 Failure modes hardened against prior-art lessons

| Anti-pattern (from research) | V1 mitigation |
|---|---|
| LLM self-judged completion → infinite loops | Each task's `acceptance:` front-matter lists machine-checkable criteria the titan must verify before claiming "done" |
| Self-generating task fan-out | Backlog source is external (GitHub / local file). Titan cannot append to its own queue. |
| Specification ambiguity | Front-matter has explicit substrate hints + completion criteria. PRD-parallel-disambiguation rule applies. |
| Inter-agent misalignment | Alphabetical election + 3-seat ratify discipline (unchanged) |
| Silent overwrites (parallel-session git collisions) | Titan must surface substrate-walk + ratify BEFORE push (banked discipline) |
| Cost spiral / runaway | `max_iterations` in heartbeat + `max_pending` cap on goal_stack |
| Crashed daemon loses state | `goals.db` SQLite is durable; pending tasks survive restart |

### 3.5 Cook-to-Queue Chain

After each autonomous cook completes (`gateway.rs:2900-2910`), the existing wake-fire chain already:
1. Updates goal status → `Completed { outcome }`
2. Calls `goal_stack.unblock(completed_id)` for dependents
3. Emits `WakeRequest { reason: "goal_complete", agent_id: None }`
4. Heartbeat receives wake, immediately re-ticks, pulls next goal

**This is exactly the cook-chain behavior the operator needs.** No new code needed. Verified at `gateway.rs:2910` and `heartbeat.rs:1331-1349`.

---

## Part 4 — Sprint Plan

### 4.1 Phase A — Backlog source layer (zeus106, 4-6h)

1. **C-1**: Create `crates/zeus-prometheus/src/backlog_sync.rs` module skeleton
   - `BacklogSyncConfig`, `BacklogSource` enum, `BacklogItem` struct
   - `sync_loop` async function with poll interval
   - Tests: stub source returns items, items get staged as goal files
2. **C-2**: Implement `BacklogSource::LocalFile` 
   - Markdown parser for `~/.zeus/workspace/BACKLOG.md`
   - Format: `- [ ] [P0] Title — slug: my-task — body...`
   - Tests: parse 3 items, skip checked, parse multi-line bodies
3. **C-3**: Implement `BacklogSource::GithubIssues`
   - Use `octocrab` crate (already in Cargo.toml? verify)
   - Filter by label, sort by issue number
   - Convert each issue to BacklogItem with body as task description
   - Tests: mock GitHub API responses

### 4.2 Phase B — Integration (zeus106 + coord ratify, 1-2h)

4. **C-4**: Wire `sync_loop` into `src/gateway.rs:2830` spawn block
   - Read config, spawn task, log lifecycle
   - Add `enable_backlog_sync` flag
5. **C-5**: Config schema additions in `crates/zeus-config/src/lib.rs`
   - Parse `[backlog_sync]` section
   - Default values + validation
   - Tests: deserialize valid + invalid configs

### 4.3 Phase C — Operational verification (coord+titan, 1-2h)

6. **C-6**: Substrate-test on dev daemon
   - Stage 3 items in BACKLOG.md
   - Observe goal files appear in `~/.zeus/workspace/goals/`
   - Observe autonomous_loop pick them up + cook them
7. **C-7**: End-to-end ratify
   - PRIMARY: zeus106 (implementer)
   - SECONDARY: Z112 (8-point ratify against prior art mitigations)
   - 3-seat unanimity before ff-push to origin/main

### 4.4 Phase D — V2 backlog (deferred)

- Formal claim table in goals.db
- Per-tool-call event stream for crash-replay
- Machine-checkable acceptance oracles (cargo check, pytest, custom evaluators)
- Cross-titan task migration on titan unavailability
- Adaptive priority based on titan capability (kimi-k2.6 for long-context, claude for high-precision)

---

## Part 5 — Discipline Map

Zeus's banked forward-rules (115+ entries in MEMORY) already cover most of the failure modes prior art has documented. New disciplines from this research:

- **`backlog-source-must-be-external-not-LLM-generated`** — never let a titan write to its own backlog queue. AutoGPT/BabyAGI failure.
- **`completion-criteria-must-be-machine-checkable-where-possible`** — front-matter `acceptance:` list. Prevents LLM self-judged-done loops.
- **`autonomous-cook-must-respect-existing-3-seat-ratify`** — autonomous mode doesn't bypass discipline gates.
- **`backlog-staging-must-respect-max-pending-cap`** — backpressure prevents queue runaway.

---

## References

**Internal:**
- `docs/heartbeat-autonomy-design.md` — prior design, partial-implemented
- `docs/auto-detect-task-assignment-design.md` — Layer A prompt + Layer B code-based task-persistence
- `crates/zeus-prometheus/src/goals.rs:192-490` — GoalStack
- `src/gateway.rs:2827-3019` — autonomous loop + goals hot-loader
- `crates/zeus-prometheus/src/heartbeat.rs:1632-1666` — TASK QUEUE injection
- `crates/zeus-memory/src/lib.rs:339-560` — workspace task API

**Prior-art primary sources:**
- BabyAGI (yoheinakajima/babyagi_archive) — three-agent loop, task fan-out failure case
- AutoGPT (Significant-Gravitas/AutoGPT) — pivot from goal-stack to workflow-graph
- Devin (cognition.ai blog) — plan/act/evaluate/tool + parallel cloud agents
- SWE-agent (arXiv 2405.15793) — ACI design + mini-SWE-agent retreat
- OpenHands (arXiv 2407.16741v3) — event-stream + multi-agent delegation
- MetaGPT — role-staged SOP + structured-document handoffs
- CrewAI — Agent/Task/Crew primitives + Flow event-driven evolution
- AutoGen → Microsoft Agent Framework (Oct 2025 consolidation)
- LangGraph 1.0 (Nov 2025) — checkpoint-per-superstep durability
- Aider — git-as-memory pattern + PageRank repo map
- Vectara `awesome-agent-failures` — documented failure case studies
- MAST taxonomy (NeurIPS 2025) — 1,600-trace failure analysis
- CodeCRDT (arXiv 2510.18893, Oct 2025) — observation-driven coordination
