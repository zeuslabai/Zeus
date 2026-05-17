---
name: learn
description: "Extract patterns and lessons from the current session. Identify reusable knowledge, store in Mnemosyne memory for future sessions."
user-invocable: true
skillKey: learn
read_when:
  - "learn from"
  - "extract pattern"
  - "remember this pattern"
---

# Learn — Pattern Extraction

Extract reusable patterns, lessons, and knowledge from the current session and store them in Zeus memory (Mnemosyne).

## When to Use

- After solving a tricky bug
- After discovering a useful pattern
- When a solution should be remembered for future sessions
- After debugging a production issue
- When establishing a new convention

## What to Extract

### Pattern Types

| Type | Example | Storage |
|------|---------|---------|
| Bug fix | "emoji in strings → use `is_char_boundary()`" | Mnemosyne lesson |
| Convention | "always gate macOS-only code with `#[cfg]`" | MEMORY.md rule |
| Workflow | "run clippy before every commit" | MEMORY.md rule |
| Architecture | "channels crate uses `ChannelAdapter` trait" | Mnemosyne fact |
| Debugging | "`std::env::set_var` is unsafe in Rust 2024" | Mnemosyne lesson |

### Extraction Format

```yaml
type: lesson | convention | pattern | debug-insight
trigger: "when [condition]"
action: "do [specific action]"
evidence: "observed in [session/commit/file]"
confidence: tentative | moderate | strong
```

## How It Works

1. **Review session** — Look at what was accomplished, errors hit, solutions found
2. **Identify patterns** — What's reusable? What would help in future sessions?
3. **Classify** — Is it a lesson, convention, pattern, or debug insight?
4. **Store** — Write to Mnemosyne via `memory_store` or update MEMORY.md
5. **Verify** — Confirm it's stored with `memory_search`

### Rules

- Only store CONFIRMED patterns (verified across multiple instances)
- Include evidence (commit hash, file, line number)
- Don't duplicate existing knowledge — check first
- Keep entries concise and actionable
- Update or remove outdated entries

## Integration

- Use at the end of a session to capture learnings
- Use `/evolve` to cluster related patterns into skills
- Pairs with Mnemosyne confidence scoring (Tentative → Established → Confident → Instinct)
