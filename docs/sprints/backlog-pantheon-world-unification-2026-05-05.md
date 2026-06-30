# Backlog — Two Pantheon Worlds: Unification

**Status:** BACKLOG (sprint candidate, not assigned)
**Origin:** Zeus112's Dispatch 32 recon (2026-05-05) caught major model drift — banked here for future planning.

## Discovery (Zeus112, 2026-05-05)

Zeus has **two parallel "Pantheon" implementations** that are not connected:

### World 1 — `zeus-orchestra::pantheon`

- In-process mission-engine event system
- `MissionEvent` enum variants: `MissionCreated / TaskAssigned / TaskCompleted / TaskFailed / MissionProgress / ArtifactCreated / MissionComplete / MissionFailed / Intervention`
- **No `PlanCard` variant**
- Broadcast-channel based, used by Prometheus mission_driver

### World 2 — `zeus-api::handlers::pantheon`

- 5,189-line REST handler
- Has its own parallel types: `MissionStatus`, `AgentRole`, `MissionEvent`, `PantheonEvent::PlanCardCreated`
- Already emits `plan_card` messages at handlers/pantheon.rs lines 2680-2705 + 3339-3369
- Uses its own `PantheonStore` (2,656 lines)
- Broadcasts via `PantheonEvent`, not orchestra's `MissionEvent`

### Endpoint shape (current, real)

- `POST /v1/pantheon/missions` ✓
- `GET /v1/pantheon/missions/:id/events` (SSE)
- `GET /v1/pantheon/missions/:id/feed`
- `POST /v1/pantheon/missions/:id/intervene` (type = pause/resume/cancel/redirect)
- (Earlier dispatch-32 brief incorrectly assumed `GET /v1/pantheon/rooms/:id/stream` + separate approve/reject/redirect routes — those don't exist.)

## Why this is debt

- Two parallel event systems means any new feature has to be plumbed twice or risk diverging
- `PantheonStore` (api) and the orchestra's in-process state aren't reconciled — risk of stale UI state vs. true mission state
- TUI Phase 4 (`33200fe0`) had to use a thin From<PantheonEvent> adapter (Dispatch 32 revised) because the worlds don't share types
- New backend Pantheon work would need to choose which world to extend, increasing collisions

## Unification options (not yet evaluated)

### Option A — Collapse to orchestra
- Move `PantheonEvent` variants into `MissionEvent`
- Refactor `handlers/pantheon.rs` to consume `zeus-orchestra` broadcast directly
- Remove `PantheonStore` in favor of orchestra's in-process state
- Risk: 5,189-line handler refactor, multi-thousand-LOC blast radius

### Option B — Collapse to api/handlers
- Make `zeus-orchestra::pantheon` a thin shim that publishes into the api/handlers PantheonEvent broadcast
- Less invasive on the api side
- Risk: orchestra is the source of truth for mission lifecycle; making it a shim feels backward

### Option C — Bridge (no collapse)
- Keep both worlds, add a formal bridge layer that translates `MissionEvent ↔ PantheonEvent`
- Lowest disruption
- Risk: enshrines the two-world design as permanent + the bridge becomes its own surface

## Suggested owner (when activated)

- **Zeus112** (already deep in Pantheon TUI) or **zeus106** (post-TUI sprint, fresh context)
- NOT a parallel-cook task — needs single-author refactor discipline
- Estimated: 1-2 day single-titan sprint

## Dependencies / blockers

- None — Dispatch 32 revised (TUI-side adapter) handles the immediate TUI <-> backend integration without forcing this unification.
- Future Pantheon backend features (e.g., remote-fleet missions, multi-orchestrator consensus) may force the issue.

## Out of scope until activated

- Sprint design (which option, which owner, what's the migration path for in-flight missions)
- Backwards-compat strategy for any consumers of the existing `PantheonEvent` API shape
- Documentation refresh in CLAUDE.md (currently lists 35 Pantheon API routes assuming the api-side world is canonical)

## Banking notes

- Zeus112's Dispatch 32 recon (the message that flagged this) is the canonical write-up of the divergence
- TUI Phase 4 mock names (`MissionSummary` / `WarRoomMessage` / `PlanCardEvent` / `EventFeedItem`) were named for the "future ideal API" — these names should drive any unification design choice
- API-side names (`Mission` / `PantheonEvent::PlanCardCreated`) are the current production reality
