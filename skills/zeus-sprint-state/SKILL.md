---
name: zeus-sprint-state
description: Check, report, or reason about the current Zeus sprint state. Use when asked about the active sprint, what's done, what's in progress, or what comes next. Also use when updating HEARTBEAT.md with a new task or when orienting after a fresh session start.
---

# Zeus Sprint State

## When to Use

Trigger on: "what sprint are we on", "what's my current task", "what's done", "what's next", "S81 status", "sprint kickoff", "orient me", updating HEARTBEAT.md, or any question about sprint progress.

NOT for: deploying code (use zeus-fleet-deploy), auditing config (use zeus-config-audit), health checks (use zeus-fleet-health).

---

## Procedure

### Step 1 — Establish Current Sprint

```bash
# Read your own HEARTBEAT.md
cat ~/workspace/HEARTBEAT.md

# Check git branch for sprint context
cd ~/Zeus && git branch --show-current

# Check recent commits on the active sprint branch
git log --oneline -10
```

The sprint number is in the branch name (`feat/s81`) and in the `## CURRENT TASK` section of HEARTBEAT.md.

---

### Step 2 — Read Memory for Context

```bash
# Read today's daily note
cat ~/workspace/memory/$(date +%Y-%m-%d).md 2>/dev/null || echo "(no daily note yet)"

# Read long-term memory
cat ~/workspace/MEMORY.md
```

If memory files don't exist yet, that's fine — you're starting fresh. Continue with git log for context.

---

### Step 3 — Determine Task Status

For each task assigned in the sprint kickoff message:

| Status | Indicator |
|--------|-----------|
| **Done** | Committed to `feat/sNN` branch |
| **In progress** | Uncommitted changes exist (`git status`) |
| **Not started** | No commits, no changes |
| **Blocked** | Error in last attempt — check daily note |

```bash
# Check for uncommitted work
cd ~/Zeus && git status --short

# Check what's already on the sprint branch
git log main..HEAD --oneline
```

---

### Step 4 — Report State

When reporting sprint state to the team channel, use this format:

```
**Sprint S{NN} — Status**

✅ Task 1: {description} — commit `{hash}`
🔄 Task 2: {description} — in progress
⬜ Task 3: {description} — not started
🚫 Task 4: {description} — blocked: {reason}

Next: {what you're doing next}
```

Always include the commit hash for completed tasks. Never say "done" without a hash.

---

### Step 5 — Update HEARTBEAT.md

When a new task is assigned, update `## CURRENT TASK` immediately:

```bash
# Edit HEARTBEAT.md with the new task
# Keep the hourly section intact — only update CURRENT TASK
```

Format:
```markdown
## CURRENT TASK
Sprint S{NN}: {task description}
Branch: feat/s{NN}
```

---

## Sprint Lifecycle

```
Kickoff message → read tasks → branch exists? → create feat/sNN if not
       ↓
   Work tasks → commit each deliverable → push
       ↓
   Report to channel with commit hashes
       ↓
   Zeus100 merges → sprint closed → wait for next kickoff
```

---

## Key Files

| File | Purpose |
|------|---------|
| `~/workspace/HEARTBEAT.md` | Current task assignment |
| `~/workspace/MEMORY.md` | Long-term curated memory |
| `~/workspace/memory/YYYY-MM-DD.md` | Daily raw log |
| `~/Zeus/` | Main codebase |
| `feat/sNN` branch | Sprint work branch |

---

## Common Gotchas

**Branch has merge conflicts:** Resolve with `git checkout --theirs {file} && git add {file}` for files you didn't touch. For files you did touch, manually resolve and commit.

**HEARTBEAT.md says wrong sprint:** Update it immediately. Don't work blind.

**Task already done by another agent:** Check if the commit is on the branch before re-doing work. Duplicate effort is wasted effort.

**No kickoff message in memory:** Check the Discord channel directly or ask Zeus100 for the task list. Don't assume — the task list is the source of truth.

**Merge conflict on feat/sNN creation:** The branch may already exist on remote. Try `git checkout feat/sNN` (no `-b`) or `git checkout -b feat/sNN origin/feat/sNN`.

---

## Quality Gates

- MUST read HEARTBEAT.md at session start before any other action
- MUST commit deliverables before reporting "done"
- MUST include commit hash in status reports
- MUST NOT start work on an unconfirmed task (Zeus100 assigns, not self-assigned)
- MUST push sprint branch before session end
