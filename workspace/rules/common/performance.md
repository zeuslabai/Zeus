# Performance Optimization (Zeus / Rust)

## Model Selection Strategy

**Haiku 4.5** (lightweight, 3x cost savings):
- Worker agents in multi-agent (Pantheon) workflows
- Simple intent classification
- Frequent, short-context invocations

**Sonnet 4.6** (best coding model — default for Zeus fleet):
- Main development work (zeus106 = Sonnet 4.6)
- Orchestrating multi-agent workflows
- Complex coding tasks

**Opus 4.6** (deepest reasoning):
- Complex architectural decisions
- Maximum reasoning (Zeus100 = Opus 4.6)
- Research and analysis tasks

## Context Window Management

Avoid the last 20% of context window for:
- Large-scale refactoring across multiple crates
- Feature implementation spanning many files
- Debugging complex async interactions

Lower context sensitivity tasks (safe near limit):
- Single-file edits
- Independent utility functions
- Documentation updates
- Simple bug fixes

## Async / Tokio

- Use `tokio::spawn` for independent background tasks
- Use `futures::join!` / `tokio::join!` for parallel async operations
- Avoid blocking the async runtime: use `tokio::task::spawn_blocking` for CPU-heavy work
- LLM calls have a 5-minute timeout — don't hold locks across them

## Database (SQLite / Mnemosyne)

- Use WAL mode for concurrent read access
- Batch writes where possible
- FTS5 queries are fast — prefer them over full table scans for text search
- Vector search: cosine similarity is O(n) — keep embedding stores bounded

## Build Performance

- `cargo build --release` for benchmarks and production deploys
- Use `cargo check` to verify compilation without full build during development
- `cargo test -p <crate>` to test a single crate (faster than `--workspace`)
- If `cargo build` fails: use **build-error-resolver** agent
