# Sprint Design — Pantheon World Unification

**Status:** Design (pending greenlight)
**Branch:** `docs/sprint-design-pantheon-unification` off `origin/main` `9e5c78b4`
**Estimate:** 1–2 days execution after greenlight
**Author:** zeus112
**Reviewers:** merakizzz (greenlight), Zeus100 (dispatch)

---

## 0. TL;DR

Pantheon currently exists as **two parallel implementations** in the codebase that share a name but not a type system:

- **World A — `zeus-orchestra::pantheon`**: in-process orchestrator with `MissionEvent` enum broadcast over an internal `MessageBus`. The "intended" runtime model.
- **World B — `zeus-api::handlers::pantheon`**: HTTP/SSE surface with its own `PantheonEvent` enum, `PantheonStore` (sqlite-backed), and a 5,189-line handler module wired to the live TUI.

These two worlds **do not talk to each other.** World B is what the TUI actually consumes (via my adapter at `18d44484`). World A is wired into `AppState` (`pantheon_orchestrator: OnceLock<Arc<PantheonOrchestrator>>` at `lib.rs:386`) but the orchestrator's `MissionEvent` stream never reaches the API or the TUI.

This sprint picks **option (B) — collapse to api/handlers** as the unification target, because the TUI, the SSE feed, and the persistent store are already there. World A becomes a thin execution backend that emits into World B's event bus, not a parallel surface.

---

## 1. Inventory — Both Worlds

### 1.1 World A — `zeus-orchestra::pantheon`

**File:** `crates/zeus-orchestra/src/pantheon.rs`
**Surface:**
- `PantheonOrchestrator` struct (manages `Arc<RwLock<HashMap<String, Mission>>>`)
- `Mission`, `MissionState`, `TeamMember`, `MissionArtifact` types
- `MissionEvent` enum (12 variants) — broadcast intent
- `Intervention` enum: `Pause | Resume | Cancel | Redirect | ApproveTask | RejectTask`

**`MissionEvent` variants** (canonical orchestra shape):

| Variant | Fields |
|---|---|
| `MissionCreated` | `mission_id`, `goal` |
| `TeamAssembled` | `mission_id`, `agents: Vec<TeamMember>` |
| `TaskAssigned` | `mission_id`, `task_id`, `agent_id`, `description` |
| `AgentActivity` | `mission_id`, `agent_id`, `activity`, `detail: serde_json::Value` |
| `TaskCompleted` | `mission_id`, `task_id`, `result` |
| `TaskFailed` | `mission_id`, `task_id`, `error` |
| `ReviewRequested` | `mission_id`, `task_id`, `reviewer` |
| `MissionProgress` | `mission_id`, `progress_pct`, `tasks_done`, `tasks_total`, `tokens_used` |
| `ArtifactCreated` | `mission_id`, `task_id`, `name`, `artifact_type` |
| `MissionComplete` | `mission_id`, `status`, `summary`, `artifacts: Vec<MissionArtifact>` |
| `MissionFailed` | `mission_id`, `error` |
| `Intervention` | `mission_id`, `action`, `message: Option<String>` |

**Wiring:** `AppState::pantheon_orchestrator` is a `OnceLock` (lazy-init at `lib.rs:1198`). The `MessageBus` is internal to the orchestrator. **No HTTP route, no TUI consumer.**

### 1.2 World B — `zeus-api::handlers::pantheon`

**Files:**
- `crates/zeus-api/src/handlers/pantheon.rs` — **5,189 lines** (handlers, types, business logic, SSE wiring)
- `crates/zeus-api/src/handlers/pantheon_store.rs` — **2,656 lines** (sqlite persistence, `PantheonStore` at line 181)

**Surface:**
- `PantheonEvent` enum at `pantheon.rs:237` (~25 variants, superset of A)
- `PantheonStore` (sqlite, `pantheon.db`) — owns missions, rooms, DMs, plan cards, reputation, economy
- Live broadcast via `tokio::sync::broadcast` (per-mission and global channels)

**`PantheonEvent` variants** (superset; differences from A in **bold**):

| Variant | Notes |
|---|---|
| `MissionCreated` | adds **`status`** vs A |
| `TeamAssembled` | matches A |
| `TaskAssigned` | matches A |
| `AgentActivity` | adds **`agent_name`** vs A |
| `TaskCompleted` | matches A |
| `ReviewRequested` | matches A |
| `MissionProgress` | matches A |
| `Artifact` | renamed from `ArtifactCreated`; adds **`path`**, drops `task_id` |
| `MissionComplete` | matches A (different `Artifact` shape) |
| `MissionApproved` | **B-only** |
| `MissionFailed` | uses `reason` not `error` |
| **Room events** | `RoomCreated`, `RoomMessageSent`, `AgentJoinedRoom`, `AgentLeftRoom` — **B-only** |
| **Plan card events** | `PlanCardCreated`, `PlanApproved`, `PlanRejected` — **B-only**, used by approval flow |
| (others) | reputation, economy, skill cards |

