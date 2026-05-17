# Pantheon — Multi-Agent Collaboration System

Technical reference for the Pantheon mission system, War Room chat, and recovery mechanisms.

**Crates**: `zeus-api` (handlers, store, routes), `zeus-prometheus` (orchestration, spawner)

---

## Architecture

```
User (Web/iOS/macOS/TUI/Android/visionOS)
  │
  ├─ REST API ──→ zeus-api/handlers/pantheon.rs (missions, rooms, plans)
  ├─ WebSocket ──→ zeus-api/websocket.rs (real-time events)
  │
  ├─ PantheonStore ──→ SQLite (WAL mode, 5 tables)
  │
  └─ PantheonOrchestrator ──→ zeus-prometheus (planning, execution)
       ├─ ProactiveSpawner (agent lifecycle)
       └─ MissionDriver (task checkpointing)
```

### Data Flow

1. User creates mission via REST or WS
2. `PantheonOrchestrator` decomposes goal into tasks via Prometheus planner
3. Team assembled from `GlobalStateManager` agent registry
4. Tasks assigned to agents, executed via agent loop
5. Activity streamed to War Room via broadcast channel
6. Results persisted to SQLite, artifacts collected
7. Mission completes or fails, summary generated

---

## Mission State Machine

```
Created ──→ Assembling ──→ Executing ──→ Reviewing ──→ Complete
                              │                           │
                              ├──→ Paused ──→ Executing   │
                              │                           │
                              └──→ Failed ←───────────────┘
                                     ↑
                              Cancelled (from any active state)
```

| Status | Description |
|--------|-------------|
| `created` | Mission submitted, not yet started |
| `assembling` | Team being assembled from agent registry |
| `executing` | Tasks actively being worked |
| `paused` | Execution paused by user intervention |
| `reviewing` | All tasks done, awaiting human review |
| `complete` | Mission finished successfully |
| `failed` | Mission failed (timeout, agent error, user cancel) |
| `cancelled` | User cancelled the mission |

### Task Status

| Status | Description |
|--------|-------------|
| `pending` | Not yet started |
| `in_progress` | Agent actively working |
| `awaiting_review` | Task done, needs approval |
| `approved` | Reviewer approved output |
| `rejected` | Reviewer rejected, may retry |
| `complete` | Task finished |
| `failed` | Task failed |

---

## REST API — Missions

### POST /v1/pantheon/missions

Launch a new mission.

```bash
curl -X POST http://localhost:8080/v1/pantheon/missions \
  -H "Content-Type: application/json" \
  -d '{
    "goal": "Build a user authentication API",
    "constraints": {
      "budget_tokens": 50000,
      "timeout_seconds": 600,
      "max_agents": 4,
      "require_review": true
    }
  }'
```

**Response** (201):
```json
{
  "id": "m-a1b2c3d4",
  "goal": "Build a user authentication API",
  "status": "created",
  "team": [],
  "created_at": "2026-02-25T10:00:00Z"
}
```

Constraints are optional — defaults: 50K tokens, 600s timeout, 4 agents, no review.

### GET /v1/pantheon/missions

List all missions.

```bash
curl "http://localhost:8080/v1/pantheon/missions?limit=20&offset=0"
```

**Response**:
```json
{
  "missions": [
    {
      "id": "m-a1b2c3d4",
      "goal": "Build a user authentication API",
      "status": "executing",
      "team_size": 3,
      "progress_pct": 60.0,
      "tasks_done": 3,
      "tasks_total": 5,
      "created_at": "2026-02-25T10:00:00Z"
    }
  ],
  "total": 1,
  "offset": 0,
  "limit": 20
}
```

### GET /v1/pantheon/missions/:id

Full mission detail with team, tasks, feed, and artifacts.

```bash
curl http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4
```

**Response**: Full `Mission` object (see Types section).

### POST /v1/pantheon/missions/:id/intervene

Control a running mission.

```bash
curl -X POST http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4/intervene \
  -H "Content-Type: application/json" \
  -d '{"action": "pause"}'
```

| Action | Effect |
|--------|--------|
| `pause` | Pauses execution (executing → paused) |
| `resume` | Resumes execution (paused → executing) |
| `cancel` | Cancels mission (any active → cancelled) |
| `redirect` | Sends new direction (requires `message` field) |

### GET /v1/pantheon/missions/:id/feed

Activity feed for a mission.

```bash
curl http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4/feed
```

