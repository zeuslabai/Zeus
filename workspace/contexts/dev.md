# Development Context

Use this context when actively implementing features or fixing bugs in Zeus.

## Active Mode: Development

You are in development mode. Follow this workflow strictly:

1. **Research first** — search the Zeus codebase before writing new code
   - `grep -rn "pattern" crates/` to find existing implementations
   - Check zeus-core for types/traits before duplicating
   - Check crates.io for existing solutions before writing utilities

2. **Plan before coding** — for anything touching >2 files, use the planner agent or `/plan`

3. **TDD** — write tests first (`#[test]` or `#[tokio::test]`), verify RED, then implement GREEN

4. **Pre-commit gate** — before any commit:
   ```bash
   cargo clippy --workspace -- -D warnings
   cargo fmt --check
   cargo test --workspace
   ```

5. **No silent errors** — never `let _ = fallible_op()`. Log or propagate.

## Zeus Codebase Quick Reference

- Types/errors: `zeus-core`
- LLM calls: `zeus-llm` (use `LlmClient`, not raw reqwest)
- Tool execution: `zeus-agent/src/tools.rs`
- Memory: `zeus-mnemosyne` (FTS5 + vector)
- Security: `zeus-aegis` (check before shell/fetch)
- macOS tools: `zeus-talos`
- API routes: `zeus-api/src/handlers/`
- Scheduling: `zeus-prometheus`

## Rust Edition

Workspace uses **edition 2024** (`rust-version = "1.88"`). Let-chains and other 2024 features are available.
