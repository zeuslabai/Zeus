# Hooks System (Zeus / Claude Code)

## Active Hooks (zeus106 / .106)

| Hook | Event | What it does |
|------|-------|-------------|
| cargo fmt | PostToolUse/Edit (*.rs) | Auto-format Rust files after edits |
| State save | PreCompact | Save branch + task state before context compaction |
| CL-v2 observer | Pre+PostToolUse/* | Capture tool observations for zeus-nous learning (S20-2) |
| SessionStart | SessionStart | Load previous session context |
| SessionEnd | SessionEnd | Persist state + extract patterns |

## Hook Best Practices

- Hooks run on every matching tool call — keep them fast (<100ms for sync hooks)
- Use `async: true` + `timeout` for slow operations
- Hook failures that block tool use should be rare — prefer warn-only hooks
- Never bypass hooks with `--no-verify` unless explicitly instructed

## TodoWrite Best Practices

Use TodoWrite/TaskCreate to:
- Track progress on multi-step tasks
- Verify understanding of instructions before starting
- Enable real-time steering on complex sprints
- Show granular implementation steps to fleet coordinator

## Zeus Fleet Hook Coordination

- Hooks are per-machine (`~/.claude/settings.json`)
- Fleet-wide hook changes should be documented in workspace rules
- Test hooks locally before proposing fleet-wide adoption
- Hooks that touch zeus-nous/Mnemosyne require backend coordination (fbsd3 lane)
