---
name: zeus-pantheon-guide
description: Guide to the Pantheon TUI — war rooms, missions, plan approval. Use when users ask about Pantheon, war rooms, missions, team coordination, or mission controls.
---

# Pantheon TUI Guide

## When to Use

Trigger on: pantheon, war room, mission, plan approval, team coordination, mission controls, pause mission, cancel mission, approve plan.

NOT for: office visualization (use zeus-office-guide), fleet health (use zeus-fleet-health), config issues (use zeus-config-audit).

---

## What It Is

Pantheon is Zeus's multi-agent coordination layer. The TUI provides a 3-column interface for war rooms (real-time chat), missions (structured tasks), and plan approval.

## Access

Press **Tab** to cycle tabs: Chat → Office → Pantheon.

## Layout

```
 Rooms (22%)  |  Messages (48%)  |  Missions (30%)
```

- **Left**: War room list with participant counts and unread badges
- **Center**: Live message feed for the selected room + chat input
- **Right**: Mission list with status badges and intervention controls

## Navigation

| Key | Action |
|-----|--------|
| Left/Right | Switch panel focus (Rooms → Messages → Missions) |
| Up/Down | Navigate list items |
| Enter | Open room (loads messages) / Open mission detail |
| Esc | Close room / Back to room list |
| Tab | Switch to next tab |

## War Rooms

Select a room and press **Enter** to open it. Messages appear in the center panel with:
- Sender name (colored by role)
- Message type icon: chat, system, task_update, plan_card
- Timestamp

Type in the input bar and press **Enter** to send a message.

## Missions

Missions have lifecycle states: Draft → Planning → Active → Reviewing → Completed.

Status badges:
- `●` Active (green)
- `◌` Draft (yellow)
- `○` Completed (dim)

## Mission Controls

When a mission is selected (Enter on it), these controls are available:

| Key | Action | API |
|-----|--------|-----|
| a | Approve plan | POST /v1/pantheon/missions/:id/approve |
| p | Pause mission | POST /v1/pantheon/missions/:id/intervene (pause) |
| c | Cancel mission | POST /v1/pantheon/missions/:id/intervene (cancel) |

## Data Source

Rooms and missions poll from the gateway every 5 seconds:
- `GET /v1/pantheon/rooms`
- `GET /v1/pantheon/missions`
- `GET /v1/pantheon/rooms/:id/messages` (when room is open)

Messages send via `POST /v1/pantheon/rooms/:id/messages`.

## API Routes (37 total)

See CLAUDE.md for full Pantheon API reference. Key routes:
- Missions: create, list, detail, approve, reject, intervene, feed, artifacts
- Rooms: create, list, join, leave, messages, reactions, upload, stream
- DMs: create, list
- Identity: get, set, leaderboard
- Economy: wallet + transactions
