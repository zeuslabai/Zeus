---
name: verification-loop
description: Comprehensive pre-commit/pre-PR verification for Zeus (Rust). Runs cargo test, clippy, fmt, and security checks. Invoke before every PR or major change.
origin: ECC (adapted for Zeus/Rust)
---

# Verification Loop Skill (Zeus / Rust)

## When to Use

- After completing a feature or significant code change
- Before creating a PR or requesting gate review
- After refactoring across multiple crates
- When you want confidence all gates will pass

## Verification Phases

### Phase 1: Format Check
```bash
cargo fmt --check
```
If fails: run `cargo fmt` then verify again.

### Phase 2: Lint / Clippy
```bash
cargo clippy --workspace -- -D warnings
```
Fix ALL warnings before continuing. No `#[allow(...)]` without a comment.

### Phase 3: Test Suite
```bash
cargo test --workspace
```
Report:
- Total tests: X
- Passed: X
- Failed: X (STOP if any failed)

For a single crate: `cargo test -p <crate-name>`

### Phase 4: Security Scan
```bash
# Check for hardcoded secrets
grep -rn "api_key\s*=\s*\"" crates/ --include="*.rs" | grep -v test
grep -rn "token\s*=\s*\"" crates/ --include="*.rs" | grep -v test

# Check for bare unwrap on external data paths
grep -rn "\.unwrap()" crates/ --include="*.rs" | grep -v test | grep -v "expect("
```

### Phase 5: Diff Review
```bash
git diff origin/main...HEAD --stat
git diff origin/main...HEAD --name-only
```
Review each changed file for:
- Unintended changes
- Missing error handling
- Missing tests for new code
- New struct fields that need initialization in all call sites

### Phase 6: Struct Initializer Sweep (critical for Zeus)
After any struct field additions:
```bash
grep -rn "StructName {" crates/
```
Verify all initializers include the new field.

## Verification Report Format

```
VERIFICATION REPORT
===================
Crate scope: [workspace / specific crate]

fmt:     [PASS/FAIL]
clippy:  [PASS/FAIL] (X warnings)
tests:   [PASS/FAIL] (X/Y passed)
secrets: [PASS/FAIL]
diff:    X files changed, +Y/-Z lines

Overall: [READY / NOT READY] for gate review

Issues to fix:
1. ...
```

## Gate Readiness Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace -- -D warnings` — 0 warnings
- [ ] `cargo test --workspace` — 0 failures
- [ ] No hardcoded secrets
- [ ] Struct initializers complete
- [ ] New code has tests
- [ ] Tests are hermetic (no live `~/.zeus/` reads)
