---
name: strategic-compact
description: Context compaction strategy for long Zeus development sessions. Compact at logical phase boundaries, not mid-implementation.
origin: ECC (adapted for Zeus fleet sessions)
---

# Strategic Compact (Zeus Fleet)

## When to Compact

Compact at **logical phase boundaries**, not mid-implementation.

| Phase Transition | Compact? | Why |
|-----------------|----------|-----|
| Research → Planning | ✅ Yes | Research context is bulky; plan is the distilled output |
| Planning → Implementation | ✅ Yes | Plan captured in TodoWrite/branch; free context for code |
| Sprint complete → Next sprint | ✅ Yes | Clean slate for new work |
| After debugging a complex issue | ✅ Yes | Debug traces pollute unrelated work |
| Mid-implementation of a feature | ❌ No | Losing variable names, file paths, partial state is costly |
| After a failed approach | ✅ Yes | Clear dead-end reasoning before trying alternative |
| Mid-gate review | ❌ No | Need full diff context |

## Zeus-Specific Guidance

### What survives compaction (safe to compact knowing these persist)
- `CLAUDE.md` + `AGENTS.md` instructions
- TodoWrite / TaskCreate task list
- Memory files (`~/.claude/projects/*/memory/MEMORY.md`)
- Git state (commits, branches, stash)
- All files on disk (workspace rules, contexts, skills)
- Zeus codebase (unchanged)

### What is lost on compaction
- Previously-read file contents (re-read after compact)
- Multi-step conversation context / reasoning chains
- Tool call history
- Nuanced preferences stated only verbally

### Before compacting — save to disk
```bash
# If you have important context, write it to workspace
# zeus memory remember "key fact to preserve"
# Or write to a notes file in ~/.zeus/workspace/
```

## Fleet Coordination

- Long sprint sessions (S20 = many files) benefit from compaction between phases
- Phase 1 (implementation) → Phase 2 (audit) boundary is a good compact point
- Each agent compacts independently — no coordination needed
- After compact, re-read relevant CLAUDE.md + sprint branch status before continuing

## Token Budget

- `MAX_THINKING_TOKENS=10000` (set in env)
- `AUTOCOMPACT` threshold: compact when context approaches 50% remaining
- Manual `/compact` preferred over auto-compact for multi-phase work

## Quick Checklist Before Compacting

- [ ] Current task status written to TodoWrite or posted to Discord
- [ ] Any unsaved analysis written to a file
- [ ] Current branch/commit hash noted
- [ ] Gate requests posted to Discord (won't be in context after compact)
