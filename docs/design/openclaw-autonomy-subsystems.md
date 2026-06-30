# OpenClaw Autonomy Subsystems — Design Specs (Phase 2)

**Author:** zeus-spark (Herald) · **Base:** `f95aa575` (origin/main, post-#137) · **Status:** cut-ready for engineering seats

Phase 1 (#137) made every titan *behave* more autonomously through shared prompt sections. Phase 2 gives them new autonomous *capabilities*. Four bounded cuts: **#130 dreaming**, **#131 taskflow**, **#132 commitments**, **#133 standing-orders**.

Each spec below states: what it does · the substrate we already have · the cut shape · the hook points · acceptance. I substrate-walked the repo before writing — the "what we have" lines are real files, not assumptions, so each cut starts from truth.

---

## Substrate map (the truth on the ground)

| Concern | What exists today | File |
|---|---|---|
| Heartbeat loop | `HeartbeatConfig`, task-complexity timeouts, `CURRENT TASK` extraction | `crates/zeus-prometheus/src/heartbeat.rs` |
| Standing orders | **Already built** — SQLite store + `HEARTBEAT.md` parse + boot reload | `crates/zeus-prometheus/src/standing_orders.rs` |
| Cron / schedule | `schedule_create/list/delete` tools over `scheduler.db` | `crates/zeus-talos/src/scheduler_tools.rs`, `zeus-prometheus/src/trigger_tools.rs`, `scheduler.rs` |
| Meta loop | Periodic self-driven loop scaffold | `crates/zeus-prometheus/src/meta_loop.rs` |
| Memory consolidation | Decay / pattern-extract / promote-to-semantic engine | `crates/zeus-nous/src/consolidation.rs` |
| Meta-cognition | `Reflection`, `MetaCognition`, `Identity`, `Capability` | `crates/zeus-nous/src/meta/mod.rs` |
| Memory store | Vector + FTS recall | `crates/zeus-mnemosyne`, `crates/zeus-memory` |

**Key finding:** two of the four are *deepenings*, not greenfield. `standing_orders.rs` already exists (#133 deepens it). Dreaming (#130) has most of its machinery in `nous/consolidation.rs` + `nous/meta` — what's missing is the *wake loop* that drives them. Commitments (#132) and taskflow (#131) are the genuinely new builds.

---

## #130 — Dreaming

### What it does
A background self-reflection loop. On a cron tick (no human in the loop), the agent wakes, reviews recent sessions + recently-recalled memories, consolidates them into durable lessons, and writes a short narrative of "what I learned." Zeus currently has **0** autonomous reflection — memory only consolidates when something else triggers it.

OpenClaw runs two phases (`extensions/memory-core/dreaming.ts`): **light** (frequent, shallow — tidy recent recall) and **REM** (infrequent, deep — extract reflections/insights across a longer lookback). We mirror that two-phase shape because the cost/value tradeoff is real: cheap tidy often, expensive synthesis rarely.

### Substrate we already have
- `nous/consolidation.rs::ConsolidationEngine` — already does decay, `extract_patterns`, `select_for_promotion`. **This is the REM payload.** Don't rebuild it.
- `nous/meta/mod.rs::MetaCognition` + `Reflection` — the structure to record a reflection.
- `zeus-mnemosyne` — the recall source to review.
- `prometheus/meta_loop.rs` — an existing periodic-loop scaffold to model the wake on.

### Cut shape
1. New module `crates/zeus-prometheus/src/dreaming.rs`.
2. `DreamingConfig { enabled: bool (default false), light_cron, rem_cron, lookback_days, limits }` — parallels `HeartbeatConfig`. Default OFF (opt-in, like OpenClaw).
3. Two entry points: `run_light_phase()` and `run_rem_phase()`.
   - **Light:** pull recent recall entries within a short lookback, run cheap decay/tidy via `ConsolidationEngine::decay_importance`. No LLM call.
   - **REM:** pull a longer lookback, run `extract_patterns` + `select_for_promotion`, then one LLM call to author a short narrative ("Today I learned…"), persisted as a `Reflection` and into `MEMORY.md` / mnemosyne semantic tier.
4. Register both phases as cron jobs on boot via the existing scheduler (`scheduler.rs`), gated on `enabled`.

### Hook points
- **Cron:** reuse `scheduler.rs` job registration — two managed jobs tagged `dreaming:light` / `dreaming:rem`. Don't invent a new scheduler.
- **Memory:** read from `zeus-mnemosyne` recall; write promoted lessons back to semantic tier + append narrative to `MEMORY.md`.
- **Nous:** call `ConsolidationEngine` and `MetaCognition::record` — no new consolidation logic.

### Acceptance
- With `dreaming.enabled = true`, a REM tick produces a dated narrative entry in `MEMORY.md` and ≥1 promoted lesson when the lookback contains promotable patterns.
- With `enabled = false` (default), zero cron jobs registered, zero behavior change. Off-by-default is load-bearing — no surprise LLM spend.
- Light phase makes no LLM call (assert in test).

### Risk
Cost. REM does an LLM call per tick — keep `rem_cron` infrequent (daily) and lookback bounded. Light is free. Ship OFF.

---

## #131 — Taskflow

### What it does
Named, reusable multi-step workflows. Define a workflow once ("release-cut": build → test → tag → changelog → announce), then replay it by name. We have cron + heartbeat + cooking-loop, but **no named, replayable workflows** — every multi-step job is re-improvised each time.

OpenClaw models this as `TaskFlowRecord` — a flow with an id, goal, ordered steps, a current-step pointer, and persisted state (`runtime-taskflow.types.ts`). A flow is a durable state machine the agent advances across turns/sessions.

### Substrate we already have
- `scheduler_tools.rs` / `trigger_tools.rs` — one-shot + cron scheduling over `scheduler.db`. **Taskflow sits above this**: a flow is an ordered set of steps, each step potentially a scheduled action.
- The agent cooking-loop already executes steps in-turn — taskflow gives it a *named, resumable* plan instead of an ad-hoc one.

### Cut shape
1. New module `crates/zeus-prometheus/src/taskflow.rs` + `flows.db` (SQLite, mirror the `scheduler.db` pattern already in `scheduler_tools.rs`).
2. Records:
   - `FlowDef { id, name, goal, steps: Vec<FlowStep> }` — the reusable template.
   - `FlowRun { id, flow_id, status (pending/running/blocked/done/cancelled), current_step, state_json, created/updated/ended }` — a live instance.
   - `FlowStep { id, description, action }`.
3. Tools (mirror `ScheduleCreateTool`'s shape exactly — same `ToolSchema` pattern):
   - `flow_define` — register a `FlowDef`.
   - `flow_run` — instantiate a `FlowRun` from a def.
   - `flow_advance` — mark current step done, move pointer, return next step into the agent's context.
   - `flow_status` / `flow_list` / `flow_cancel`.
4. On heartbeat tick, surface any `running`/`blocked` flow's current step into the prompt (so the agent resumes without re-improvising).

### Hook points
- **Heartbeat:** `heartbeat.rs` already extracts `CURRENT TASK`; add a parallel "extract active flow step" that injects the next step. One new call beside the existing extractor.
- **Scheduler:** a `FlowStep` action may *be* a `schedule_create` call — taskflow composes the existing scheduler, doesn't replace it.
- **Tools:** register the `flow_*` tools where `schedule_*` tools are registered (same toolbox).

### Acceptance
- Define a 3-step flow, run it, call `flow_advance` 3× → status transitions `pending→running→done`, `current_step` walks the list, final state persisted.
- A `blocked` flow surfaces its step on the next heartbeat tick.
- Survives restart: define+run, kill gateway, reboot, `flow_status` returns the in-progress run.

### Risk
Scope creep into a full workflow engine. **Bound it:** linear steps only (no branching/parallel) in v1. Branching is a follow-up.

---

## #132 — Commitments

### What it does
Inferred follow-ups the agent tracks and honors across sessions. When a conversation implies a promise ("I'll check the build after lunch", "ping me when the deploy's green"), the agent extracts it as a commitment, stores it with a due-window, and the heartbeat delivers it back when due — so promises don't evaporate at session end.

OpenClaw: `CommitmentsConfig { enabled, maxPerDay }` + a commitment store + heartbeat delivery (`infra/heartbeat-runner.ts` — `listDueCommitmentsForSession`, `markCommitmentsAttempted`). Zeus has **no** commitment store today (only the unrelated `standing_orders.rs`).

### Substrate we already have
- `heartbeat.rs` — the delivery vehicle. It already wakes on a tick and reads task state; commitments add "also check for due commitments."
- `standing_orders.rs` — the **structural template** (SQLite store + status enum + boot reload). Commitments is a sibling store, same pattern, different semantics: standing orders are durable directives the *human* sets; commitments are follow-ups the *agent* infers.

### Cut shape
1. New module `crates/zeus-prometheus/src/commitments.rs` + `commitments.db` — clone the `standing_orders.rs` store skeleton (it's the proven pattern in-repo).
2. Record: `Commitment { id, text, channel, due_window {earliest_ms, latest_ms}, status (pending/attempted/done/expired), created_at, source_session }`.
3. `CommitmentsConfig { enabled: bool (default false), max_per_day: u32 (default 3) }` — matches OpenClaw's knobs exactly.
4. **Extraction:** at end-of-session (or on a light heartbeat tick), one LLM pass over the recent transcript → "did I promise anything time-bound?" → insert commitments (capped at `max_per_day`).
5. **Delivery:** on heartbeat tick, `list_due()` → inject due commitments into the prompt → `mark_attempted()`. Dedup by channel/thread so we don't nag.

### Hook points
- **Heartbeat:** add a `deliver_due_commitments()` call in the tick alongside the existing task pickup — gated on `enabled` and on heartbeat having a real target (mirror OpenClaw's `canHeartbeatDeliverCommitments`).
- **Nous:** extraction can reuse `nous/intent` to judge "is this a commitment?" rather than a raw prompt, if cheaper.
- **Store:** copy `standing_orders.rs` SQLite scaffolding verbatim, swap the schema.

### Acceptance
- Seed a transcript containing "I'll check the deploy at 3pm" → extraction inserts one pending commitment with a due-window.
- Heartbeat tick after the due-window injects the commitment text and flips status to `attempted`.
- `max_per_day` cap enforced (insert 5, only 3 land in a rolling day).
- `enabled = false` (default) → no extraction, no delivery, no behavior change.

### Risk
Nagging / false positives. Cap hard (`max_per_day = 3`), dedup per channel, ship OFF. A wrong commitment is worse than a missed one — bias toward precision in the extraction prompt.

---

## #133 — Standing Orders (deepen)

### What it does
Persistent, multi-day directives that survive restarts ("monitor #alerts", "prune MEMORY.md weekly"). **This already exists** — `standing_orders.rs` parses them from `HEARTBEAT.md`, persists to SQLite, and reloads active orders into the heartbeat prompt on boot. #133 is a **deepening**, not a build.

### Substrate we already have (this IS the subsystem)
`crates/zeus-prometheus/src/standing_orders.rs` — already provides:
- `StandingOrder` with `OrderPriority` (P1/P2/P3) + `OrderStatus`.
- `StandingOrderStore` — `add/list/active/complete/remove`.
- `sync_from_heartbeat()` + `parse_standing_orders()` — `HEARTBEAT.md` `## STANDING ORDERS` parsing.
- Boot reload into the heartbeat prompt.

### Cut shape (the gaps to close)
The store is solid; the **lifecycle and accountability** are thin. Deepen these:
1. **Cadence-awareness.** Orders like "every morning" / "weekly" carry an implicit schedule. Parse a cadence hint and register a matching cron via `scheduler.rs` so the order actively *fires* instead of only sitting in the prompt.
2. **Progress tracking.** Add `last_acted_at` + a short `notes` field. An order the agent hasn't touched in N cadence-periods gets flagged ("standing order P1 not actioned in 3 days") on heartbeat — closes the "silently ignored directive" gap.
3. **In-session tools.** `standing_order_add` / `complete` / `list` tools so the agent (or human via chat) manages orders without hand-editing `HEARTBEAT.md`. Today it's parse-only from the file.
4. **Conflict/dedup.** `sync_from_heartbeat` should reconcile, not duplicate, when an order's text is edited — match on stable id/hash.

### Hook points
- **Cron:** cadence-bearing orders register jobs via `scheduler.rs` (same path dreaming/taskflow use).
- **Heartbeat:** the staleness check rides the existing tick.
- **Tools:** register `standing_order_*` beside `schedule_*` / `flow_*`.

### Acceptance
- A "every morning" order registers a daily cron on sync and fires.
- An untouched P1 order surfaces a staleness flag after its cadence window lapses.
- Editing an order's text in `HEARTBEAT.md` updates in place (no duplicate row).
- `standing_order_add` tool inserts a row visible in next-boot reload.

### Risk
Lowest of the four — store is proven. Main risk is cadence-parsing ambiguity ("weekly" → which day?). Default to a sane anchor (Monday 9am) and let the order text override.

---

## Recommended cut order

1. **#133 standing-orders** — smallest, deepens proven code, establishes the `*_tools` + cron-registration pattern the others reuse.
2. **#132 commitments** — clones #133's store skeleton; high user-visible value (promises kept).
3. **#131 taskflow** — new state machine; medium effort, composes the scheduler.
4. **#130 dreaming** — most machinery already in `nous`, but highest cost-risk; ship last, OFF by default.

All four default **OFF** / opt-in. Phase 1's voice contract makes them *feel* autonomous; these make them *capable* — but capability that surprises the operator with cost or nagging is a regression, so every new loop is opt-in until proven.

## Shared guardrails (carry from Phase 1)
- **Persona-latch still wins:** these capabilities never override correctness/safety/permissions.
- **Off-by-default:** no new autonomous LLM spend without an explicit config flag.
- **One scheduler:** dreaming, taskflow cadence, and standing-order cadence all register through `scheduler.rs` — do not fork a second scheduler.
- **Verify-before-claim:** each cut lands with its acceptance tests green before the seat reports done.
