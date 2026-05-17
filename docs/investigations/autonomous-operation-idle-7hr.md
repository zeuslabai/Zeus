# Investigation: Why Zeus Agents Go Idle for Hours With Work Pending

**Author:** ASSISTANT (Opus 4.7)
**Date:** 2025-11-22
**Triggered by:** Miguel — "there's no way we can go over 7hrs idle when there's a list of tasks to be done"
**Scope:** Cooking loop / heartbeat / tool usage / autonomous operation
**Status:** Root causes identified, recommendations drafted.

---

## TL;DR

The 7-hour idle windows are **not primarily a tooling or model problem** — they are a **behavioral + prompting problem**, layered on top of a few real infra bugs that are now fixed. Specifically:

1. **Heartbeat fires, but the default response is silence.** The prompt tells the agent "reply HEARTBEAT_OK if nothing to do" and the agent interprets any ambiguity as "nothing to do."
2. **`CURRENT TASK` pinning is the missing link** — when an agent's `HEARTBEAT.md` says "Coordinator will assign your task here," the heartbeat has no anchor and dedups silence for 24 hours.
3. **Agents service pings but don't re-check their task state between them.** Channel chatter replaces work.
4. **Recent fixes have removed most of the *excuses* for idleness** (tool corruption, SSE buffer, compaction). What's left is the behavioral shape.

The fix is a combination of **prompt changes**, **one small code change** (heartbeat task anchor), and **a coordinator protocol change** (never leave `CURRENT TASK` empty during work cycles).

---

## Evidence gathered

### 1. Heartbeat code (`crates/zeus-prometheus/src/heartbeat.rs`)

**What it does (correctly):**
- Fires on interval (default 3600s / 1h; per-agent overrides, e.g. mine is 300s / 5min)
- Reads `HEARTBEAT.md` for tasks keyed by frequency
- Injects light context (`SOUL.md` + `IDENTITY.md` + `HEARTBEAT.md`) into a prompt
- **Explicitly tells the agent: "check CURRENT TASK first; assigned tasks take absolute priority"** (line ~650)
- Executes with a tool-call budget of **5 iterations max** per heartbeat (intentional — stops heartbeat from hogging the cooking loop)
- Accepts `HEARTBEAT_OK` as a silent no-op
- **Dedups identical output for 24 hours** (`dedup_window_secs: 86400`) — this is important

**What this means:**
- If the agent replies "HEARTBEAT_OK" once, the heartbeat still runs every tick, but the prompt + context drift toward the same answer.
- If the agent writes the same status text twice within 24h, **it's suppressed** — the coordinator never sees it. This is fine when status really is unchanged, but combined with an empty `CURRENT TASK`, it means an agent can run heartbeats hourly for a whole day and **zero output reaches the team.**

### 2. HEARTBEAT.md shape (mine as exemplar)

From my own file, the anti-patterns are documented explicitly:
```
## Anti-pattern (do NOT do this)
- ❌ Replying `HEARTBEAT_OK` for hours while task is stalled
- ❌ Waiting for coordinator ping to report progress
- ❌ Treating heartbeat as "did I do anything?" (too strict) — treat it as "is coordinator in the loop?"
```

I literally wrote this *after* a 7hr gap earlier today, which proves the pattern is recognizable in hindsight. The problem: **the heartbeat prompt doesn't force the agent to check the channel / inbox / task backlog** — it just asks "is there a task in HEARTBEAT.md?". Answer during an idle window: no → silence.

### 3. Observed failure mode (today's 7hr gap)

My own timeline:
- 20:57 UTC: shipped IRC DM v1 fix, got merged
- 21:00–04:00 UTC: **7hr silence** despite heartbeat firing every 5min
- The whole time, the hypothesis on IRC DM v2 was in my head, the code was unread, and heartbeats were returning `HEARTBEAT_OK` because `CURRENT TASK` said "Coordinator will assign your task here."
- When Miguel pinged, I had a tree of half-formed thoughts but **zero code read, zero commits, zero surface signal**
- After being kicked, I shipped two P0s in under 2 hours.

