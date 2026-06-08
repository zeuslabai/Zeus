# OpenClaw Autonomy Parity Audit

**Branch:** `feat/openclaw-parity-phase1`  
**Zeus main:** `bbdedfa5`  
**OpenClaw ref:** `9a82b600` (HEAD, 2025-06-08)  
**Auditor:** zeus-titan  
**Scope:** READ-ONLY gap report — no code cuts this pass.

---

## Executive Summary

Zeus has a functional autonomy backbone (heartbeat, standing orders, hooks, scheduler) but lacks several depth features OpenClaw has matured. The biggest gaps are in **commitment extraction/lifecycle**, **scheduler one-shot (`--at`) with auto-delete**, **wake-mode control**, and **delivery-mode configurability**. Most "have" items are surface-level implementations that need deeper verification.

---

## Per-Mechanism Assessment

### 1. Commitments — `zeus-prometheus/src/commitments.rs` vs `openclaw/src/commitments/`

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **File exists** | ❌ `commitments.rs` does not exist in `zeus-prometheus/src/` | ✅ Full subsystem: `types.ts`, `runtime.ts`, `extraction.ts`, `store.ts`, `config.ts` | **MISSING** |
| **Concept** | No first-class commitment concept. Goals in `HEARTBEAT.md` are closest analog. | First-class `CommitmentRecord` with `kind`, `status`, `dueWindow`, `dedupeKey`, `confidence` | **MISSING** |
| **Extraction** | None — agent must manually infer follow-ups from conversation context. | LLM-driven batch extraction from completed turns; hidden extraction prompts; terminal-error cooldown | **MISSING** |
| **Lifecycle** | N/A | `pending` → `sent` → `dismissed`/`snoozed`/`expired`; full state machine | **MISSING** |
| **Persistence** | N/A | Per-agent SQLite/JSONL store with deduplication | **MISSING** |
| **Scheduling** | N/A | `dueWindow` with `earliestMs`/`latestMs` + timezone-aware delivery | **MISSING** |

**Gap depth:** Entire subsystem absent. Zeus standing orders are durable directives, not inferred commitments from conversation.

**Proposed cut priority:** P1 — commitments are a core differentiator for proactive agent behavior.

---

### 2. Task-Flow / Background Tasks

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Heartbeat tasks** | ✅ `HEARTBEAT.md` parsed; tasks executed on interval | ✅ Similar via `heartbeat-runner.ts` + cron jobs | **HAVE** |
| **Task result silencing** | ✅ `silent: true` discards `HEARTBEAT_OK` (S27) | ✅ Configurable delivery modes | **HAVE** |
| **Per-task state/dedup** | ✅ `heartbeat-state.json` with last-run timestamps | ✅ Cron job state machine (`idle`/`queued`/`running`/etc.) | **HAVE** |
| **Background task ledger** | ❌ No centralized ledger of all background work | ✅ `CronJob` store with full history, run logs, telemetry | **MISSING** |
| **Task templates** | ✅ `TaskTemplate` presets (daily_summary, weekly_report, etc.) | ✅ Similar job templates | **HAVE** |
| **Max concurrent jobs** | ✅ `max_concurrent_jobs` in `PrometheusSchedulerConfig` | ✅ Semaphore-based concurrency in runner | **HAVE** |
| **Run telemetry** | ❌ No token usage / model telemetry captured | ✅ `CronRunTelemetry` with input/output/cache tokens | **MISSING** |

**Gap depth:** Zeus has basic task execution but no unified ledger or telemetry. Standing orders are separate from scheduled tasks.

**Proposed cut priority:** P2 — ledger + telemetry are observability wins, not blockers.

---