**Endpoint inventory** (`crates/zeus-api/src/routes.rs:1061+`):

```
# Missions
POST   /v1/pantheon/missions
GET    /v1/pantheon/missions
GET    /v1/pantheon/missions/:id
POST   /v1/pantheon/missions/:id/intervene
POST   /v1/pantheon/missions/:id/approve
GET    /v1/pantheon/missions/:id/feed         (SSE)
GET    /v1/pantheon/missions/:id/artifacts

# Rooms (war rooms)
POST   /v1/pantheon/rooms
GET    /v1/pantheon/rooms
GET    /v1/pantheon/rooms/:id
POST   /v1/pantheon/rooms/:id/join
POST   /v1/pantheon/rooms/:id/leave
POST   /v1/pantheon/rooms/:id/messages
GET    /v1/pantheon/rooms/:id/messages
POST   /v1/pantheon/rooms/:id/upload
GET    /v1/pantheon/rooms/:id/members
POST   /v1/pantheon/rooms/:id/skill-card

# DMs
POST   /v1/pantheon/dms
GET    /v1/pantheon/dms

# Plans
POST   /v1/pantheon/plans/:id/approve
POST   /v1/pantheon/plans/:id/reject

# Reputation / economy
POST   /v1/pantheon/reputation/:agent_id
GET    /v1/pantheon/leaderboard
GET    /v1/pantheon/economy
```

> Note: PRD references `/missions/:id/{events,review}` — these are **not implemented**. The live SSE channel is `/missions/:id/feed`. Closing this gap is part of the migration.

### 1.3 TUI Mock Contract + Adapter at `18d44484`

**Adapter location:** `crates/zeus-tui/src/screens/pantheon/` (per Dispatch 32 revised)
**Files added in `18d44484`:**
- `api_types.rs` — TUI-local mirror of `PantheonEvent` + `ApiMission` with `#[serde(other)]` catch-all for forward-compat
- `adapter.rs` — `From<PantheonEvent> for AdapterOutput` (fan-out: `war_room` / `event_feed` / `open_plan_card` / `close_plan_card`); `From<&ApiMission> for MissionSummary`
- `sse.rs` — minimal WHATWG SSE decoder over `reqwest::bytes_stream` (no new dep)

**Known field gap (flagged in adapter):** `PlanCardCreated.steps[]` — TUI plan card UI expects an ordered list of steps to render checkboxes, but the API variant carries only `goal | complexity | risk`. Adapter has a `FIELD GAP` block; mock data fills steps locally.

**Contract guarantee:** the adapter is the **only** translation layer between B's wire types and the TUI display structs. Anything that breaks `PantheonEvent` forces an adapter change but does **not** force TUI screen rewrites.

---

## 2. Design Choice — A / B / C

### Option A — Collapse to orchestra
World B becomes a thin HTTP veneer over `zeus-orchestra::PantheonOrchestrator`. `PantheonStore` either moves into `zeus-orchestra` or becomes a side-effect listener.

**Pros:** clean architectural story (orchestra owns the runtime model); `MissionEvent` is shorter and cleaner.
**Cons:** ~7,800 lines of handler/store code must be ported into a crate that currently has no HTTP, no sqlite, no SSE infra. Rooms/DMs/reputation/economy don't fit the "orchestrator" mental model. **Massive blast radius.** TUI adapter fully breaks (different enum shape and no `agent_name`/`status` fields). High correctness risk on in-flight missions.

### Option B — Collapse to api/handlers ✅ **CHOSEN**
`zeus-orchestra::PantheonOrchestrator` becomes an internal **execution backend** that emits into the existing `PantheonEvent` broadcast bus. `MissionEvent` is deleted (or kept private to the orchestrator for internal step tracking) and replaced at the bus boundary by `PantheonEvent`.

**Pros:**
- Preserves the live HTTP surface, the SSE feed, the sqlite store, the TUI adapter contract — **zero breaking change to consumers.**
- Smallest blast radius: ~12 files in `zeus-orchestra`, no `zeus-api` rewrite.
- `PantheonEvent` is already the superset; A's variants all map cleanly.
- The plan-card / room / approval flows stay where they live (in handlers).