**Response**:
```json
[
  {
    "agent_id": "zeus-112",
    "agent_name": "Zeus112",
    "activity": "tool_call",
    "detail": {"tool": "shell", "args": "cargo test"},
    "timestamp": "2026-02-25T10:05:00Z"
  }
]
```

### GET /v1/pantheon/missions/:id/artifacts

Generated outputs from a mission.

```bash
curl http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4/artifacts
```

**Response**:
```json
[
  {
    "name": "auth_api.rs",
    "path": "/tmp/zeus/missions/m-a1b2c3d4/auth_api.rs",
    "type": "code",
    "created_at": "2026-02-25T10:08:00Z"
  }
]
```

### POST /v1/pantheon/missions/:id/review

Approve or reject a task output.

```bash
curl -X POST http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4/review \
  -H "Content-Type: application/json" \
  -d '{"task_id": "t-456", "action": "approve"}'
```

Actions: `approve` or `reject` (with optional `reason`).

### GET /v1/pantheon/missions/:id/events

SSE stream of real-time mission events.

```bash
curl -N http://localhost:8080/v1/pantheon/missions/m-a1b2c3d4/events
```

---

## REST API — War Room Chat

### POST /v1/pantheon/rooms

Create a chat room.

```bash
curl -X POST http://localhost:8080/v1/pantheon/rooms \
  -H "Content-Type: application/json" \
  -d '{"name": "general", "room_type": "public"}'
```

Room types: `public`, `private`, `mission` (auto-created with missions).

### GET /v1/pantheon/rooms

List all rooms with member counts.

### GET /v1/pantheon/rooms/:id

Room detail.

### POST /v1/pantheon/rooms/:id/join

Join a room. Body: `{"agent_id": "...", "agent_name": "..."}`.

### POST /v1/pantheon/rooms/:id/leave

Leave a room. Body: `{"agent_id": "..."}`.

### GET /v1/pantheon/rooms/:id/messages

Fetch messages. Query: `?limit=100`.

### POST /v1/pantheon/rooms/:id/messages

Send a message.

```bash
curl -X POST http://localhost:8080/v1/pantheon/rooms/room-id/messages \
  -H "Content-Type: application/json" \
  -d '{
    "sender_id": "zeus-112",
    "sender_name": "Zeus112",
    "content": "Build complete",
    "message_type": "chat"
  }'
```

Message types: `chat`, `system`, `tool_call`, `task_update`, `plan_card`, `plan_progress`, `deploy_status`, `skill_card`.

### GET /v1/pantheon/rooms/:id/members

List room members.

### POST /v1/pantheon/rooms/:id/messages/:msg_id/reactions

Add a reaction. Body: `{"user_id": "...", "emoji": "..."}`.

### DELETE /v1/pantheon/rooms/:id/messages/:msg_id/reactions

Remove a reaction. Body: `{"user_id": "...", "emoji": "..."}`.

---

## REST API — Plans & Economy

### GET /v1/pantheon/plans/pending

List plans awaiting approval.

### POST /v1/pantheon/plans/:id/approve

Approve a plan. Body: `{"user_id": "...", "user_name": "..."}`.

### POST /v1/pantheon/plans/:id/reject

Reject a plan. Body: `{"reason": "...", "user_id": "...", "user_name": "..."}`.

### PUT /v1/pantheon/identity

Set user identity. Body: `{"user_id": "...", "display_name": "..."}`.

### GET /v1/pantheon/identity/:id

Get identity by user ID.

### GET /v1/pantheon/identities

List all known identities.

### GET /v1/pantheon/economy

Shared economy dashboard (wallet balances, transaction history).

---

## SQLite Persistence

**File**: `~/.zeus/pantheon.db` (WAL mode)

### Tables

| Table | Columns | Purpose |
|-------|---------|---------|
| `missions` | id, goal, status, team (JSON), tasks (JSON), progress_pct, constraints (JSON), created_at, updated_at, completed_at, summary | Mission state |
| `tasks` | id, mission_id, description, assigned_to, status, result, created_at, updated_at | Individual tasks |
| `events` | id, mission_id, event_type, payload (JSON), created_at | Real-time events |
| `artifacts` | id, mission_id, name, path, type, created_at | Generated outputs |
| `activities` | id, mission_id, agent_id, agent_name, activity, detail (JSON), timestamp | Activity feed |

### Indexes

- `idx_missions_status` — fast status queries for recovery
- `idx_tasks_mission_id` — task lookups by mission
- `idx_events_mission_id` — event stream by mission
- `idx_activities_mission_id` — feed by mission

---

## Recovery System

### Startup Recovery

