# Coding Style (Zeus / Rust)

## Immutability (CRITICAL)

ALWAYS prefer immutable patterns:

```rust
// WRONG: mutate in place unnecessarily
fn update(config: &mut Config, key: &str, value: &str) { ... }

// CORRECT: return new value where possible
fn with_model(config: &Config, model: &str) -> Config { ... }
```

## File Organization

MANY SMALL FILES > FEW LARGE FILES:
- High cohesion, low coupling
- 300–500 lines typical per source file; flag anything over 800 lines
- Extract utilities into separate modules
- Organize by feature/domain (e.g. per-crate, not per-type)

## Error Handling

ALWAYS handle errors explicitly — no silent swallowing:

```rust
// WRONG
let _ = some_fallible_op();

// CORRECT — log or propagate
if let Err(e) = some_fallible_op() {
    warn!("operation failed: {e}");
}
```

- Use `anyhow::Context` for external call errors
- Use `thiserror` for library error types
- No `unwrap()` on fallible paths in production code
- `expect()` only in tests or truly unreachable branches

## Input Validation

ALWAYS validate at system boundaries:
- Validate all user input before processing (API endpoints, CLI args)
- Use typed schemas (serde + validation) where available
- Fail fast with clear error messages
- Never trust external data (API responses, user input, file content)

## Code Quality Checklist

Before marking work complete:
- [ ] Code is readable and well-named
- [ ] Functions are focused (single responsibility)
- [ ] Files are focused (<800 lines)
- [ ] No deep nesting (>4 levels)
- [ ] Proper error handling (no silent `let _ =`)
- [ ] No hardcoded values (use config or env vars)
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo fmt --check` passes
