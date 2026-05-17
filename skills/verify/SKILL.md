---
name: verify
description: "Run full verification cycle: cargo build + test + clippy + fmt check. Reports pass/fail for each stage."
user-invocable: true
skillKey: verify
read_when:
  - "verify"
  - "verification"
  - "check everything"
  - "run all checks"
---

# Verify — Full Verification Cycle

Run the complete Rust verification pipeline: build → test → clippy → fmt.

## When to Use

- After implementing a feature
- Before committing or pushing
- After refactoring
- Before creating a PR
- Periodic workspace health check

## Pipeline

### Step 1: Build
```bash
cargo build --workspace 2>&1
```
**Pass**: Exit code 0, no errors
**Fail**: Fix with `/build-fix`

### Step 2: Test
```bash
cargo test --workspace 2>&1
```
**Pass**: All tests pass
**Fail**: Fix failing tests, then re-run

### Step 3: Clippy
```bash
cargo clippy --workspace 2>&1
```
**Pass**: 0 warnings
**Fail**: Fix warnings (never suppress without justification)

### Step 4: Format Check
```bash
cargo fmt -- --check 2>&1
```
**Pass**: No formatting issues
**Fail**: Run `cargo fmt` to fix

## Output Format

```
## Verification Report

| Stage | Status | Details |
|-------|--------|---------|
| Build | ✅ PASS | 0 errors |
| Test  | ✅ PASS | 1711/1711 pass |
| Clippy | ✅ PASS | 0 warnings |
| Format | ✅ PASS | Clean |

**Verdict**: ALL CLEAR — ready to commit
```

Or on failure:

```
| Stage | Status | Details |
|-------|--------|---------|
| Build | ✅ PASS | 0 errors |
| Test  | ❌ FAIL | 2 failures in zeus-nous |
| Clippy | — | Skipped (tests failed) |
| Format | — | Skipped |

**Verdict**: FIX REQUIRED — 2 test failures
```

## Rules

- Run ALL stages in order
- Stop on first failure (no point running clippy if tests fail)
- Report exact error counts and locations
- Suggest specific fixes for common issues
