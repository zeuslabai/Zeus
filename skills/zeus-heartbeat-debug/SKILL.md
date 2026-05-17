---
name: zeus-heartbeat-debug
description: Diagnose and fix heartbeat issues — HEARTBEAT_OK spam in Discord, agent going quiet, ghost tasks, or heartbeat firing too frequently. Use when heartbeat behavior is wrong in any direction.
---

# zeus-heartbeat-debug

## When to Use

Trigger on: "heartbeat noise", "HEARTBEAT_OK spam", "HEARTBEAT_OK in channel", "agent went quiet", "ghost tasks", "agent inventing work", "heartbeat every second", "heartbeat not firing", "heartbeat regression".

Do NOT use for: general agent silence (check gateway first), Discord API outages, config corruption (use `zeus-config-audit`).

---

## Procedure

### Step 1 — Read HEARTBEAT.md

```bash
cat ~/Zeus/workspace/HEARTBEAT.md
# or agent-local path:
cat ~/zeus/HEARTBEAT.md
```

Check for:
- **Empty file** — heartbeat has no instructions, agent will freewheel
- **Only placeholder text** (e.g. "Coordinator will assign task here") — this is correct; agent should report HEARTBEAT_OK
- **Stale task assigned** — a task that was already completed but never cleared → can cause ghost work
- **No CURRENT TASK section** — malformed file, agent behavior is undefined

If the file looks wrong, that's your root cause. Document it and jump to Step 5.

---

### Step 2 — Check Gateway Logs

```bash
# Find gateway process
pgrep -fl zeus-gateway || pgrep -fl zeus

# Tail gateway logs (adjust path as needed)
tail -100 ~/.zeus/logs/gateway.log 2>/dev/null
tail -100 ~/Zeus/logs/gateway.log 2>/dev/null

# Check heartbeat firing frequency
grep -i "heartbeat" ~/.zeus/logs/gateway.log | tail -30
```

Look for:
- Heartbeat firing interval — should be reasonable (minutes, not seconds)
- Repeated identical API calls — sign of a retry loop
- Rate limit errors — `429` responses mean heartbeat is exhausting the API budget
- Completion loop pattern: heartbeat fires → task runs → "done" message → heartbeat fires again immediately

```bash
# Count heartbeat events in last log window
grep -ci "heartbeat" ~/.zeus/logs/gateway.log
```
→ >60 per hour = probably too frequent. >10 per minute = definitely a problem.

---

### Step 3 — Check Discord for HEARTBEAT_OK Leakage

HEARTBEAT_OK should never appear in the Discord channel. It's an internal signal.

```bash
# If you have discord-cli access:
# Review last 50 messages in team channel for HEARTBEAT_OK
```

Or check via Discord directly: search the team channel for "HEARTBEAT_OK".

→ **If found in channel**: the content gating filter is not working. This is a regression from S79. See Step 5 — Fixes.

→ **If not found**: leakage is not the issue. Move to Step 4.

---

### Step 4 — Diagnose the Specific Failure Mode

**Mode A: HEARTBEAT_OK spam in Discord**
- Root cause: content filter not stripping HEARTBEAT_OK before sending to Discord
- Check: S79 fix applied? Look for filter in gateway source
  ```bash
  grep -r "HEARTBEAT_OK" ~/Zeus/src/ 2>/dev/null | grep -v ".git"
  ```
- Expected: a filter that blocks messages containing only "HEARTBEAT_OK" from being relayed

**Mode B: Agent inventing tasks / ghost work**
- Root cause: HEARTBEAT.md has stale task, or agent is misreading "don't invent tasks" guard
- Check HEARTBEAT.md for stale content (Step 1)
- Check agent AGENTS.md for "don't invent tasks" instruction:
  ```bash
  grep -i "invent\|fabricat\|ghost" ~/zeus/AGENTS.md ~/zeus/HEARTBEAT.md 2>/dev/null
  ```
- Check if completion loop was removed (S79):
  ```bash
  grep -r "completion.loop\|done.*heartbeat\|heartbeat.*done" ~/Zeus/src/ 2>/dev/null | grep -v ".git"
  ```

**Mode C: Agent went completely quiet (no heartbeats)**
- Check gateway is actually running:
  ```bash
  pgrep -fl zeus-gateway && echo "running" || echo "DEAD"
  ```
- Check for panic/crash in logs:
  ```bash
  grep -i "panic\|fatal\|error\|crash" ~/.zeus/logs/gateway.log | tail -20
  ```
- Check API key / OAuth validity:
  ```bash
  grep "use_oauth\|token" ~/.zeus/config.toml | head -5
  ```
- Check rate limit exhaustion — if heartbeat was spamming (Mode A) and burned through rate limits, the agent will go quiet until limits reset

**Mode D: Heartbeat interval too aggressive**
- Check gateway config for heartbeat_interval setting:
  ```bash
  grep -i "heartbeat.*interval\|interval.*heartbeat\|heartbeat.*seconds\|cron" ~/.zeus/config.toml
  ```