**Cons:**
- `zeus-api::handlers::pantheon` stays at 5,189 lines. Long-term it should be split, but **that's a separate refactor sprint, not this one.**
- Slight conceptual smell: orchestra crate depends on api types for the event enum. Mitigated by extracting `PantheonEvent` into `zeus-pantheon-types` (small new crate, ~1 file) so both sides depend on it without crate-cycle.

### Option C — Formal bridge layer
Keep both enums; define a `PantheonBridge` trait with `MissionEvent ↔ PantheonEvent` conversions. Orchestrator and API stay independent.

**Pros:** zero forced unification; can ship piece-by-piece.
**Cons:** **permanently** institutionalizes the duality. Every new event variant must be added in two places, kept in sync, and round-tripped. Future cost is unbounded. The current adapter at `18d44484` is already a bridge — option C just formalizes that we're stuck with it. Rejected.

### Decision — **Option B**

Argued on three axes:

| Axis | A | **B** | C |
|---|---|---|---|
| Blast radius | large | **small** | trivial |
| Correctness (in-flight missions) | risky | **safe** | safe |
| Future cost | medium | **low** | high (unbounded) |

B is the only option that is **simultaneously low blast-radius and low future-cost.**

---

## 3. Migration Plan

### Phase 0 — Pre-flight (30 min)
- Branch `feat/pantheon-unify-on-api` off `origin/main`.
- `cargo test -p zeus-api -p zeus-orchestra -p zeus-tui --lib` — capture baseline.

### Phase 1 — Extract `PantheonEvent` to shared crate (1–2h)
- New crate `crates/zeus-pantheon-types/` (or feature-gate inside `zeus-api`).
- Move `PantheonEvent`, `TeamMember`, `Artifact`, `Room`, `RoomMessage`, `MessageAttachment`, `MissionStatus` enums.
- `zeus-api::handlers::pantheon` re-exports for backward-compat; nothing else changes.
- **Verify gate:** `cargo build -p zeus-api`, all tests still pass.

### Phase 2 — Wire orchestrator to api event bus (2–4h)
- Add `Arc<broadcast::Sender<PantheonEvent>>` field to `PantheonOrchestrator`.
- Replace internal `MissionEvent` emission with direct `PantheonEvent` emission at the broadcast boundary. Keep `MissionEvent` private if useful for internal step tracking; otherwise delete.
- Map A→B at every existing emission site (12 variants). Where B has extra fields (`status`, `agent_name`, `path`), populate from `Mission` / `TeamMember` lookups.
- Wire `pantheon_orchestrator()` getter in `AppState` to pass the api's broadcast sender at init.
- **Verify gate:** integration test — start orchestrator, drive a mock mission, assert all events arrive on api SSE channel.

### Phase 3 — Close the `/feed` ↔ `/events` gap (1h)
- Add `/v1/pantheon/missions/:id/events` as alias of `/feed` (or document `/feed` as canonical).
- Add `PlanCardCreated.steps: Vec<PlanStep>` field — closes the field gap flagged in `18d44484`. TUI adapter loses its mock fill-in.
- **Verify gate:** TUI screen renders real plan card with steps end-to-end.

### Phase 4 — Cleanup (1h)
- Delete `zeus-orchestra::MissionEvent` if fully replaced.
- Remove dead `_message_bus` field if no longer needed.
- Update inline docs / CLAUDE.md.

### In-flight mission survival
The orchestrator currently doesn't persist mission state (`PantheonStore` does, on the api side). This means in-flight missions **already** live in B's world — so collapsing to B is safe. The orchestrator is restartable; on restart it reads from `PantheonStore` and resumes emitting into the same broadcast bus. **No data migration needed.**

---

## 4. Backwards Compatibility

| Surface | Impact |
|---|---|
| HTTP routes (`/v1/pantheon/*`) | **No change.** All endpoints preserved. |
| SSE wire format (`PantheonEvent` JSON) | **No breaking change.** Adding `PlanCardCreated.steps` is additive; TUI adapter has `#[serde(other)]` catch-all. |
| `PantheonStore` sqlite schema | **No change.** |
| TUI adapter at `18d44484` | **No breaking change.** `api_types.rs` already mirrors B; the `FIELD GAP` block on `PlanCardCreated.steps` becomes wired-up real data instead of mock. |
| `zeus-orchestra::MissionEvent` consumers | **Breaking, but zero known consumers** outside the orchestrator itself. (Verified by `grep -rn MissionEvent crates/`. If anything pops up at execution time, dispatch ping.) |
| `AppState::pantheon_orchestrator` callers | **No change** (still `OnceLock<Arc<PantheonOrchestrator>>`); the orchestrator just gets a new field. |

