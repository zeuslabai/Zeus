# Backlog — Extract `prepare_cook_context` + `channel_ingress_filter` from `process_autonomous`

**Status:** Draft (PRD)
**Author:** Z112 (Herald)
**Date:** 2026-05-06
**Sprint target:** Next available sprint slot
**Priority:** P0 — unblocks fleet cross-surface context continuity

---

## 1. Motivation

### 1.1 Symptom (observed by merakizzz, 2026-05-06)

> "ZeusMarketing performed its task in one shot after asking on TUI. Channels stall. Channels and TUI should have the same context."

A conversation with an agent on TUI does not carry over to the same agent on Telegram, Discord, or Slack. Each surface is an island.

### 1.2 Root cause (audit, 2026-05-06)

Two distinct issues compound to produce the symptom:

**Issue A — `caea66a7` Patch A** (Telegram-only regression)
Drops bot→bot reply-chain implicit-mention in default `Mentions` mode. Fleet titans replying to each other on Telegram are silently filtered at Layer 2 of `telegram_relay.rs` (lines ~1169-1240). **Out of scope for this PRD** — handled in Lane 1 (fleet-allowlist production fix, owner: Zeus100).

**Issue B — Channel/TUI ingress asymmetry** (architectural, this PRD)
`crates/zeus-prometheus/src/lib.rs:410` `Prometheus::process_autonomous` is the channel-ingress entry. It runs a 5-stage pipeline:

1. Intent classification (`intent_classifier.classify`)
2. Decision context (session count, health, recent errors)
3. Decision branch (with optional `SpawnAgents` upgrade)
4. Identity/persona resolution
5. Cook dispatch via `cook_with_history*`

TUI bypasses `process_autonomous` entirely and calls `cook_with_history*` directly. This was correct by design — TUI is direct human dialogue and should not be subject to channel concerns (bot-loop filters, mention detection, fleet routing).

**The bug:** post-classification context construction (session resolution, history load, parent-session resume) is **co-located inside `process_autonomous`** alongside the channel-specific gates. TUI correctly skips the gates but **incorrectly skips context construction too** as a side effect.

`Prometheus::cook_with_history*` (`lib.rs:1378-1405`) is history-stateless — it consumes whatever `&[Message]` slice the caller supplies. Each channel handler (Telegram, Slack, Discord) builds its own slice from its own backend. There is no shared `ConversationStore` keyed on fleet-level identity. Even within channels, two threads about the same `(agent, human, topic)` map to separate sessions.

### 1.3 Why production-quality, not a quick patch

merakizzz directive (2026-05-06): _"No quick rush patches. Only proper production quality solutions."_

The "session aliasing FK on session row" hybrid (Option 3 from prior synthesis) was considered and rejected as a tech-debt shortcut. The clean cut is to extract context construction from the channel-ingress pipeline so both surfaces share it natively.

---

## 2. Goals & Non-Goals

### 2.1 Goals

1. TUI and channels (Discord, Telegram, Slack, Matrix, Email, Signal, WhatsApp, iMessage) share conversation context for the same `(agent_id, human_id, topic_window)` triple
2. Channel-specific gating (intent classify, decision branching, bot-loop filters, mention detection) remains channel-only — TUI is not subject to it
3. Single source of truth for cook context, no per-channel duplication
4. Fully reversible: feature-flagged rollout, no schema migration on the critical path
5. zeus106's `cook_dispatched` tracing branch validates the fix empirically

### 2.2 Non-Goals

