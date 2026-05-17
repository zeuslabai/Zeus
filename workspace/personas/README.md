# Workspace Personas

Task-scoped agent configurations for focused, repeatable work. These are *what the agent does* — not who it is.

## What This Is

Personas here are operational configs: they define tools, model, and a specific task workflow. They're invoked for a job and dismissed when done. Think of them as specialized modes, not identities.

This is distinct from `personalities/` which defines durable character and voice.

## Personas

| File                    | Purpose                                              |
|-------------------------|------------------------------------------------------|
| `code-reviewer.md`      | Rust/Zeus code review — quality, security, idioms    |
| `refactor-cleaner.md`   | Targeted refactoring with minimal blast radius       |
| `security-auditor.md`   | Threat modeling and vulnerability review             |
| `tdd-guide.md`          | Test-driven development workflow guide               |
| `build-error-resolver.md` | Diagnose and fix build/compile failures            |

## Personalities vs Personas

| | `personalities/` | `workspace/personas/` |
|---|---|---|
| **Purpose** | Who the agent is | What the agent does |
| **Scope** | Persistent identity | Task-scoped mode |
| **Format** | Character + voice prose | Tool config + workflow steps |
| **Lifespan** | Across all sessions | Duration of a task |