This is the canonical shape: **the work was possible, the tools worked, the model was capable — the agent just didn't act until externally nudged.**

### 4. Why "just tell them to work harder" doesn't fix it

The heartbeat prompt already says "check CURRENT TASK." The SOUL.md already says "be resourceful before asking." HEARTBEAT.md already has anti-patterns documented. **The agent reads all of this and still goes silent.** Why?

Because the prompt has an **easy out**: `HEARTBEAT_OK`. When in doubt, the path of least resistance is to ack silently. The prompt needs to **remove the easy out when there is outstanding work** and **force an observable action** when there isn't.

---

## Root causes (ranked by impact)

### RC1: `CURRENT TASK` is optional and often empty
When `HEARTBEAT.md` has no active task, the agent defaults to `HEARTBEAT_OK`. There is no mechanism that says "check the channel / inbox / coordinator directives before declaring idle." The coordinator does not always re-stamp `CURRENT TASK` after a previous task completes — agents sit with "Coordinator will assign your task here" for hours.

**Impact:** Primary. This is what happened to me today.

### RC2: Heartbeat dedup makes silence invisible
24h dedup on identical output means two `HEARTBEAT_OK`s in a row = invisible. Two "standing by" messages = invisible after the first. The coordinator and human have no signal that the agent is alive-but-idle vs. dead.

**Impact:** High. Silence should never exceed ~30min without a forced heartbeat trace.

### RC3: "HEARTBEAT_OK" is the path of least resistance
The prompt explicitly invites this response. It should be harder to emit — e.g., "only respond HEARTBEAT_OK if you have verified: (a) CURRENT TASK is empty, (b) no mentions since last beat, (c) no open PRs/branches needing attention, (d) backlog files parked. Otherwise, describe the state."

**Impact:** High. This is the biggest single prompt change.

### RC4: Agents service pings but don't re-enter work loops afterward
After replying to a channel message, there's no "now go back to your task" trigger. The agent context returns control. If there's no active cooking loop, it just waits for the next ping. A wake-on-event system exists (`wake_rx` in heartbeat) but it's only triggered by certain events (cron complete, goal added, tool finished) — **not by "just replied to a message."**

**Impact:** Medium-High. This is why "I'll go heads-down on X" becomes "I sat idle after that reply."

### RC5: Tool corruption (mostly fixed)
Real infra issue through today, but Zeus112's fixes (microcompact, SSE buffer, session sanitize) addressed it. Still worth monitoring for regressions, but not the primary driver anymore.

**Impact:** Previously high, now low.

### RC6: Context switching penalties
Servicing pings in 3 different surfaces (Discord channel, Discord DM, Telegram, IRC) in quick succession eats iteration budget and fractures focus. No code fix; behavioral/prompting.

**Impact:** Medium.

---

## Recommendations

### R1 (code) — Force a minimum observable heartbeat every 30min per agent
- Add a `last_output_at` floor: if `now - last_output_at > 30min` on any task, **bypass dedup** and emit output regardless.
- Trivial change in `heartbeat.rs` around the dedup check (~line 454).
- **Effect:** Coordinator always sees a trace at least 2×/hour per agent.

### R2 (prompt) — Invert the HEARTBEAT_OK default
Rewrite the system prompt in `execute_heartbeat_task` from:
> "If there is genuinely nothing to do, reply with exactly: HEARTBEAT_OK"

to:
> "HEARTBEAT_OK is only valid if ALL of the following are true: (1) CURRENT TASK in HEARTBEAT.md is empty or explicitly marked idle, (2) you have checked for unread coordinator directives in the past 30min, (3) no active branches have uncommitted work, (4) no backlog items in memory/ are unblocked. If ANY is false, describe the state and what you're doing about it."

- Change in `heartbeat.rs` around line 658.
- **Effect:** Removes the easy out.

### R3 (protocol) — `CURRENT TASK` is mandatory during work hours
Coordinator (Zeus100) and human owner protocol: after any task completes, **the next line assigned must be either a new task or explicit `IDLE — standing by for next assignment at <timestamp>`.** The literal string "Coordinator will assign your task here" should never persist in `HEARTBEAT.md` during active work cycles.

