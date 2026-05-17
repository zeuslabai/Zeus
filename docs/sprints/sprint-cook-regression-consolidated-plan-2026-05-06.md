# Sprint Plan — Cook Regression Resolution

**Sprint lead:** Zeus100
**Date:** 2026-05-06
**Status:** Active — three lanes in flight

---

## TL;DR

Three architectural islands (Telegram, Discord, TUI) don't share conversation context. A 24-hr regression on Telegram (`caea66a7` Patch A) made the symptom acute, but the root cause is longstanding: each surface builds its own conversation history independently, with no shared `ConversationStore` keyed on fleet-level identity.

**Three lanes ship in production-quality discipline (no rush patches):**

1. **Lane 1 — Telegram-specific Patch A revert + fleet allowlist.** Halt-fix for the 24-hr regression.
2. **Lane 2 — Channel-ingestion tracing breadcrumbs.** Production-grade falsifier for cross-surface diagnosis.
3. **Lane 3 — Extract `prepare_cook_context` + session aliasing resolver.** The structural unification fix.

---

## Findings convergence (2026-05-06 audit)

Two parallel Q1 separability investigations converged on the same architectural shape via different angles:

- **Zeus112 read** (in-channel synthesis): `process_autonomous` is mentally factored already (numbered comments 1-9). TUI is SSE/API client. Gateway has 4 direct `cook_with_history` callsites at lines 1113, 2014, 2270, 2709.
- **zeus106 read** (`9ae4739f`): `zeus-tui` has zero dep on `zeus-prometheus`. Only `zeus-api` depends on prometheus. TUI cook entry is HTTP-mediated through `zeus-api` → `gateway.rs` → `prom_guard.cook_with_history(...)`.

**Same finding at two layers.** Gateway.rs IS the `zeus-api` surface zeus106 flagged. Three-surface asymmetry inside `zeus-api`, not a missing crate dep.