On gateway boot, `recover_stale_missions()` runs:

1. Loads all missions with status `Executing` or `Assembling`
2. Checks `updated_at` — if older than 5 minutes, marks as `Failed`
3. Marks all `InProgress` tasks within stale missions as `Failed`
4. Adds activity entry: `{ agent: "system", activity: "recovery" }`
5. Emits `MissionFailed` event for each recovered mission
6. Returns list of recovered mission IDs for logging

### Periodic Timeout Check

`start_timeout_check_task()` runs every 60 seconds:

1. Queries all executing missions
2. Compares elapsed time against `timeout_seconds` constraint (default: 30 minutes)
3. Auto-fails missions exceeding their timeout
4. Logs timeout events to activity feed

### Spawn Failure Recovery

`ProactiveSpawner::handle_spawn_failure()`:

1. Records failure in spawn tracker
2. Counts retries per `request_id`
3. If under retry budget (default: 3), returns replacement `SpawnRequest`
4. If exhausted, returns `None` — mission task marked failed

`SpawnHealthSummary` provides diagnostics:
- `active_spawns` — currently running
- `completed_total` — lifetime completions
- `completed_failures` — lifetime failures
- `success_rate` — percentage
- `is_healthy` — true if success_rate > 80%

---

## WebSocket Events

Extend the existing Zeus WebSocket protocol at `GET /v1/ws`.

### Client → Server

```json
{"type": "pantheon_mission", "goal": "...", "constraints": {...}}
{"type": "pantheon_intervene", "mission_id": "m-123", "action": "pause|resume|cancel|redirect"}
{"type": "pantheon_approve", "mission_id": "m-123", "task_id": "t-456"}
{"type": "pantheon_reject", "mission_id": "m-123", "task_id": "t-456", "reason": "..."}
```

### Server → Client

```json
{"type": "pantheon_mission_created", "mission_id": "m-123", "goal": "...", "team": [...]}
{"type": "pantheon_team_assembled", "mission_id": "m-123", "agents": [...]}
{"type": "pantheon_task_assigned", "mission_id": "m-123", "task_id": "t-456", ...}
{"type": "pantheon_agent_activity", "mission_id": "m-123", "agent_id": "...", "activity": "...", "detail": {...}}
{"type": "pantheon_task_completed", "mission_id": "m-123", "task_id": "t-456", "result": "..."}
{"type": "pantheon_review_requested", "mission_id": "m-123", "task_id": "t-456"}
{"type": "pantheon_mission_progress", "mission_id": "m-123", "progress_pct": 60.0, ...}
{"type": "pantheon_artifact", "mission_id": "m-123", "name": "...", "path": "...", "type": "code"}
{"type": "pantheon_mission_complete", "mission_id": "m-123", "status": "success", "summary": "..."}
```

---

## Key Files

| File | Lines | Purpose |
|------|-------|---------|
| `crates/zeus-api/src/handlers/pantheon.rs` | ~3,500 | REST handlers, types, mission orchestration |
| `crates/zeus-api/src/handlers/pantheon_store.rs` | ~2,500 | SQLite persistence, recovery, timeout checks |
| `crates/zeus-api/src/routes.rs` | — | Route registration (~30 Pantheon endpoints) |
| `crates/zeus-prometheus/src/spawner.rs` | — | Agent spawn lifecycle, failure recovery |
| `crates/zeus-prometheus/src/pantheon_bridge.rs` | — | Prometheus ↔ Pantheon integration |
| `apps/ZeusWeb/src/pages/pantheon.rs` | ~1,300 | Web frontend (Leptos/WASM) |
| `crates/zeus-tui/src/screens/pantheon.rs` | ~1,500 | TUI frontend (Ratatui) |
| `apps/ZeusDesktop/Views/Pantheon/` | — | macOS frontend (SwiftUI) |
| `apps/ZeusMobile/Views/Pantheon/` | — | iOS frontend (SwiftUI) |

---

## Frontend Support

All 6 frontends implement the Pantheon UI:

| Frontend | War Room Chat | Mission Detail | Plan Cards | Intervention | Artifacts |
|----------|:---:|:---:|:---:|:---:|:---:|
| Web (Leptos) | Y | Y | Y | Y | Y |
| macOS (SwiftUI) | Y | Partial | Y | — | — |
| iOS (SwiftUI) | Y | Partial | Y | — | — |
| TUI (Ratatui) | Y | Partial | — | — | — |
| Android (Compose) | Y | Y | Y | — | — |
| visionOS (SwiftUI) | Y | Y | Y | — | — |