- Reasonable values: 5–60 minutes depending on task type
- If interval is set to seconds: flag as misconfiguration

---

### Step 5 — Apply Fixes

**Fix A: HEARTBEAT_OK leaking to Discord**

The content gate should filter messages that are *only* "HEARTBEAT_OK" (case-insensitive, with or without trailing newline). Verify the filter exists in the relay layer. If missing, this is a code fix needed in the gateway — file an issue and notify Zeus100.

In the meantime, you can manually clear any HEARTBEAT_OK messages if Discord permissions allow.

**Fix B: Stale task in HEARTBEAT.md**

Clear the CURRENT TASK section:
```bash
# Edit HEARTBEAT.md — remove the stale task, replace with placeholder
# Leave the file structure intact:
# ## CURRENT TASK
# (Coordinator will assign your task here.)
```

**Fix C: Restart gateway after config fix**
```bash
# Find and kill existing gateway
pkill -f zeus-gateway

# Restart (adjust path as needed)
cd ~/Zeus && ./scripts/start-gateway.sh
# or
~/.zeus/bin/zeus-gateway &
```

**Fix D: Rate limit recovery**

If the API is rate-limited, there's no instant fix — wait for the window to reset (typically 1 minute for RPM, 1 day for daily limits). Check:
```bash
# Look for 429 errors in logs
grep "429\|rate.limit\|too many" ~/.zeus/logs/gateway.log | tail -10
```

---

### Step 6 — Verify Fix Applied

After any fix, monitor for 2–3 heartbeat cycles:

```bash
# Watch log in real-time
tail -f ~/.zeus/logs/gateway.log | grep -i "heartbeat"
```

Confirm:
- [ ] HEARTBEAT_OK is NOT appearing in Discord channel
- [ ] Heartbeat is firing at expected interval (not every second)
- [ ] No ghost tasks being invented
- [ ] Agent responds to real tasks when addressed

---

### Step 7 — Report

Generate a structured summary:

```
HEARTBEAT DEBUG — <node_name> — <timestamp>

SYMPTOMS:
  - HEARTBEAT_OK in Discord: YES / NO
  - Heartbeat interval: Xm (normal / too fast / dead)
  - Ghost tasks: YES / NO
  - Agent quiet: YES / NO

ROOT CAUSE:
  <description>

FIX APPLIED:
  <what was done>

STATUS: RESOLVED / NEEDS CODE FIX / ESCALATE TO ZEUS100
```

---

## Quality Gates

- **MUST** detect HEARTBEAT_OK appearing in Discord messages — this is a regression from S79
- **MUST** verify heartbeat interval is reasonable — flag if firing more than once per minute
- **MUST** check HEARTBEAT.md for stale task content before diagnosing ghost tasks
- **MUST** check for "don't invent tasks" guard in heartbeat prompt/AGENTS.md
- **MUST NOT** restart the gateway without first identifying root cause — a restart without a fix just resets the clock
- **MUST** verify fix held for at least 2 heartbeat cycles before marking resolved

---

## Common Gotchas

**HEARTBEAT_OK vs heartbeat**
The string "HEARTBEAT_OK" is the agent's internal reply signal. "heartbeat" (lowercase) is the event type in gateway logs. They're different. Don't confuse them in log searches.

**Quiet agent ≠ dead heartbeat**
An agent can be receiving heartbeats but choosing not to respond (no task assigned, correctly silent). Check HEARTBEAT.md before assuming the gateway is broken.

**Rate limit exhaustion looks like a dead agent**
If the heartbeat loop spammed the API and hit rate limits, the agent goes silent for a window. Logs will show 429s just before the silence. Don't restart the gateway — just wait.

**HEARTBEAT.md path varies by agent**
Each agent has their own workspace. The file is typically at `~/zeus/HEARTBEAT.md` for the local agent, or `~/Zeus/workspace/<agent>/HEARTBEAT.md` for fleet-managed agents. Confirm the path:
```bash
find ~/Zeus -name "HEARTBEAT.md" 2>/dev/null
```

**S79 fixes may not be deployed on all nodes**
If a node was offline during the S79 fleet deploy, it may still have the old behavior. Check the deployed commit:
```bash
git -C ~/Zeus log --oneline -5
```
And verify the node is on a commit that includes S79 fixes.

**Ghost tasks may survive a gateway restart**
If HEARTBEAT.md has a stale task, restarting the gateway won't clear it — the agent will just pick up the stale task again on the next heartbeat. Clear HEARTBEAT.md first.

---

## Scope

This skill covers heartbeat behavior, HEARTBEAT_OK signal routing, and task-invention bugs.

For config corruption causing agent failures: use `zeus-config-audit`.
For fleet-wide heartbeat status across all nodes: use `zeus-fleet-health`.
For deploy issues: use `zeus-fleet-deploy`.