**TUI mock contract: preserved.** Mock data structures stay valid; live data slots in transparently once Phase 3 lands.

---

## 5. Test Strategy

### Unit
- `PantheonEvent` round-trip serde tests (already exist in `zeus-api`).
- `PantheonOrchestrator::emit_*` helpers — assert each emission produces the expected `PantheonEvent` variant with expected fields.

### Integration (`crates/zeus-api/tests/`)
- New: `pantheon_unification.rs` — start `AppState` with a real orchestrator, create a mission via `POST /v1/pantheon/missions`, drive it through team-assemble → task-assign → task-complete → mission-complete. Subscribe to `/feed` (SSE), assert event sequence and field shape.
- Negative: intervention path — `POST /:id/intervene` causes `Intervention` event on bus.
- Plan card path: `POST /pantheon/rooms/:id/...` triggers `PlanCardCreated` with `steps[]` populated.

### TUI
- Existing adapter tests (`crates/zeus-tui/src/screens/pantheon/adapter.rs`) — assert no regression on the catch-all variant.
- New: end-to-end with a stub api server emitting `PantheonEvent::PlanCardCreated { steps }` — assert TUI plan card renders all steps without mock fallback.

### Verify gates per phase
- Phase 1: `cargo build -p zeus-api && cargo test -p zeus-api --lib` clean.
- Phase 2: integration test green; manual smoke (`cargo run -p zeus-api`, drive a mission, watch SSE feed).
- Phase 3: TUI manual smoke — Tab 21, real plan card with steps.
- Final: `cargo test --workspace --lib` shows no new failures vs Phase 0 baseline.

---

## 6. Risk Register + Rollback

| # | Risk | Severity | Mitigation | Rollback |
|---|---|---|---|---|
| R1 | Hidden `MissionEvent` consumers outside orchestrator | Med | Pre-flight `grep -rn MissionEvent crates/`. If hits, dispatch ping before Phase 2. | Keep `MissionEvent` as `#[deprecated]` re-export for a release. |
| R2 | `PantheonEvent` extraction breaks `zeus-api` re-exports | Low | Phase 1 keeps re-export; only types move. | `git revert` Phase 1 commit; fully isolated. |
| R3 | Orchestrator can't populate B's extra fields (`agent_name`, `status`) | Low | Lookups against `Mission` / `state_manager` are cheap; if a field is unobtainable at emit time, default to `""` and add a TODO. | Make those fields `Option<String>`. |
| R4 | In-flight missions during deploy | Med | Orchestrator is stateless from a persistence standpoint; restart is safe. Deploy during a quiet window or drain first. | Restart api server; missions resume from `PantheonStore`. |
| R5 | `PlanCardCreated.steps` schema decision (struct shape) | Low | Choose minimal `PlanStep { id, description, status }`. Iterate later. | Field is additive — drop without breaking TUI. |
| R6 | 5,189-line `pantheon.rs` getting bigger | Low (out-of-scope) | Note in CLAUDE.md as future split candidate. | N/A — this sprint doesn't touch the size. |

### Rollback path (whole-sprint)
Each phase is a single commit (or short commit chain) on a feature branch. If Phase N fails verify gate, `git reset --hard` to Phase N-1 and ship the partial. Phases 1–2 are the only ones with type-shape changes; Phases 3–4 are additive/cleanup and can ship independently.

**Hard kill switch:** if Phase 2 reveals that orchestrator emission semantics genuinely diverge from `PantheonEvent` semantics (e.g., timing, ordering, retry behavior), abort and re-evaluate option C as a managed-debt path.

---

## 7. Open Questions for Reviewer

1. **`PlanStep` schema** — minimal `{id, description, status}` or richer `{agent_id, estimated_tokens, ...}`? Affects Phase 3 scope.
2. **Crate split** — extract `zeus-pantheon-types` as a real crate, or feature-gate inside `zeus-api`? Real crate is cleaner; feature-gate is faster.
3. **`MissionEvent` deletion** — full delete in Phase 4, or `#[deprecated]` for one release? Recommend full delete (no known consumers); flag if any surface during Phase 2.

---

## 8. Execution Owner

Recommend: **zeus112** (this author). Full design context + adapter context (`18d44484`) already loaded. Single-owner execution avoids handoff cost. Estimate 1.5 days end-to-end after greenlight.

---

*End of design.*
