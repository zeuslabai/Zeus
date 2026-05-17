---
name: tdd
description: "Test-driven development workflow: write failing tests first, implement minimal code, refactor. Enforces RED→GREEN→REFACTOR cycle with cargo test."
user-invocable: true
skillKey: tdd
read_when:
  - "test driven"
  - "tdd"
  - "write tests first"
---

# TDD — Test-Driven Development

Enforce test-driven development for Rust code using `cargo test`.

## When to Use

- Implementing new features or functions
- Fixing bugs (write test that reproduces bug first)
- Refactoring existing code
- Building critical business logic

## How It Works

### TDD Cycle: RED → GREEN → REFACTOR

1. **RED** — Write a failing test
   - Define the expected behavior in a `#[test]` function
   - Run `cargo test -p <crate> -- <test_name>` — it MUST fail
   - Verify it fails for the RIGHT reason (not a compile error)

2. **GREEN** — Write minimal implementation
   - Write the smallest amount of code to make the test pass
   - Do NOT add extra features or handle edge cases yet
   - Run `cargo test` — it MUST pass

3. **REFACTOR** — Improve code quality
   - Clean up duplication, naming, structure
   - Run `cargo test` after each change — tests MUST stay green
   - Run `cargo clippy` — 0 warnings

4. **REPEAT** — Next test case

### Rules

- NEVER write implementation before tests
- NEVER skip running tests after changes
- Tests should be small, focused, and independent
- Test behavior, not implementation details
- Prefer integration tests over excessive mocking
- Target: all new code paths covered

### Example

```rust
// 1. RED — Write failing test
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_score_high_confidence() {
        let result = calculate_score(0.9, 100);
        assert!(result > 80);
        assert!(result <= 100);
    }

    #[test]
    fn test_calculate_score_zero_input() {
        let result = calculate_score(0.0, 0);
        assert_eq!(result, 0);
    }
}

// 2. GREEN — Minimal implementation
fn calculate_score(confidence: f64, interactions: u32) -> u32 {
    if interactions == 0 { return 0; }
    (confidence * interactions as f64).round() as u32
}

// 3. REFACTOR — Improve
fn calculate_score(confidence: f64, interactions: u32) -> u32 {
    match interactions {
        0 => 0,
        n => (confidence * n as f64).round().min(100.0) as u32,
    }
}
```

## Integration

- Use `/plan` first to understand what to build
- Use `/tdd` to implement with tests
- Use `/build-fix` if cargo errors occur
- Use `/code-review` to review implementation
- Use `/verify` to run full verification cycle
