# Coordinator Context Injection (B)

**Status:** Approved (operator 2026-06-25), pending implementation.
**Pairs with:** `personalities/leadership/the-coordinator.md` (persona hardened in `46bd43f9`).

## Goal

Make the coordinator's "track everything" behavior **code-enforced** (recalled-not-remembered), not prose-hoped. Extend the existing `[Work State]` injection with coordinator-scoped blocks so a coordinator structurally *cannot* lose track of the fleet, its own open loops, the roadmap, or the current mission — even across a context reset.

## Motivation

Operator-reported failure modes on other-project coordinators: not proactive, lose track of their *own* work, sideline onto off-mission work, no roadmap/dev-plan. The persona describes the right behavior, but persona prose is LLM-cooperation that drifts over a long session. The durable fix is at the code gate: inject the state every turn. (Same principle as `[Work State]` / "recalled, not remembered".)

## Foundation (already exists)

`crates/zeus-prometheus/src/goals.rs::format_work_state(active_goals, incomplete_plans, pending_tasks) -> Option<String>` builds a `[Work State]` block injected into the system prompt every cook (`crates/zeus-agent/src/agent_loop.rs:~1735`), shared no-drift across the cook path **and** the REST handlers (#168 Phase 4b). This is the engine we extend. Self-tracking (#2) is already half-solved when the coordinator's commitments land in `active_goals`; we ensure they do.

## Design

### 1. Role flag (prerequisite)
No `is_coordinator` runtime flag exists today. Add a minimal one:
- Config: `[agent].role = "coordinator"` (string; default none).
- Surface as `Agent::is_coordinator()` (reads config) so the injection path can branch.
- Only coordinators get the extra blocks → **zero context bloat for IC seats.**

### 2. `[Fleet Ledger]` block
- Data model: `FleetLedgerEntry { agent, assignment, status, branch: Option<String>, last_sha: Option<String>, updated_at: i64 }`.
- Store: lightweight `FleetLedger` — JSON at `~/.zeus/workspace/fleet_ledger.json` (v1; sqlite optional later), keyed by agent.
- Written by the coordinator on every dispatch + gate (`fleet_ledger.upsert(agent, ...)`, exposed as a tool the coordinator calls).
- Injected each turn as `[Fleet Ledger]`, one line per agent → coordinator always sees who's on what, status, last SHA.

### 3. `[Roadmap]` block
- Source: `~/.zeus/workspace/ROADMAP.md` (coordinator-owned artifact; same read pattern as `HEARTBEAT.md` CURRENT TASK).
- Inject the active **Now / Next / Blocked** section, truncated to a compact budget (don't blow context).
- Reader mirrors the existing workspace-file read path.

### 4. `[Mission]` line
- Surface the single current top priority prominently (reuse CURRENT TASK / the operator's stated priority) → coordinator re-anchors each turn → anti-drift.

### 5. Formatter + wiring
- Add `format_coordinator_context(fleet: &[FleetLedgerEntry], roadmap: Option<&str>, mission: Option<&str>) -> Option<String>` as a **sibling** to `format_work_state` (one formatter, both cook + REST paths — same no-drift discipline). Returns `None` when all empty.
- Wire into `agent_loop.rs` next to the existing `[Work State]` / `[Active Goals]` injection, gated behind `is_coordinator()`.

## Acceptance
- `cargo build --workspace` + `cargo test -p zeus-prometheus` + `cargo test -p zeus-core` green.
- New tests: formatter output shape; **role-gate (an IC agent gets NO coordinator blocks)**; fleet-ledger upsert + read; roadmap reader + truncation; empty-state → `None` (no bloat).
- A coordinator cook shows `[Fleet Ledger]` / `[Roadmap]` / `[Mission]` in its system prompt; an IC seat does not.
- No-drift: cook path + REST path emit identical blocks (shared formatter).

## Out of scope (follow-ups)
- v2: auto-populate the fleet ledger from git / agent-registry (v1 = coordinator writes it on dispatch/gate).
- Mission-lock **enforcement** (blocking IC tool-use for coordinators) is a separate cut — v1 is injection-only (re-anchor), not a hard tool-gate.
