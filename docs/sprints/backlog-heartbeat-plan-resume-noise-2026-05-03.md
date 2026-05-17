# Backlog — Heartbeat Plan-Resume Noise Loop

**Date:** 2026-05-03
**Author:** Zeus100 (diagnosis), Pre-launch backlogged by merakizzz
**Status:** Diagnosed + fix proposal locked. NOT dispatched. Hold until post-launch.
**Priority:** Medium — quality-of-life noise, not user-blocking. Some agents (zeus106, zeus107) emit `[Plan Resume]` and `[Heartbeat] hourly-N` on every tick.

---

## Symptom

Recurring auto-narration messages in the fleet Discord channel from multiple agents:

```
[Plan Resume] 2026-05-03-you-re-on-shared-team: Acknowledged @merakizzz — investigating the regression for a proper root-cause fix...
[Plan Resume] 2026-05-03-you-re-on-shared-team: Completed after 5 tool iterations
[Heartbeat] hourly-2: Completed after 5 tool iterations
[Heartbeat] hourly-3: Completed after 5 tool iterations
```

Same slug, same content, fires multiple times per session. Crowds the channel and offers no actionable signal.

---

## Source

`crates/zeus-prometheus/src/heartbeat.rs:712`:

```rust
let note = format!("[Plan Resume] {}: {}", slug, resume_result.output);
let _ = tx.try_send(note);
```

Surrounding logic (lines 686-724):

1. Each heartbeat tick scans `~/.zeus/workspace/plans/` for incomplete plans (PLAN.md files).
2. For **every** plan found, runs a "resume" LLM call: *"Resume this interrupted plan. Continue from where you left off..."*
3. LLM emits terse acknowledgement (especially on non-Claude models — glm-5.1, MiniMax, Kimi).
4. Output forwarded to channel as `[Plan Resume] <slug>: <output>`.

---

## Two compounding bugs

### Bug 1 — No completion marker on PLAN.md

`plan_mode::PlanMode` (in `crates/zeus-prometheus/src/plan_mode.rs`) loads PLAN.md and treats it as "incomplete" by default. There's no metadata indicating whether the plan is in-progress, complete, or abandoned. So:

- Plans created days ago are still "incomplete" by the runtime's definition
- Each heartbeat tick re-resumes them
- The slug `2026-05-03-you-re-on-shared-team` (visible in zeus106's emissions) is a stale plan from earlier today that should be marked done — but can't be

### Bug 2 — Plan-resume runs before `preflight_gate`, no `last_run` gating

The structured-task path was just fixed in commit `32f64d38` (heartbeat legacy-path gate by `last_run`). Plan-resume is on a DIFFERENT code path that wasn't touched by that fix:

- Line 690: `incomplete_plans` scan + resume loop
- Line 726: `preflight_gate(&workspace, &state).await` (the structured-task gate)

Plan-resume runs before the gate, with no `last_run` check. So unlike the now-correctly-gated hourly task, plan-resume fires on **every** heartbeat tick.

---

## Proper fix (three parts)

### 1. Add `status` frontmatter to PLAN.md

```markdown
---
status: in_progress | complete | abandoned
created_at: 2026-05-03T14:32:00Z
updated_at: 2026-05-03T15:48:00Z
---

# Plan title

[plan content]
```

Update `plan_mode::PlanMode::load` to parse this frontmatter and skip plans where `status != "in_progress"`.

### 2. Gate plan-resume by `last_run["plan_resume:<slug>"]`

Mirror the `preflight_gate` pattern from line 1031:

```rust
let last_run = state.last_run.get(&format!("plan_resume:{}", slug)).copied().unwrap_or(0);
let elapsed = now_unix.saturating_sub(last_run);
if elapsed < config.plan_resume_interval_secs {
    debug!("plan-resume: skipping '{}' — elapsed {}s < interval {}s",
           slug, elapsed, config.plan_resume_interval_secs);
    continue;
}
// proceed with resume...
state.last_run.insert(format!("plan_resume:{}", slug), now_unix);
```

Add `plan_resume_interval_secs: u64` to `HeartbeatConfig` (default 3600).

### 3. Auto-mark plan complete on LLM "done" signal

When `execute_heartbeat_task` returns `resume_result.success` AND the LLM output contains a clear completion marker (e.g., "PLAN COMPLETE" sentinel, or a heuristic like "no more steps remaining"), call `plan.mark_complete()` which writes `status: complete` to the frontmatter. Future ticks skip via Bug 1's gate.

Conservative alternative: just gate by Bug 2's `last_run` and let operators manually mark plans complete via a TUI command or by editing the frontmatter. Simpler, less risk of false-completion.

---

## Immediate cleanup (post-launch, before ship of fix)

For each fleet host with stale plan dirs, manually delete:

```
ssh mike@<host>
rm -rf ~/.zeus/workspace/plans/2026-05-03-you-re-on-shared-team/
# (or whatever slugs are stale — `ls ~/.zeus/workspace/plans/` to inspect)
```

This stops the noise immediately on the affected hosts without waiting for the code fix.

---

## Estimate

- Code change: ~80-120 LOC in `heartbeat.rs` + `plan_mode.rs`
- Tests: 2-3 new unit tests (frontmatter parsing, gate behavior, completion-marker flow)
- 1 session for an idle agent (zeus106 is the natural fit — just shipped the legacy-gate fix, has heartbeat context)

---

## Why backlogged

merakizzz's directive 2026-05-03: launch is Monday, no new features pre-launch, fixes only for actual regressions. The plan-resume noise loop is a quality-of-life issue (fleet channel cluttered with terse status), not a user-blocking bug. Banked here so the dispatch is ready when the post-launch sprint begins.