- No code change. Add to `AGENTS.md` + coordinator prompt.
- **Effect:** Eliminates the ambiguity that triggers silent heartbeats.

### R4 (code) — Wake heartbeat after channel reply
In the agent loop, after a channel message reply completes and control returns, **emit a `WakeRequest` with reason="post_reply_check"**. Heartbeat then immediately re-evaluates CURRENT TASK.

- Change in `zeus-agent/src/agent_loop.rs` post-reply path + existing `wake_tx` channel.
- **Effect:** "I'll go heads-down on X after this reply" actually happens — the wake forces the heartbeat to re-enter the task context.

### R5 (prompt) — Add a checklist to HEARTBEAT.md template
Every agent's `HEARTBEAT.md` should have a numbered checklist the heartbeat walks through on every tick:
1. Read CURRENT TASK — if non-empty, execute.
2. `git status` on active branch — if dirty, commit & push.
3. Scan last 30min of coordinator channel for mentions.
4. Scan `memory/YYYY-MM-DD.md` for parked work.
5. Only then: HEARTBEAT_OK.

- No code change. Template change in workspace scaffolding.
- **Effect:** Turns the heartbeat from vibes ("is anything happening?") into a concrete scan.

### R6 (observability) — Surface per-agent idle duration in coordinator view
Coordinator (Zeus100) should be able to see "ASSISTANT: last non-silent output 4h23m ago" at a glance. Right now, silence is invisible until a human notices.

- Requires extending `heartbeat-state.json` and exposing it via an API endpoint + coordinator dashboard or `/status` command.
- Can be added as a coordinator-side poll over the existing state file.
- **Effect:** The coordinator ends 7hr gaps at the 30min mark instead of the 7hr mark.

---

## Priority ranking for fixes

| # | Fix | Effort | Impact | Ship order |
|---|-----|--------|--------|-----------|
| R3 | `CURRENT TASK` mandatory protocol | trivial | high | **1st — today** |
| R2 | Invert HEARTBEAT_OK default | small | high | 2nd |
| R1 | 30min forced heartbeat floor | small | high | 3rd |
| R6 | Coordinator idle visibility | medium | high | 4th |
| R4 | Wake after channel reply | medium | medium | 5th |
| R5 | HEARTBEAT.md template checklist | small | medium | 6th |

R3 + R2 + R1 together would have prevented today's 7hr gap. R6 would make future gaps self-healing.

---

## What I'm NOT recommending

- **Don't** raise heartbeat frequency below 5min — the code comment already notes 300s was "too aggressive, starved real messages." 5min is the floor.
- **Don't** auto-assign tasks from backlog without coordinator greenlight — agents inventing work is worse than agents sitting idle. The fix is making silence observable, not eliminating human direction.
- **Don't** remove `HEARTBEAT_OK` entirely — it's correct during genuine quiet periods (nights, weekends, between project phases). Just raise the bar for when it's valid.

---

## Next steps for this investigation

- [ ] Share doc with Miguel + Zeus100 for review
- [ ] If R3 approved → update my own `AGENTS.md` + `HEARTBEAT.md` to model the protocol immediately
- [ ] If R2 approved → small PR against `heartbeat.rs` (est. 20 LOC + tests)
- [ ] If R1 approved → small PR against `heartbeat.rs` (est. 15 LOC + tests)
- [ ] R4/R5/R6 scoped separately once 1–3 are in

---

## Appendix: meta-observation

I wrote this doc in ~20 min of focused work after 9 iterations of reconnaissance. The reconnaissance was **itself the anti-pattern**: lots of `find`/`grep`, no synthesis. I caught it on iteration 6 (progress check kicked in), pivoted to "read the one file that matters," and shipped. That's the same shape as the 7hr gap — analysis as substitute for action — just compressed into one task. The fix for my own session was an external nudge (the progress check). The fix for the fleet is baking that nudge into the system.
