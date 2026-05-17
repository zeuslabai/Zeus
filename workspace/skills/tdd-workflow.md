---
name: tdd-workflow
description: Test-driven development for Zeus (Rust). Write tests first, implement to pass, refactor. Use for all new features, bug fixes, and refactoring.
origin: ECC (adapted for Zeus/Rust)
---

# TDD Workflow (Zeus / Rust)

## Core Cycle

```
RED   → write failing test
GREEN → write minimal code to pass
REFACTOR → improve while keeping green
```

## When to Use

- Writing any new function, module, or crate
- Fixing a bug (write test that reproduces the bug first)
- Refactoring (ensure tests exist before touching code)
- Adding API endpoints

## Step-by-Step

### Step 1: Write the failing test

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_happy_path() {
        // Arrange
        let input = ...;
        // Act
        let result = my_function(input);
        // Assert
        assert_eq!(result, expected);
    }

    #[test]
    fn test_feature_error_case() {
        let result = my_function(invalid_input);
        assert!(result.is_err());
    }
}
```

For async code:
```rust
#[tokio::test]
async fn test_async_feature() {
    let result = my_async_function().await;
    assert!(result.is_ok());
}
```

### Step 2: Run — verify RED
```bash
cargo test -p <crate-name> test_feature
# Should fail: function not found / assertion fails
```

### Step 3: Implement minimal code to pass GREEN
Write only what's needed to make the test pass. No extra logic.

### Step 4: Run — verify GREEN
```bash
cargo test -p <crate-name> test_feature
# Should pass
```

### Step 5: Refactor
Improve code quality while keeping tests green:
- Extract common logic
- Improve naming
- Remove duplication
- Run `cargo test` after each refactor step

### Step 6: Add edge cases
```rust
#[test]
fn test_feature_empty_input() { ... }

#[test]
fn test_feature_boundary_values() { ... }
```

## Zeus-Specific Patterns

### Hermetic tests (never read live ~/.zeus/)
```rust
#[tokio::test]
async fn test_queue_operations() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db = MyDb::new(tmp.path()).unwrap();
    // test against tmp, not live DB
}
```

### Env-gated integration tests
```rust
#[tokio::test]
async fn test_llm_call() {
    if std::env::var("ZEUS_HAS_LLM").is_err() {
        eprintln!("SKIP: set ZEUS_HAS_LLM=1 to run LLM integration tests");
        return;
    }
    // real test
}
```

### Mutex for env-var-mutating tests
```rust
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_env_dependent() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("MY_VAR", "value");
    // test
    std::env::remove_var("MY_VAR");
}
```

## Coverage Target

- Aim for meaningful coverage on all new code
- 100% coverage required for: security-critical paths, financial calculations, core agent loop
- Use `cargo tarpaulin -p <crate>` to measure
