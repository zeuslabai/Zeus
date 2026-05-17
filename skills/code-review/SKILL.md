---
name: code-review
description: "Review code for quality, security, and maintainability. Run clippy, check error handling, validate tests, flag CRITICAL/HIGH/MEDIUM issues."
user-invocable: true
skillKey: code_review
read_when:
  - "code review"
  - "review code"
  - "review my code"
---

# Code Review

Structured code review for Rust code quality, security, and maintainability.

## When to Use

- After implementing a feature
- Before merging a branch
- When reviewing someone else's code
- Periodic codebase quality checks

## Review Checklist

### 1. Build & Lint
- `cargo clippy -p <crate>` — 0 warnings required
- `cargo fmt -- --check` — formatting clean
- `cargo test -p <crate>` — all tests pass

### 2. Code Quality
- [ ] Functions < 50 lines (split if longer)
- [ ] Files < 800 lines (refactor if longer)
- [ ] No deep nesting (> 4 levels)
- [ ] Clear, descriptive names
- [ ] No `unwrap()` on fallible paths (use `?` or proper error handling)
- [ ] No `clone()` where a reference suffices
- [ ] No dead code (`#[allow(dead_code)]` must be justified)

### 3. Error Handling
- [ ] All `Result` types properly handled
- [ ] Error messages are actionable (include context)
- [ ] No silent error swallowing
- [ ] `anyhow::Context` on external calls

### 4. Security
- [ ] No hardcoded secrets
- [ ] User input validated at boundaries
- [ ] SQL queries parameterized
- [ ] No `unsafe` without safety comment
- [ ] File paths sanitized

### 5. Testing
- [ ] New code has tests
- [ ] Edge cases covered
- [ ] Integration tests for cross-crate changes
- [ ] Tests are independent (no ordering dependency)

### 6. Rust-Specific
- [ ] `is_char_boundary()` before byte-slicing strings
- [ ] `#[cfg(target_os = "...")]` for platform-specific code
- [ ] Proper `Send + Sync` bounds on async types
- [ ] No `std::env::set_var` without `unsafe` block (Rust 2024)

## Issue Severity

| Level | Action | Examples |
|-------|--------|---------|
| CRITICAL | Fix before merge | Security vuln, data loss, panic in prod |
| HIGH | Fix before merge | Missing error handling, untested code path |
| MEDIUM | Fix or justify | Clippy warning, style inconsistency |
| LOW | Optional | Naming suggestion, minor refactor |

## Output Format

```
## Code Review: [file/crate]

### CRITICAL (must fix)
- [issue description + fix suggestion]

### HIGH (must fix)
- [issue description]

### MEDIUM (should fix)
- [issue description]

### Summary
- Issues: X critical, Y high, Z medium
- Verdict: APPROVE / REQUEST CHANGES
```
