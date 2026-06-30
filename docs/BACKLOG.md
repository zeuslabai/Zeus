# Zeus Backlog

Issues logged here are confirmed bugs or improvements not yet assigned to an active sprint.

---

## ✅ SHIPPED

### [BUG] OpenAI: Orphaned tool_calls corrupt session history

**Reported:** 2026-04-12
**Severity:** P0 — every subsequent request fails once triggered
**Reporter:** merakizzz
**Resolved:** 2026-04-27 (audit by zeus106)

**Symptom (historical):** OpenAI returned 400 Bad Request when an assistant
message with `tool_calls` was not followed by matching tool result messages.

**Resolution:** Provider-aware orphan sanitizer shipped in the LLM layer:

- `crates/zeus-llm/src/lib.rs:1195` and `:2877+` — bidirectional sanitizer.
  Strips orphaned `tool_calls` for providers that reject synthetic results
  (Moonshot/Kimi, MiniMax); injects synthetic `tool_result` messages for
  others (OpenAI, Anthropic, etc.).
- `crates/zeus-agent/src/intelligence.rs` — `ContextGuard` calls
  `repair_orphaned_tool_calls` upstream as a second line of defense.

**Relevant commits:**
- `e617054e` fix: sanitizer handles orphaned tool RESULTS + tool_calls bidirectionally
- `1b3d787c` fix(llm): strip orphaned tool_calls for MiniMax like Moonshot/Kimi
- `58ec7293` fix: proper Kimi K2.6 tool_call handling — position-aware sanitizer + strip orphans
- `b6e1604a` fix: GPT-5.5 rejects both temperature AND reasoning_effort with tools

---

## OPEN

### [BUG] P1 — install.sh: macOS gateway never gets launchd supervision on fresh deploy (launchd label mismatch) — #233

Fresh macOS installs end up running a **bare, unsupervised** `nohup zeus gateway` process — when it dies (crash / deploy / machine sleep) **nothing restarts it**, so the gateway "crashes too often" / stays down. Root cause is a launchd **label mismatch** in install.sh:
- `install.sh:1077` checks `launchctl list | grep -q 'ai.zeus.gateway'`, but `zeus daemon install` (`src/daemon.rs:26` `SERVICE_LABEL`) installs the plist as **`com.zeus.gateway`**. The grep never matches → the `then` branch that runs **`zeus daemon start`** (the `launchctl load`, `install.sh:1080`) is **skipped** → it drops to the **nohup fallback** (unsupervised).
- Codebase label split: `crates/zeus-setup/src/ops/deploy.rs:392` uses `ai.zeus.gateway`; `src/daemon.rs:26` uses `com.zeus.gateway`; `install.sh:361` (restart path) uses `com.zeus.gateway`. A rename landed in one path, not the others.
- Both `zeus daemon install` calls (`install.sh:792`, `:1076`) use `2>/dev/null`, masking any real error.