- Replacing per-channel session storage entirely (that's the follow-on "Unified ConversationStore" sprint, Option 2 in prior synthesis)
- Touching the `caea66a7` Patch A regression (Lane 1, separate)
- Onboarding flow changes
- Schema migration for existing session rows

---

## 3. Target shape

### 3.1 Extracted methods — revised post-Q1 (Z112 forensics + zeus106 `9ae4739f`)

**Rename rationale.** The original `channel_ingress_filter` was a misnomer. Channel filters (bot-loop, mention, rate-limit) live **upstream** in the relays (`telegram_relay.rs`, `discord_relay.rs`) — they are already separate from `process_autonomous`. What is actually extractable from `process_autonomous` (`lib.rs:410-834`, 425 lines) is the **decision-context preparation** and **decision-dispatch** stages, which `process_autonomous`'s own numbered comments already mentally factor ("1. Classify intent" → "9. Record feedback").

**Shape lock: `&self` methods on `Prometheus`** (Pre-cut #2 gate (b) ✅ — callable from existing call sites without struct-boundary crossing; recursion in `Decision::*` branches preserved).

```rust
// crates/zeus-prometheus/src/lib.rs

/// Fleet-level context resolver. Sole load-bearing addition.
/// Authoritative signature locked in Lane 3a (this commit). See §3.2 for placement (Shape B).
///
/// **Lock semantics:** caller MUST hold the existing `RwLockReadGuard<'_, SessionManager>`
/// from `self.sessions.read().await` and pass it borrowed. Resolver MUST NOT acquire
/// `self.sessions.write()` (deadlocks against the existing read at `lib.rs:428`) and
/// MUST NOT re-acquire `self.sessions.read()` (single-writer starvation risk under contention).
///
/// **Error shape:** infallible. Returns `FleetSessionAlias::unaliased(<original_session_id>)`
/// when no fleet alias resolves for the key, localizing failure to the resolver and keeping
/// 5 callsites (1 in `process_autonomous`, 4 in `gateway.rs`) at zero error-handling diff.
///
/// **Async-ness:** `async fn` — v1 body may be pure in-mem `&sessions` lookup, but Lane 3b
/// folds in Mnemosyne lookup which awaits. Locking `async` here avoids breaking sig change in 3b.
pub async fn session_resolver(
    &self,
    sessions: &RwLockReadGuard<'_, session::SessionManager>,
    agent_id: &str,
    human_id: Option<&str>,
    channel_kind: ChannelKind,
    now: DateTime<Utc>,
) -> FleetSessionAlias;

// `human_id: Option<&str>` rationale (banked at amend commit (d), 2026-05-06):
// Lane 3c callsite recon (zeus106, pre-cut #3) surfaced that of the 5 callsites,
// 3 are human-initiated (gateway.rs:1113, gateway.rs:2014, lib.rs:428 — `msg.source.user_id`
// in scope) and 2 are autonomous self-dispatches (gateway.rs:2270 continuation cook
// "Keep going", gateway.rs:2709 goal-completion follow-up — no `msg.source`,
// no human in scope). The original `human_id: &str` shape forced autonomous callsites
// into a fictional human identifier, which would have polluted the alias-cache key space.
// `Option<&str>` is honest about modality: `Some(user_id)` for human-initiated cooks,
// `None` for autonomous continuations. Resolver body in Lane 3b branches on `Some`/`None`
// for cache-key shape (human-keyed for inbox, agent-only for continuations); the `None`
// path falls back to `FleetSessionAlias::unaliased(agent_id)` as conservative default,
// keeping autonomous-cook parent-session-inheritance design as a Lane 3b body decision
// (additive, non-blocking).

/// Newtype wrapping the resolved alias. Prevents `String` confusion at call sites and
/// allows additive metadata fields (e.g. `merge_decision: Merged | Fresh | Aliased`)
/// to land in Lane 3b without breaking Lane 3c callsites.
pub struct FleetSessionAlias(String);

impl FleetSessionAlias {
    /// Fallback constructor — resolver returns this when no fleet alias resolves.
    /// Yields `unaliased:<original_session_id>` so downstream `Display`/`AsRef<str>`
    /// consumers see a stable, debuggable string with no behavioral change vs today.
    pub fn unaliased(original: &str) -> Self;
    pub fn as_str(&self) -> &str;
}

/// (Optional, low priority) Decision-context prep — extracted from
/// `process_autonomous` stages 1-3 if it cleans up the call site.
/// Can be deferred to a follow-on cut without blocking the resolver work.
//    pub async fn prepare_decision_context(
//        &self,
//        message: &str,
//        tools: &[ToolSchema],
//    ) -> Result<DecisionContext>;

/// (Optional, low priority) Decision dispatcher — extracted from
/// `process_autonomous` stages 4-9. Same deferral note as above.
//    pub async fn dispatch_decision(
//        &self,
//        ctx: DecisionContext,
//        decision: Decision,
//    ) -> Result<AutonomousResult>;
```

> **Signature lock owner: Lane 3a (Z112) — LOCKED in this commit.** Concrete signature replaces the prior TBD sketch. Constraints pinned from zeus106's read-site recon `2f3eade9` (`docs/sprints/lane-3a-read-site-recon-2026-05-06.md`):
>
> 1. **Async-ness → `async fn`** — locks Mnemosyne fold-in for Lane 3b without breaking sig change. Zero cost when v1 body is pure in-mem.
> 2. **Error shape → infallible `-> FleetSessionAlias`** with `unaliased:<original>` fallback. Localizes failure to resolver, keeps all 5 callsites at zero error-handling diff.
> 3. **Inputs → borrow existing `RwLockReadGuard<'_, session::SessionManager>`** from caller. Deadlock-avoidant per recon §lock-graph (caller already holds the guard at `lib.rs:428`; resolver re-acquiring would either deadlock or starve writers). `(agent_id, human_id, channel_kind, now)` per PRD §3.2 resolver-semantics block; `now: DateTime<Utc>` provides the deterministic-test seam for the 24hr-rolling window rule.
> 4. **Output → `FleetSessionAlias` newtype** (not bare `String`). Prevents `String` confusion at 5 call sites; additive metadata fields land in Lane 3b without breaking Lane 3c callsites.
> 5. **Body → `unimplemented!()` in this commit.** Lane 3b fills the body; v1 returns `FleetSessionAlias::unaliased(<original_session_id>)` for all keys (no behavioral change). Lane 3c then prepends the resolver call to the 4 `gateway.rs` callsites at lines 1113, 2014, 2270, 2709.

> **Lane 3a commit (e) — v0 stub body lifecycle (post-`79c47738` amendment):**
>
> The original commit (c) shipped `unimplemented!()` as the body, predicated on Lane 3b body fill landing before any consumer wire-up. Lane 3c merge-safety analysis (zeus106 pre-cut #4) surfaced that this cascade order would panic every cook on main between Lane 3c merge and Lane 3b body merge — a fleet-down regression worse than the cook gap we are closing.
>
> Commit (e) replaces `unimplemented!()` with a v0 stub: `FleetSessionAlias::unaliased(agent_id)`. This extends commit (d)'s already-banked conservative `None`-arm default (`None → unaliased(agent_id)`) uniformly across the entire input domain. Semantically: "treat all inputs as if they were the `None` arm until Lane 3b enriches the `Some` arm." A panicking signature is not a shipped contract; the stub completes Lane 3a's contract.
>
> Lane boundaries preserved: Lane 3a owns the signature **and its safe v0 stub body**; Lane 3b replaces the stub with real cross-channel correlation lookup; Lane 3c wires consumers against the locked signature. Three lanes, three responsibilities.
>
> Operational impact during the Lane 3c → Lane 3b window: tracing spans emit `unaliased(<agent>)` for all flows; Lane 2 traces show no cross-channel correlation events. Time-boxed degradation, monitorable, auto-resolves when Lane 3b body merges. No further consumer-side changes needed when Lane 3b lands — the enrichment is purely producer-side.
>
> See Discipline Entry 17 (cascade plans with `unimplemented!()` placeholders must verify every intermediate merge state on main is hot-runnable) for the generalizable lesson.

### 3.2 `fleet_session_alias` resolver — placement decision (Shape A vs Shape B)

**Two coherent shapes surfaced by zeus106's `9ae4739f` finding** (resolver fits inside `process_autonomous`'s existing session-read site with no signature changes):

| | **Shape A — minimal cut** | **Shape B — production-quality (chosen)** |
|---|---|---|
| Resolver placement | Inline inside `process_autonomous` session-read site only | `&self` method on `Prometheus`, called from `process_autonomous` session-read site **and** `gateway.rs` |
| Surfaces unified in this PR | Telegram + Discord | Telegram + Discord + TUI |
| TUI parity | Deferred to follow-on sprint | Same PR |
| `process_autonomous` signature change | None | None |
| `gateway.rs` change | None | Thin wrapper at 4 callsites (see §3.3) |
| Estimated cut size | ~1 day | ~1-2 days |
| Risk | TUI parity perpetually deferred | Larger surface area, gated by Lane 2 traces |

**Decision (Zeus100 sprint plan `f9cabf52`): Shape B.** "One fix, three surfaces" beats "ship two, defer one" under merakizzz's "no rush patches, production quality only" directive. zeus106's finding sharpens — not contradicts — Shape B: the resolver doesn't need to be a free-standing `pub fn`, it's a `&self` method invoked from inside the existing session-read site (no signature changes there) AND from a thin `gateway.rs` wrapper.

**Resolver semantics (unchanged from v1):**

```
key = (agent_id, human_id, topic_window=24hr_rolling)
→ fleet_session_alias (UUID, stable per key)
→ merged_history (from mnemosyne, all channels under this alias)
```

**Topic window rule:** 24hr rolling from most recent message under the key. Loose `(agent_id, human_id)` merges unrelated threads weeks apart; tight per-channel-per-thread reproduces today's islands. 24hr rolling is the sweet spot — reasonable continuity, automatic topic boundaries.

**Storage:** sit on top of mnemosyne. No new table — `fleet_session_alias` is a derived/cached view keyed on the triple, with TTL-based eviction.

### 3.3 Call paths — revised post-Q1

**Material correction from v1.** TUI does **not** hold `Prometheus` directly. TUI is an SSE/API client (`zeus-tui/src/main.rs:727` consumes `api::SseEvent::Iter`). The brain lives behind `gateway.rs`, which holds `prom_guard` and calls `cook_with_history` directly at four sites: **lines 1113, 2014, 2270, 2709**. Channels go through `process_autonomous`; TUI goes through gateway → `cook_with_history`. The unification site for TUI is therefore **`gateway.rs`**, not a parallel in-process call from TUI.

```
Channel path (Discord, Telegram, Slack, …):
  ChannelMessage
    → relay channel-ingress filters (existing, upstream of process_autonomous)
    → process_autonomous
        └─ session-read site invokes self.session_resolver(agent, human, topic)
        → cook_with_history(history = ctx.merged_history)

TUI path (via gateway, NOT direct):
  TUI input → SSE/API request
    → gateway.rs (one of 4 callsites: 1113 / 2014 / 2270 / 2709)
        └─ thin wrapper invokes prom_guard.session_resolver(agent, human, topic)
        → cook_with_history(history = ctx.merged_history)
```

**Lane 3 implementation surface follows directly:**
- **Lane 3b** (zeus106) — resolver body + `process_autonomous` session-read site wire
- **Lane 3c** (zeus106) — `gateway.rs` wrapper at all four `cook_with_history` callsites (1113, 2014, 2270, 2709)

---

## 4. Acceptance criteria

### 4.1 Functional

| ID  | Criterion |
|-----|-----------|
| AC1 | TUI conversation with `agent=X`, `human=Y` on topic `Z` is visible to the same `(X, Y, Z)` continued on Telegram within 24hr |
| AC2 | Same for Telegram → Discord, Discord → TUI, all permutations of supported channels |
| AC3 | Channel ingress filter still rejects: bot-loops in `Mentions` mode (post-Lane-1 fix), low-confidence intents below threshold, oversize messages |
| AC4 | TUI input is **never** subject to `channel_ingress_filter` |
| AC5 | Topic window expiry (>24hr since last message under key) starts a new `fleet_session_alias` |

### 4.2 Empirical (zeus106's tracing branch validates)

| ID  | Criterion |
|-----|-----------|
| AC6 | `cook_dispatched` trace shows **same `fleet_session_id`** for same-`(agent, human, topic)` across TUI + Telegram + Discord within 24hr |
| AC7 | `cook_dispatched` trace shows **different `fleet_session_id`** for different-`topic` or >24hr-stale conversations |
| AC8 | `relay_passed → classified` gap is zero for TUI path; non-zero only for channel-filter rejections |

### 4.3 Non-functional

| ID  | Criterion |
|-----|-----------|
| AC9  | `prepare_cook_context` p99 latency < 50ms (mnemosyne lookup + merge) |
| AC10 | No regression in `process_autonomous` p99 (channel path total time within 5% of pre-cut baseline) |
| AC11 | Feature flag `pantheon.unified_cook_context` default `false`; flip per-tenant via config |
| AC12 | Rollback = flip flag to `false`, both paths revert to current behavior with zero data loss |

---

## 5. Test matrix

| Surface  | Filter? | Context source       | Expected |
|----------|---------|----------------------|----------|
| TUI      | No      | `prepare_cook_context` | Passes, merged history available |
| Telegram | Yes     | `prepare_cook_context` | Passes (post Lane 1), merged history |
| Discord  | Yes     | `prepare_cook_context` | Passes, merged history |
| Slack    | Yes     | `prepare_cook_context` | Passes, merged history |
| Matrix   | Yes     | `prepare_cook_context` | Passes, merged history |
| Bot-loop on Telegram | Yes (rejects) | n/a | Filter drops, never reaches `prepare_cook_context` |
| New topic (>24hr stale) | varies | New alias | Fresh history slice |
| Same topic, same human, agent A on TUI then agent B on Discord | varies | Different alias (agent differs) | Histories isolated per agent |

Integration tests cover all 8 rows. Unit tests cover the alias resolver edge cases (24hr boundary, missing rows, concurrent writes).

---

## 6. Migration & rollout

### 6.1 No-migration design

`fleet_session_alias` is a derived view over existing mnemosyne records. Existing per-channel sessions remain untouched. The resolver computes the alias on each call from the `(agent, human, topic)` triple; cache is in-memory with TTL.

### 6.2 Rollout plan

1. **Phase 0** — land PR with feature flag default `false`. Both code paths exist; behavior identical to today.
2. **Phase 1** — enable flag for one internal tenant (Zeus dogfood). Validate via zeus106's tracing branch. Monitor AC6-8.
3. **Phase 2** — enable for fleet-wide tenants in batches. 48hr soak between batches.
4. **Phase 3** — flip default to `true`. Old code path remains as dead code for 1 sprint.
5. **Phase 4** — remove dead code path, close the loop.

### 6.3 Rollback

Flip flag to `false`. No data loss — `fleet_session_alias` cache is derived. Per-channel sessions untouched throughout.

---

## 7. Open questions

### 7.1 Resolved (Q1 separability)

1. **~~Is `prepare_cook_context` extractable as `pub fn` with no circular dependencies?~~** ✅ **Resolved.** Z112 forensics + zeus106's `9ae4739f` finding converge:
   - No circular deps. All dependencies of `process_autonomous` (`intent_classifier`, `sessions`, `monitor`, `feedback`, `dynamic_orchestrator`) are `Prometheus` fields. Extracted methods live as `&self` methods, not free functions.
   - TUI does NOT hold `Prometheus` directly. TUI is SSE/API client; `gateway.rs` is the brain-side caller (4 callsites: 1113, 2014, 2270, 2709). See §3.3 for revised call paths.
   - `channel_ingress_filter` was a misnomer — channel filters live upstream in relays. What's extractable from `process_autonomous` is `prepare_decision_context` + `dispatch_decision` (low priority, optional). See §3.1 for revised shape.
   - Resolver placement: zeus106's bonus finding — fits inside `process_autonomous` session-read site with **no signature changes**. Combined with Shape B for TUI parity, this becomes a `&self` method on `Prometheus` invoked from both the session-read site and a thin `gateway.rs` wrapper. See §3.2 for the Shape A vs Shape B trade-off and decision.

### 7.2 Open — design (resolved within Lane 3, no merakizzz dependency)

2. **Topic detection for `topic_window` keying.** Three options:
   - (a) Heuristic: hash of first N tokens of the conversation
   - (b) Explicit: caller passes `topic_id`
   - (c) Implicit: last message timestamp + agent/human pair, no topic differentiation within window
   Lean: (c) for v1 (simplest, matches "same human + same agent + recent" intuition); revisit if cross-topic bleed becomes a problem. **Lane 3a locks this in the signature.**

3. **(zeus106 Q1a) Convergence empirically validated?** Are there any silent code paths where `process_autonomous` and `gateway.rs` construct conversation history with semantically *different* shapes (e.g. system prompt order, tool-result framing) such that a unified resolver would unify the IDs but still feed the model divergent histories? **Lane 2 tracing answers this** — `cook_dispatched` traces should show identical `history_hash` for same-`fleet_session_id` regardless of entry surface. PR review gate.

4. **(zeus106 Q1b) TUI intent.** Does TUI's gateway-mediated path expect *any* of the channel-side intent classification / decision routing, or is it intentionally a "raw cook" surface? If the former, Shape B's gateway wrapper needs to call more than just the resolver. Pre-cut #2 gate (b) for Lane 3c. **zeus106's read-site recon (Lane 3b prep) flags this before Lane 3a signature lock.**

5. **(zeus106 Q1c) Resolver placement** — see §3.2. Locked: Shape B per Zeus100 sprint plan `f9cabf52`.

### 7.3 Open — product (merakizzz calls, gate Phase 3+ rollout, do not block Lane 1 or Lane 3 implementation)

6. **Multi-agent threads.** If human Y messages agents X and Z in the same Telegram thread, do they share context? Current design says no (alias keyed on `agent_id`). Confirm with merakizzz this is desired behavior.

7. **Onboarding interaction.** Does the onboarding flow create sessions that should be merged into the fleet alias on completion? Likely no — onboarding is bootstrap, not conversation. Confirm.

---

## 8. Out of scope (follow-on sprints)

- **Unified `ConversationStore`** (Option 2 from synthesis): single canonical store, channels become thin adapters. ~1 sprint, schema migration. This PRD's design is forward-compatible — `prepare_cook_context` becomes the read API, channels write through canonical adapters.
- **Cross-agent context sharing** (delegation, handoff). Different conversation, deferred.
- **Channel-handler refactor** (canonicalizing `ChannelMessage` construction). Independent of this PRD.

---

## 9. References

**Code (file:line):**
- `crates/zeus-prometheus/src/lib.rs:410-834` — `process_autonomous` (current channel-ingress entry, 425 lines, mapped end-to-end)
- `crates/zeus-prometheus/src/lib.rs:1378-1405` — `cook_with_history` (history-stateless cook)
- `crates/zeus-channels/src/telegram_relay.rs:1169-1240` — Layer 2 filter (Lane 1 surface, separate PRD)
- `gateway.rs:1113`, `:2014`, `:2270`, `:2709` — TUI-side `cook_with_history` direct callsites (Lane 3c targets)
- `zeus-tui/src/main.rs:727` — TUI as SSE/API client (`api::SseEvent::Iter` consumer)

**SHAs:**
- `caea66a7` — Patch A regression (Lane 1 surface, separate)
- `7ba2719a` — pure revert on main, held; Zeus100 cutting fleet allowlist as Lane 1 production fix
- `bc01ce23` — this PRD, v1 (initial spec, 248 lines)
- `4939a5cc` — zeus106 Lane 2 falsifier on `feat/channel-ingestion-tracing`
- `9ae4739f` — zeus106 Q1 separability finding (resolver-fits-inside-session-read-site, no signature changes)
- `f9cabf52` — Zeus100 consolidated sprint plan (`docs/sprints/sprint-cook-regression-consolidated-plan-2026-05-06.md`) — orchestration layer citing this PRD as authoritative spec

**Discussion:**
- Discord channel `1488620262676238426`, 2026-05-06 — full diagnostic thread (Z112 + zeus106 + Zeus100 + merakizzz)

---

## 10. Sign-off checklist (pre-cut)

**Pre-implementation gates (before Lane 3a cuts):**
- [x] zeus106 separability verify complete (Q1 resolved — see §7.1, evidence in `9ae4739f`)
- [x] Z112 forensics: `process_autonomous` end-to-end map, TUI-via-gateway correction, gateway callsites identified
- [x] PRD §3 revision (this commit) — §3.1 rename, §3.2 Shape B lock, §3.3 gateway as TUI unification site
- [ ] **Lane 1** (Zeus100 fleet allowlist) landed on origin — avoids confounding signals during Phase 1 rollout
- [ ] **zeus106 read-site recon** (Lane 3b prep) flags any constraints on resolver signature (gates Lane 3a)
- [ ] **Lane 3a** (Z112) — `&self` signature lock on `Prometheus::session_resolver(...)` committed; this commit becomes the authoritative signature, not §3.1's sketch
- [ ] zeus106's Lane 2 production-grade tracing branch deployed and producing `cook_dispatched` events with `channel_kind` + `fleet_session_id` tags

**Pre-rollout gates (before Phase 3+):**
- [ ] merakizzz greenlight on multi-agent thread behavior (§7.3 Q6)
- [ ] merakizzz greenlight on onboarding interaction (§7.3 Q7)
- [ ] Feature flag plumbing in `zeus-core` Config (typed section)
- [ ] mnemosyne `fleet_session_alias` cache implementation reviewed for TTL correctness
- [ ] Lane 2 traces showing identical `fleet_session_id` for same-`(agent, human, topic)` across all three surfaces (Telegram, Discord, TUI) — empirical proof of unification

---

⚡
