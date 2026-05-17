# Testing Requirements (Zeus / Rust)

## Minimum Coverage

Target: meaningful test coverage for all new code. Use `cargo tarpaulin` for measurement.

Test types (all required for new features):
1. **Unit tests** — `#[test]` blocks in each module
2. **Integration tests** — `tests/` directory per crate; test full API paths
3. **Async tests** — `#[tokio::test]` for async functions

## Test-Driven Development (Mandatory)

```
RED   → write failing test first
GREEN → write minimal code to pass
REFACTOR → improve code while keeping tests green
```

1. Write `#[test]` or `#[tokio::test]` first
2. Run `cargo test -p <crate>` — verify it FAILS
3. Implement minimal code
4. Run `cargo test -p <crate>` — verify it PASSES
5. Refactor; run again

## Zeus-Specific Test Patterns

```rust
// Env-gated integration tests (not #[cfg] — use early-return)
#[tokio::test]
async fn test_llm_integration() {
    if std::env::var("ZEUS_HAS_LLM").is_err() {
        eprintln!("SKIP: set ZEUS_HAS_LLM=1 to run");
        return;
    }
    // ... real test
}

// Tempfile pattern for hermetic tests (never write to ~/.zeus/ in tests)
#[tokio::test]
async fn test_queue_drain() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    // ...
}
```

## Pre-merge Test Gate

```bash
cargo test --workspace           # all tests pass
cargo clippy --workspace -- -D warnings   # 0 warnings
cargo fmt --check                          # 0 fmt issues
```

## Troubleshooting Test Failures

1. Use **tdd-guide** agent
2. Check test isolation (no shared state, no live ~/.zeus/ reads)
3. Verify async runtime setup (`#[tokio::test]` not `#[test]` for async)
4. Fix implementation, not tests (unless test itself is wrong)
5. Never use `--no-verify` to bypass hooks