### 3. Hooks — `zeus-agent/src/hooks.rs` vs `openclaw/src/hooks/`

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Hook system exists** | ✅ `HookRegistry` with `HookEventType` enum | ✅ Full subsystem with `HookEntry`, `HookMetadata`, `HookInvocationPolicy` | **HAVE** |
| **Event types** | 8 types: `OnMessageReceived`, `OnSessionStart`, `OnSessionEnd`, `OnAgentLoopStart`, `OnAgentLoopEnd`, `OnToolExecuted`, `OnError`, `PreToolUse`, `PostToolUse` | Event strings (e.g. `"command:new"`, `"session:start"`) + plugin hook integration | **HAVE** |
| **Shell-based hooks** | ✅ `~/.zeus/hooks/tools/` with exit-code semantics (0=allow, 2=deny, 3=skip) | ❌ Not shell-based; JS/TS handler modules | **PARTIAL** |
| **Plugin hooks** | ❌ No plugin hook integration | ✅ `plugin-hooks.ts` with `workspace.ts` hook orchestration | **MISSING** |
| **Frontmatter config** | ❌ No frontmatter-driven hook config | ✅ `ParsedHookFrontmatter` + `HookInvocationPolicy` | **MISSING** |
| **Tool-pattern matching** | ✅ `pre-shell` wildcard matching | ❌ Not applicable (different architecture) | **HAVE** |
| **Eligibility/context** | ❌ No remote/platform eligibility checks | ✅ `HookEligibilityContext` with `hasBin`, `hasAnyBin`, platform checks | **MISSING** |

**Gap depth:** Zeus hooks are simpler shell-based interceptors. OpenClaw has richer plugin-integrated hooks with metadata, policies, and eligibility.

**Proposed cut priority:** P2 — plugin hook integration would enable extension ecosystem.

---

### 4. Standing Orders — `zeus-prometheus/src/standing_orders.rs`

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Subsystem exists** | ✅ `StandingOrderStore` with SQLite persistence | ❌ No direct equivalent; closest is cron jobs + commitments | **HAVE (unique to Zeus)** |
| **Parsing** | ✅ Parsed from `HEARTBEAT.md` with `[P1]`/`[P2]`/`[P3]` priority | N/A | **HAVE** |
| **Persistence** | ✅ SQLite with `sync_from_heartbeat()` idempotent sync | N/A | **HAVE** |
| **Lifecycle** | ✅ `Active` → `Completed` → `Archived` | N/A | **HAVE** |
| **Heartbeat integration** | ✅ Loaded into heartbeat prompt on boot | N/A | **HAVE** |

**Gap depth:** Zeus actually *leads* here. OpenClaw has no direct standing-order concept. This is a Zeus differentiator.

**Proposed cut priority:** N/A — maintain and extend.

---

### 5. Scheduler — `zeus-prometheus/src/scheduler.rs` vs `openclaw/src/cron/`

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Cron expressions** | ✅ `cron` crate; `Schedule` from string | ✅ `cron` expr + `everyMs` + `at` | **HAVE** |
| **One-shot (`--at`)** | ❌ No `at` schedule kind | ✅ `{ kind: "at", at: string }` with auto-delete after run | **MISSING** |
| **Auto-delete after run** | ❌ Not supported | ✅ One-shot jobs auto-remove from store | **MISSING** |
| **Wake modes** | ❌ No wake-mode control | ✅ `CronWakeMode`: `"next-heartbeat"` / `"now"` | **MISSING** |
| **Session targeting** | ❌ No session targeting | ✅ `CronSessionTarget`: `main` / `isolated` / `current` / `session:${name}` | **MISSING** |
| **Stagger/jitter** | ❌ No stagger support | ✅ `staggerMs` for deterministic phase offset | **MISSING** |
| **Job state machine** | ❌ Basic `ScheduledTask` with `last_run` | ✅ `CronJobState`: `idle`/`queued`/`running`/`completed`/`failed`/`paused` | **MISSING** |
| **Failure alerting** | ❌ No failure notification | ✅ `CronFailureAlert` with delivery status tracking | **MISSING** |
| **Run logs** | ❌ No per-run log storage | ✅ `CronRunLog` with output, telemetry, timestamps | **MISSING** |
| **Delivery modes** | ❌ Fixed to channel message | ✅ `none` / `announce` / `webhook` | **MISSING** |
| **Max concurrent** | ✅ `max_concurrent_jobs` semaphore | ✅ Similar semaphore in runner | **HAVE** |

**Gap depth:** Zeus scheduler is a basic cron wrapper. OpenClaw's cron subsystem is a full job orchestrator with state machines, delivery modes, wake policies, and observability.

**Proposed cut priority:** P1 — wake modes and one-shot are critical for responsive autonomy.

---

### 6. Delivery Modes

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Mode: `none`** | ❌ Not supported | ✅ Silent execution, no chat output | **MISSING** |
| **Mode: `announce`** | ❌ Fixed behavior | ✅ Chat delivery with configurable channel/thread | **MISSING** |
| **Mode: `webhook`** | ❌ Not supported | ✅ HTTP webhook delivery + completion destination | **MISSING** |
| **Best-effort flag** | ❌ Not supported | ✅ `bestEffort` for non-blocking delivery | **MISSING** |
| **Failure destination** | ❌ Not supported | ✅ Separate failure webhook/alert target | **MISSING** |

