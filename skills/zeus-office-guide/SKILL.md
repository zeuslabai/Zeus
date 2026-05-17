---
name: zeus-office-guide
description: Guide to The Office TUI — pixel art fleet visualization. Use when users ask about the office view, agent locations, zone meanings, or office keyboard shortcuts.
---

# The Office TUI Guide

## When to Use

Trigger on: the office, office tab, agent locations, where is agent, zone meaning, office keys, pixel art view, fleet visualization.

NOT for: Pantheon war rooms (use zeus-pantheon-guide), fleet deployment (use zeus-fleet-deploy), agent health (use zeus-fleet-health).

---

## What It Is

The Office is a half-block pixel art visualization of the Zeus fleet. Each terminal cell shows 2 vertical pixels using the `▀` character. An 80x40 pixel office renders in 80x20 terminal cells.

Agents appear as 8x12 pixel sprites that walk between zones based on their current activity.

## Access

Press **Tab** to cycle tabs: Chat → Office → Pantheon.

## Zones

| Zone | Location | Agent States | Color |
|------|----------|-------------|-------|
| Engineering | Top-left | Writing, Executing | Red |
| Comms | Top-right | Syncing | Blue |
| Research | Bottom-left | Researching, Error | Cyan |
| Break Room | Bottom-right | Idle | Yellow |

When an agent's status changes (e.g. idle → executing), they walk from their current zone to the new one.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Tab | Switch to next tab |
| F | Cycle focus between agents |
| M | Toggle yesterday's memo overlay |
| ? | Toggle help overlay |
| R | Force reconnect to gateway |
| Esc | Close overlays / unfocus agent |

## Sidebar

Right panel shows:
- **AGENTS**: List with status dot, name, state label, current task, model
- **ZONES**: Agent count per zone
- **STATS**: Tick counter, active/idle/error counts

## Focus Mode

Press **F** to cycle through agents. Focused agent shows expanded detail:
- Full status, zone, model
- Complete task text (wrapped)
- Position coordinates
- Agent ID

Press **Esc** to unfocus.

## Memo Overlay

Press **M** to see yesterday's daily note from Mnemosyne. Shows agent activity summary from the previous day.

## Data Source

The Office polls `GET /v1/network/agents` every 5 seconds. Agent status, current_task, and model are synced from the fleet registry. Demo agents appear when the gateway is offline.
