---
name: search-first
description: Research-before-coding for Zeus. Search the existing codebase, crates.io, and prior art before writing new code. Prevents duplication and wheel-reinvention.
origin: ECC (adapted for Zeus/Rust)
---

# Search First (Zeus / Rust)

## Trigger

Use this skill before:
- Writing any new utility function
- Adding a new dependency
- Implementing a feature that sounds common
- Creating a new type or trait

## Workflow

```
1. CODEBASE SEARCH — does Zeus already have this?
2. CRATE SEARCH — does crates.io have a battle-tested solution?
3. ECC / PRIOR ART — did the fleet build something similar?
4. DECIDE — adopt / extend / build
```

## Step 1: Zeus Codebase Search

```bash
# Find existing types
grep -rn "struct TypeName\|trait TraitName" crates/

# Find existing functionality
grep -rn "fn function_name\|fn.*keyword" crates/

# Find existing patterns
grep -rn "pattern_keyword" crates/ --include="*.rs"
```

Check these crates first:
- `zeus-core` — common types, errors, config
- `zeus-agent/src/tools.rs` — existing tools
- `zeus-talos` — macOS automation (193 tools)
- `zeus-mnemosyne` — memory/search
- `zeus-prometheus` — scheduling, pipelines

## Step 2: Crates.io Search

Before writing a utility, check:
- HTTP: `reqwest` (already in workspace)
- Serialization: `serde` / `serde_json` (already in workspace)
- DB: `rusqlite` (already in workspace)
- Async: `tokio` / `futures` (already in workspace)
- Error handling: `anyhow` / `thiserror` (already in workspace)
- Scheduling: `cron` (already in workspace)
- IDs: `uuid` / `ulid` (already in workspace)

New dep candidates: search crates.io, check downloads, check last update, check license (MIT/Apache preferred).

## Step 3: ECC / Fleet Prior Art

```bash
ls ~/everything-claude-code/skills/
grep -rn "keyword" ~/everything-claude-code/ --include="*.md"
```

## Decision Matrix

| Signal | Action |
|--------|--------|
| Already in Zeus codebase | **Use existing** — don't duplicate |
| In workspace dependencies | **Import from workspace** — add to crate's Cargo.toml |
| Battle-tested crate, needed | **Add to workspace.dependencies** — propose in PR |
| Nothing suitable | **Build custom** — informed by research |

## Anti-Patterns

- Writing a URL parser when `url` crate is already in workspace
- Creating a new `Error` type per-crate when `zeus-core::Error` exists
- Implementing retry logic when zeus-llm already has `is_retryable_status()`
- Adding a new HTTP client when `reqwest::Client` via `build_llm_client()` pattern exists