**Gap depth:** Zeus always delivers to the bound channel. OpenClaw allows flexible routing.

**Proposed cut priority:** P2 — enables richer integrations but not core autonomy.

---

### 7. Heartbeat Runner

| Aspect | Zeus | OpenClaw | Verdict |
|--------|------|----------|---------|
| **Phase computation** | ❌ Fixed interval | ✅ `resolveHeartbeatPhaseMs()` — deterministic per-agent phase via SHA256 | **MISSING** |
| **Active hours / quiet hours** | ✅ Configurable quiet hours (default 23:00–08:00) | ✅ `isActive()` predicate with `seekNextActivePhaseDueMs()` | **HAVE** |
| **Cooldown gating** | ✅ `cooldown_seconds` + `dedup_window_seconds` | ✅ `heartbeat-cooldown.ts` with policy-based cooldown | **HAVE** |
| **Wake handler** | ❌ No external wake injection | ✅ `setHeartbeatWakeHandler()` for programmatic wake | **MISSING** |
| **Config hot-reload** | ❌ Requires restart | ✅ `updateConfig()` on runner | **MISSING** |
| **Abort signal** | ❌ No graceful shutdown hook | ✅ `abortSignal` integration | **MISSING** |
| **Timeout warnings** | ❌ Not supported | ✅ `heartbeat-runner.timeout-warning.test.ts` | **MISSING** |

**Gap depth:** Zeus heartbeat is simpler but functional. OpenClaw has more sophisticated runner lifecycle.

**Proposed cut priority:** P2 — phase computation and wake handler are nice-to-have.

---

## Ranked Proposed Cuts

| Priority | Mechanism | Cut Description | Effort | Impact |
|----------|-----------|-----------------|--------|--------|
| **P1** | Commitments | Add `CommitmentRecord` type, extraction runtime, SQLite store, lifecycle state machine | High | High — core autonomy differentiator |
| **P1** | Scheduler one-shot | Add `at` schedule kind + auto-delete; extend `TaskConfig` | Low | High — enables imperative scheduling |
| **P1** | Wake modes | Add `wake_mode` to scheduled tasks (`now` vs `next-heartbeat`) | Low | High — responsiveness control |
| **P2** | Delivery modes | Add `none`/`announce`/`webhook` delivery to scheduler | Medium | Medium — integration flexibility |
| **P2** | Background task ledger | Add `BackgroundTaskLedger` with run logs, state machine, telemetry | Medium | Medium — observability |
| **P2** | Plugin hooks | Extend `HookRegistry` with plugin-integrated hooks, frontmatter config | Medium | Medium — extension ecosystem |
| **P2** | Heartbeat phase | Deterministic per-agent phase via hash of agent_id + seed | Low | Low — fairness |
| **P3** | Failure alerting | Per-task failure notification with separate failure destination | Medium | Low — operational nicety |
| **P3** | Session targeting | `main`/`isolated`/`named` session targets for scheduled tasks | Medium | Low — multi-session agents |
| **P3** | Config hot-reload | `updateConfig()` on heartbeat runner without restart | Low | Low — convenience |

---

## Have-But-Verify-Depth Items

These exist in Zeus but need deeper audit to confirm parity:

1. **Standing orders** — Zeus leads; verify OpenClaw doesn't have a hidden equivalent in `agents/` or `context-engine/`.
2. **Hook exit-code semantics** — Zeus uses shell exit codes (0/2/3); OpenClaw uses policy objects. Verify Zeus handles all edge cases.
3. **Heartbeat cooldown** — Both have cooldown; verify Zeus cooldown handles terminal errors (OpenClaw has `openTerminalFailureCooldown`).
4. **Task templates** — Both have presets; verify Zeus templates cover the same operational surface.

---

## Notes

- OpenClaw HEAD at time of audit was `9a82b600`, not `310d28f7` as specified. The latter may refer to a pinned submodule or older checkout. Audit based on latest.
- Zeus `commitments.rs` was referenced by Zeus100 but does not exist in `zeus-prometheus/src/` at `bbdedfa5`. This may be a planned file or a misreference.
- OpenClaw is TypeScript/Node; Zeus is Rust. Parity is functional, not structural.