**Fix (small, surgical):** (1) unify the launchd label across `daemon.rs` / `deploy.rs` / `install.sh` (canonical = `com.zeus.gateway`, matching the CLI + restart path); (2) run `zeus daemon start` **unconditionally** after a successful `zeus daemon install` (don't gate it on the label grep); (3) drop `2>/dev/null` on the install call so failures surface.

**Evidence:** titan (zeus-titan Mac, the only fully-fresh nuked deploy) had **no plist** in `~/Library/LaunchAgents` + a bare `/usr/local/bin/zeus gateway` process, no auto-restart. Diagnosed by Zeus100 2026-06-12 (merakizzz-directed gateway-crash audit). **Refs:** `scripts/install.sh:361,792,1076-1077,1080`; `src/daemon.rs:26,104-126`; `crates/zeus-setup/src/ops/deploy.rs:392`.

### [REGRESSION] minimax seats (titan / ZM) stall at recon→build — won't execute coding tasks they handled a week ago — #232

minimax (MiniMax-M3) seats were on par with mimo-v2.5-pro ~1 week ago (merakizzz, 2026-06-12) — they executed channel-assigned coding tasks reliably. Now they do solid **recon** (titan's #185 Advanced-tab stub-map was line-accurate, substrate-verified) but **refuse to transition recon→build**: handed explicit numbered build+edit steps for the #185 Voice subview plus "this is an active coding task, don't stand down," titan replied "Understood. Standing down." **twice**. It parks instead of executing. merakizzz: this is a regression on the Zeus side, not an inherent model limit — "that model was working on par with Xiaomi about a week ago."

**Investigate the minimax implementation on the Zeus platform — three angles:**
1. **Prompt-scaffolding over-trigger (lead hypothesis).** The shipped `AGENTS.md` template carries a literal `**Stand down:** When told to stand down — go completely silent. No acknowledgment. Just stop.` plus a "Stay silent when:" block + line 28 "narrating that you're standing down — just stand down." Hypothesis: minimax follows the stand-down / silence directives **rigidly** where mimo/fable apply judgment — conflating "nothing in my HEARTBEAT.md `CURRENT TASK`" or "I finished the recon I was asked for" with "stand down," even mid-assignment. Direct echo of the `76ca3c42` silent-default autonomy-kill (agents idled — conflated stay-quiet with stay-idle; reverted by `3b267295`).
2. **LLM provider / tool-call path** (#175/#177 lineage). minimax has the weak-tool-calling profile (emits inline prose action instead of a `tool_call`). Check whether a recent change to the minimax provider path / tool-call parsing / prompt format in `zeus-llm` or the agent loop regressed its tool-call emission so a build instruction no-ops.
3. **What changed in the last ~7 days.** Diff commits touching agent prompt scaffolding, persona corpus, heartbeat intake, and the `zeus-llm` minimax path between the working window (~2026-06-05) and now; bisect for the regressor.

**Fix direction (TBD, do NOT naive-resurrect silence-kills-autonomy):** likely make the stand-down/silent scaffolding **judgment-gated** rather than a rigid literal, and/or strengthen minimax task-intake so a coordinator @mention assignment registers as `CURRENT TASK`. Recon-class work (audits, stub-mapping) is currently minimax's strength; build-class work it stalls on.

**Refs:** `crates/zeus-tui/src/onboarding/templates/AGENTS.md` (:28, :73-87), `crates/zeus-tui/src/onboarding/mod.rs` (:2614-2623), `scripts/deploy-identity.sh` (:604), `crates/zeus-llm/` (minimax provider) + agent-loop tool-call parsing. Related: #175/#177 (weak tool-calling), `76ca3c42`/`3b267295` (silent-default lineage). Reported by merakizzz 2026-06-12.

### [BUG] TUI (production): global `?` / single-char hotkeys fire while typing in the chat input — #231

In the production TUI chat tab, typing a `?` in the message input pops the `[ ? ] KEYBINDINGS` help overlay instead of inserting a literal `?` — i.e. the global hotkey fires mid-typing for no apparent reason. Same class likely affects other bare single-char global/tab hotkeys (`n`, `m`, `f`, etc.) when the input is focused. Root cause: global single-char keybindings are not suppressed while the chat text-input has focus. **Fix:** when the input is in text-entry mode, route printable chars to the input buffer and only treat non-printable/modified keys as global commands (Tab, Ctrl-K, F10, Esc, Enter, PgUp/PgDn, arrows). Reported by merakizzz 2026-06-12 w/ screenshot (mimo-v2.5-pro seat, v0.1.2 main, chat tab — typed "whats your persona", overlay appeared).

**Refs:** `crates/zeus-tui/src/app.rs` (key/event dispatch + chat input handler). Sibling of the onboarding render-fidelity pass (fix/tui-onboarding-* branches).

> **📋 Closeout 2026-06-09 — the issues below (#173 through #178) all SHIPPED this session; kept here pending a structural move to ✅ SHIPPED. Merged to `main`:**
> - **#173** scheduled agent-task execution — `3d81ac30` (+ phase-1 `db932d10`)
> - **#173-b** `/clear` / `--fresh` goals.db purge — `5bbb45fa`
> - **#173-c** zeus-spark heartbeat — MOOT (verified ticking via journalctl; no code change needed)
> - **#174** OpenClaw scheduler-depth parity — one-shot `run_at` (P1 `88647466`) · wake-modes (P2) · delivery-modes (P3), through `b6d5fad2`
> - **#176** cooking-timeout auto-adjust — dead-code harden + H1 task-derived budget + H2 progress-based bounded extension, through `7ea2a81d`; `cooking_loop_max` ceiling
> - **#177** phantom-action guard — structural claimed-action-without-tool_call detection, weak-provider scoped, single re-prompt — `07e65588`
> - **#178** WebUI-first onboarding → gateway self-launch — full API router in bootstrap + `/v1/gateway/restart` — `6bbd299e`
>
> **Still genuinely open:** web4 **P0-1b** (marketplace→agora consolidation, in flight) · **#66** v0.1.2 publish (deferred).

### [BUG] Scheduled agent-tasks never fire ("post X at 5pm") — #173

**✅ RESOLVED 2026-06-08** — phase-1 `db932d10` (scheduled task runs a tool-capable Agent turn) + phase-2c `3d81ac30` (schema-clarity making `schedule_create task_type="prompt"` discoverable + `ZEUS_SCHEDULER_DB` test-seam + full-loop e2e test). The root-cause below was traced on `031d795d` and went STALE vs current main: the talos `schedule_create` path already wrote `TaskType::LlmPrompt` rows the prometheus executor reads (same `~/.zeus/scheduler.db`, serde `tag="type"` round-trips) and was loaded into the agent toolset via `TalosRegistry::with_defaults()` — the real remaining gap was *discoverability* (the param read as a generic "LLM prompt"), not missing wiring.

**Reported:** 2026-06-08
**Severity:** P1
**Reporter:** merakizzz

**Symptom:** Tasks an agent is asked to schedule (e.g. "post this on X at 5pm") never execute. Heartbeat/scheduler under-tested end-to-end.

**Root cause (traced on `031d795d`):** the scheduler has two disconnected halves. `create_trigger` (the only scheduler tool wired into the daemon agent, `zeus-agent/src/tools.rs:666` → `trigger_tools.rs`) is **shell-only** (`TaskType::Shell`) and injects stdout "before the next turn" — it can't perform an agent action like an X post, and an idle daemon seat has no next turn. talos `schedule_create` (`zeus-talos/src/scheduler_tools.rs`) supports an `llm_prompt` task type but (a) is not wired into the daemon agent toolset (only exposed over MCP) and (b) writes a payload shape `{"type":"llm_prompt",…}` that does not deserialize into prometheus's `TaskType` enum (`scheduler.rs:660`). The prometheus `CronScheduler` executor is fine (`execute_llm_prompt`, 30s loop) — it is just never fed a correct agent-task row.

**Fix:** wire one coherent path — agent tool → `TaskType::LlmPrompt` row in `~/.zeus/scheduler.db` → executor — plus an actual end-to-end test (schedule agent task → fires → agent executes it).

---

### [BUG] `/clear` (+ `--fresh`) does not clean the goals.db queue — #173-b

**Reported:** 2026-06-08
**Severity:** P2
**Reporter:** merakizzz

**Symptom:** `/clear` is expected to clean goals/plans/etc, but a stuck loop's **pending goals.db row** survived both `/clear` and a `--fresh` gateway restart (only marking the row `completed` truly killed the loop). `--fresh` clears context + `~/.zeus/workspace/goals/*.md` + procs, but not the goals.db pending rows.

**Fix:** make `/clear` (and `--fresh`) clear/terminate the goals.db pending queue, not just context/files/procs.

---

### [BUG] zeus-spark heartbeat not ticking post-deploy — #173-c

**Reported:** 2026-06-08
**Severity:** P2
**Reporter:** Zeus100

**Symptom:** zeus-spark keeps firing watchdog "Last tick: never" after the `031d795d` deploy, even though the tick-writer (`spawn_fast_pulse`, 60s, `heartbeat.rs:1100`) is wired and other seats (e.g. zeus106) went quiet post-restart. That seat either did not take the update or its heartbeat loop is not starting.

**Action:** verify zeus-spark is on `031d795d` and its gateway log shows the heartbeat started; if the heartbeat genuinely is not starting on a seat, that overlaps the #173 scheduler/heartbeat reliability work.

---

### [TASK] OpenClaw autonomy/automation parity — #174

**Reported:** 2026-06-08
**Severity:** P2 (autonomy depth; supports web4 + proactive work)
**Reporter:** merakizzz

**Context:** From the `~/openclaw` @ `310d28f7` autonomy study (merakizzz directive). We already HAVE the core mechanisms — hooks (`crates/zeus-agent/src/hooks.rs`), inferred commitments (`crates/zeus-prometheus/src/commitments.rs`), task-flow, standing orders (`crates/zeus-prometheus/src/standing_orders.rs`), heartbeat, cron-in-Gateway + SQLite persistence, dreaming (#130/#143). The genuine gaps are scheduler depth + a parity audit of the rest.

**Scheduler-depth gaps (overlap #173 — build alongside the agentTurn fix):**
- One-shot `--at` scheduled jobs (ISO timestamp or relative like `20m`) with auto-delete-after-run. We have cron/interval but no clean one-shot primitive.
- Wake modes `now` / `next-heartbeat` to tie cron + heartbeat together.
- Delivery modes `none` / `announce` / `webhook` for scheduled-job output routing.

**Taxonomy depth audit (we have these — verify depth vs openclaw, close real gaps):**
- Commitments, task-flow, hooks, standing-orders: confirm parity vs openclaw `docs/automation` depth.
- Background-tasks ledger: openclaw has unified detached-work visibility (`tasks list` / `tasks audit`). We have approvals + agent-spawner but maybe not a single detached-task ledger — verify + add if missing.

**Refs:** `~/openclaw` docs/automation, src/agents/tools/cron-tool.ts, docs/gateway/heartbeat.md, docs/automation/cron-jobs.md.

---

### [TASK] Cooking-loop timeout: auto-adjusting, no rigid hardcoded default — #176

**Reported:** 2026-06-08
**Severity:** P2 (autonomy reliability)
**Reporter:** merakizzz

**Context:** The config-driven NL timeout shipped — `resolve_cooking_loop_timeout()` (zeus-core, wired `src/gateway.rs:1452`) resolves `[prometheus] cooking_loop_timeout` ("2h"/"30 hours", humantime) → `cooking_loop_timeout_secs` → `gateway.timeout_secs` (default 1800); 6 tests. But it's opt-in + defaults to 1800, so titans still hit it (zeus106 on the #173 cut). merakizzz directive: **there should NOT be a rigid hardcoded default — 1800 is a sensible baseline for most tasks, but the timeout should ADJUST AUTOMATICALLY.**

**Work:**
- Wire **task-derived** duration: extract/infer the budget from the task itself ("work on this for 30 hours" → 30h) and/or auto-scale by task scope, instead of only a static config knob.
- Keep 1800 as the baseline when nothing's specified, but it must not be a hard cap — auto-extend/adjust to the task.
- Reconcile both cook paths: `zeus-agent/src/cook.rs:154-228` has a SEPARATE `wall_budget_seconds` (default 1800, cook.rs:165) on the lower-level CookConfig — make both consume the same resolved/auto-adjusting value.
- Interim: set `[prometheus] cooking_loop_timeout` generously in seat configs (config-guard) OR bump the default.

**Aside (cut-hygiene, ZeusWeb owner):** `apps/ZeusWeb/Cargo.lock` is not gitignored → keeps getting swept into unrelated cuts (#155, #173 both hit it). Add it to `.gitignore`.

**Refs:** zeus-prometheus/src/lib.rs `resolve_cooking_loop_timeout`, src/gateway.rs:1452/2398, zeus-agent/src/cook.rs:165.

---

### [BUG] minimax/qwen seats hallucinate tool actions instead of calling tools — #177

**Reported:** 2026-06-08
**Severity:** P2
**Reporter:** merakizzz

**Symptom:** ZM (minimax) asked to generate a `.md` and attach it to a Discord channel **narrated** success ("created + attached the file") but emitted no tool call — no file on iCloud, no real attachment. A hallucinated tool-success.

**Substrate (main):** NOT a wiring or parser bug. `send_file` + `message(attachment)` tools are wired (`zeus-agent/src/tools.rs:588-594`); minimax runs on the **Anthropic Messages API** (`api.minimax.io/anthropic/v1/messages`, `tool_use` blocks) and `minimax.rs` parses `tool_use`→ToolCall correctly. The model simply does not emit the `tool_use` block — it answers in prose. Same class affects qwen (raspizeus).

**Fix direction (per merakizzz — do NOT hardcode intent→tool_choice forcing; native tool-calling is the LLM's job):**
- Primary lever = **capable-model assignment** — run seats on models that tool-call reliably (ZM → GLM/flagship move is merakizzz's lane). minimax/qwen that narrate-instead-of-call get moved, not band-aided.
- Optional, non-hardcoded safety net = a **generic "claimed-action-without-tool_call" guard**: if a turn's text asserts a tool action (created/attached/sent) but emitted zero tool_calls, re-prompt once ("you said you'd X but didn't call the tool — call it"). Intent-agnostic, applies to any weak tool-caller; this is the OpenClaw stale-ack re-prompt generalized (overlaps #173 phase-3 hardening). Implement ONLY this generic guard — no per-intent hardcoding.

**Refs:** crates/zeus-llm/src/minimax.rs (Anthropic endpoint, tool_use parse), zeus-agent/src/tools.rs (send_file/message attach). Related: #141 (minimax fabricated vision).

---

### [TASK] WebUI-first onboarding — launch the gateway from the wizard (deploy → webui, skip TUI) — #178

**Reported:** 2026-06-08
**Severity:** P2 (onboarding UX — enable WebUI as a full TUI-alternative onboarding path)
**Reporter:** merakizzz

**Use case:** Users skip the TUI and onboard entirely from the WebUI on a fresh deploy, with the wizard launching the full gateway when done.

**Substrate (traced on `6b44e107`):** The gateway already has a WebUI-only bootstrap mode (`src/gateway.rs:380-422`): started with no LLM configured, it serves the SPA + a minimal API on `gateway.web_port` (default 8081). But the deploy→webui flow is incomplete in two ways:

1. **`/v1/gateway/restart` has no server handler.** The wizard's final IGNITION step calls `POST /v1/gateway/restart` (`apps/ZeusWeb/src/api/mod.rs:2703`, `pages/onboarding_wizard.rs:3513`), but no route/handler exists anywhere in zeus-api (only the webui client). So onboarding writes config but never transitions out of bootstrap into the full gateway.
2. **The bootstrap router is too minimal.** It wires only 5 routes (`gateway.rs:409-413`: config/test, onboarding/status, onboarding/complete, config PUT, health). The wizard also calls `/v1/models`, `/v1/onboarding/personalities`, `/v1/onboarding/skills`, `/v1/channels/*`, `/v1/auth/*`, `/v1/security/permissions` — all 404 in bootstrap mode. So model-list, personality/skill pickers, channel config, auth, and permissions break on a fresh-deploy webui onboard.

**Fix:**
- Add a `/v1/gateway/restart` handler: persist config → clean process exit → rely on the launchd `KeepAlive` / systemd `Restart=always` supervisor (already in `zeus-setup/src/ops/{deploy,package,service}.rs`) to relaunch `zeus gateway`, which now finds the LLM and boots full. A browser WASM SPA cannot spawn the binary itself — the launch must ride the supervisor.
- Have the bootstrap server mount the FULL API router (backed by a no-LLM dummy `AppState`) instead of the hand-picked 5-route subset, so every wizard endpoint works un-onboarded.
- Ensure deployment installs both the service (KeepAlive) and the WebUI dist to `~/.zeus/web/` so a fresh deploy auto-starts bootstrap mode with the wizard reachable (`install.sh` redeploys dist; the bootstrap server requires `~/.zeus/web/index.html` or it bails).

**Refs:** `src/gateway.rs:380-422`, `crates/zeus-api/src/routes.rs:624`, `apps/ZeusWeb/src/pages/onboarding_wizard.rs`, `crates/zeus-setup/src/ops/{deploy,package,service}.rs`. Coord memory: `reference_webui_first_onboarding_gw_launch_gap`.