**Authoritative spec:** `docs/sprints/backlog-prepare-cook-context-extraction-2026-05-06.md` (Z112's PRD `bc01ce23`, revision in flight).

---

## Lane 1 — Patch A handling (Telegram-specific)

**Status:** SHIPPED on main as `7ba2719a` (pure revert) — held from deploy per merakizzz's "no rush patches" directive.

**Production-quality follow-up:** **fleet allowlist** — `[channels.telegram] fleet_bot_ids = [...]` in config; bot IDs in list bypass Layer 2 mention filter. Allows fleet titans to reply-chain coordinate while still blocking external bot loops.

**Owner:** Zeus100. Branch `feat/telegram-fleet-bot-allowlist` off `7ba2719a`.

**Acceptance:**
- `[channels.telegram] fleet_bot_ids: Vec<String>` in `TelegramRelayConfigCore` (zeus-core) + `TelegramRelayConfig` (zeus-channels)
- `Layer 2` bypass: `if msg["from"]["id"] in fleet_bot_ids → skip mention filter, allow message`
- Unit tests: bot in list passes, unknown bot still filtered, DM unaffected, structured @-mention unaffected
- `cargo check -p zeus-channels` clean
- Cross-check by Zeus112 before merge

**Out of scope:** Discord allowlist (Discord doesn't have the Patch A regression class; Lane 3 closes the deeper context-island issue for Discord).

---

## Lane 2 — Channel-ingestion tracing (production-grade)

**Status:** First breadcrumb shipped as `4939a5cc` on `feat/channel-ingestion-tracing` (cook_dispatched falsifier with `history_len`). Production-grade reshape pending.

**Production-quality scope:**
- `tracing::span!` with `session_id` threaded through cook signatures
- Structured `gate` enum (typed, not stringly-typed)
- `channel_kind: Discord | Telegram | TUI` tag at every gate
- 6 gates: `received → relay_passed → classified → decided → spawner_checked → execute_dispatched`
- Doc-comments in crate explaining gate model
- Integration test confirming falsifier emits expected line shape per surface

**Owner:** zeus106. Continues on `feat/channel-ingestion-tracing` branch after Lane 3 design lands.

**Acceptance:**
- Same prompt from TUI + Telegram + Discord produces three traces with same `(agent_id, human_id, topic_window)` resolving to the same `fleet_session_id` post-Lane-3
- `cargo check + test -p zeus-prometheus + zeus-channels` clean

**Sequencing:** Lane 2 polishes after Lane 3 implementation begins, since the gate model in Lane 3 PRD §3 should align with the tracing gates.

---

## Lane 3 — Extract `prepare_cook_context` + session aliasing

**Status:** PRD `bc01ce23` shipped, revision in flight by Zeus112. zeus106 Q1 findings `9ae4739f` shipped. Implementation unblocked from a design standpoint.

**PRD revision (Zeus112, in flight):**
- §3.1 rename: `channel_ingress_filter` → `prepare_decision_context` + `dispatch_decision` (the actual extractable shape from `process_autonomous` — channel filters live upstream in `telegram_relay.rs` already)
- §3.3 update: gateway.rs as TUI unification site (4 callsites cited)
- §3.2 reshape: resolver-inside-session-read-site as the implementation pattern
- §7 merge: zeus106's Q1a/Q1b/Q1c into open questions, Q1c flagged as load-bearing decision

**Two coherent shapes (Zeus112's framing):**

- **Shape A — minimal cut.** Resolver lives inside `process_autonomous`'s `self.sessions.read()` site. Telegram + Discord unify. TUI deferred.
- **Shape B — full extract** (recommended). Resolver as `&self` method on `Prometheus`, called from both `process_autonomous` AND `gateway.rs` (4 callsites). All 3 surfaces unified in one PR.

**Decision LOCKED: Shape B** per Z112 + zeus106 + Zeus100 convergence + merakizzz directive. zeus106's 4-point argument:
1. Shape A's "TUI deferred" becomes never (regression we ship on purpose)
2. Marginal cost over A is small — 1 `&self` method + 4 gateway callsite edits, all mechanical, all in localized files
3. Lane 2 falsifier (`4939a5cc`) is built for Shape B verification — under Shape A, TUI shows permanent `history_len: 0` flagging permanent regression
4. Shape A's "follow-up sprint targeting gateway.rs" creates worse ownership outcomes (refactor-to-Shape-B-later OR duplicate-resolvers)

**Implementation lane split (zeus106's proposal, accepted):**
- **Lane 3a — Zeus112:** PRD §3.1 signature lock — `&self` resolver method on `Prometheus`
- **Lane 3b — zeus106:** Resolver method body + wire `process_autonomous` read-site to call it
- **Lane 3c — zeus106:** Wire 4 `gateway.rs` callsites (lines 1113, 2014, 2270, 2709) to invoke resolver before `cook_with_history`
- **Lane 3d — Zeus112:** Test matrix per PRD §5 — Shape B's all-three-surfaces validation
- **Lane 3e — Zeus112:** Feature-flag rollout per PRD §6 (5 phases)
- **Cross-check + merge gate — Zeus100**

**Acceptance per Z112's PRD §4 (12 ACs across functional / empirical / non-functional):**
- Same prompt from TUI + Telegram + Discord resolves to the same `fleet_session_id`
- 24hr rolling topic window correctly groups conversations
- No schema migration on critical path
- Feature-flag rollout per §6 (5 phases)

---

## Sequencing

1. **Now:** Zeus100 ships Lane 1 fleet allowlist (Telegram halt unblock at production quality)
2. **Now:** Zeus112 ships PRD revision (§3.1 rename + §3.3 gateway-as-unification + §3.2 resolver shape + §7 Q1a/b/c merge)
3. **Concurrent with #2:** zeus106 prep work for Lane 3 implementation (read gateway.rs callsites, sketch wiring shape)
4. **After #2 lands:** Zeus112 + zeus106 implement Lane 3 (signatures + body extraction + gateway wiring)
5. **After Lane 3 implementation begins:** zeus106 polishes Lane 2 production-grade tracing
6. **End-to-end validation:** Lane 2 tracing confirms `fleet_session_id` consistency across surfaces (per AC §4)

**Open product calls (merakizzz):**
- PRD §7 Q1b: TUI intent (current TUI cook-entry is intentional architectural choice OR unfinished migration?)
- PRD §7 Q3: multi-agent thread merging policy
- PRD §7 Q4: onboarding interaction with session aliasing

These don't block Lane 1 or Lane 2; they shape Lane 3 §6 rollout phases.

---

## Discipline rules locked this sprint

1. **Verify-before-claim** — `git log origin/` confirmed before any SHA claim (caught 6+ times today)
2. **Pre-cut #2 gate (b)** — confirm extracted target is callable from both surfaces before cutting
3. **Pre-cut #3 verify-the-model** — re-grep before assuming spec contracts hold
4. **No half-answers** — if the budget is exhausted, hold and report; don't half-ship
5. **No SHA fabrication** — claim only sticks if production starts inside the same iteration
6. **Use `<@id>` Discord mentions** — name-only doesn't ping
7. **Discord-in → Discord-out, terminal-in → terminal-out** — surface affinity for replies
8. **Production quality only, no rush patches** — merakizzz's directive 2026-05-06

---

## Active branches

- `main` — `7ba2719a` Patch A revert (held from deploy)
- `feat/telegram-fleet-bot-allowlist` — Lane 1 production fix (Zeus100, in flight)
- `feat/channel-ingestion-tracing` — Lane 2 (zeus106; `4939a5cc` first breadcrumb + `9ae4739f` Q1 findings)
- `docs/prepare-cook-context-extraction-prd-2026-05-06` — Lane 3 PRD (Zeus112; `bc01ce23` initial + revision in flight)

---

## References

- Zeus112's Lane 3 PRD: `docs/sprints/backlog-prepare-cook-context-extraction-2026-05-06.md` (`bc01ce23`)
- zeus106's Q1 findings: `docs/sprints/q1-separability-findings-2026-05-06.md` (`9ae4739f`)
- Patch A revert commit: `7ba2719a` on main
- Original Patch A (regression): `caea66a7` on main (reverted)
- Cook regression audit thread: Discord channel `1488620262676238426`, 2026-05-06 14:00-16:00 UTC+4
