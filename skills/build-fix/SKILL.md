---
name: build-fix
description: "Fix Rust build errors incrementally. Analyze cargo output, fix one error at a time, verify after each fix."
user-invocable: true
skillKey: build_fix
read_when:
  - "build error"
  - "compile error"
  - "cargo build failed"
  - "build fix"
---

# Build Fix

Fix Rust build errors systematically, one at a time.

## When to Use

- `cargo build` or `cargo test` fails
- After refactoring that breaks compilation
- Resolving dependency or type errors
- Fixing clippy warnings

## How It Works

### Step-by-Step Process

1. **Run build** — `cargo build 2>&1 | head -50`
2. **Analyze first error** — Read the compiler message carefully
3. **Fix one error** — Make the minimal change to resolve it
4. **Verify** — `cargo build` again
5. **Repeat** — Until all errors resolved
6. **Final check** — `cargo clippy` + `cargo test`

### Rules

- Fix errors ONE AT A TIME — don't try to fix everything at once
- Read the FULL error message — Rust's compiler messages are excellent
- Check the suggested fix — `rustc` often tells you exactly what to do
- Don't suppress errors with `#[allow(...)]` unless truly justified
- After all errors fixed, run `cargo clippy` for warnings

### Common Patterns

| Error | Fix |
|-------|-----|
| `E0308` type mismatch | Check expected vs actual types, add conversion |
| `E0382` use after move | Clone, borrow, or restructure ownership |
| `E0277` trait not satisfied | Add trait bound or implement trait |
| `E0433` unresolved import | Check `use` path, add to Cargo.toml |
| `E0599` method not found | Check trait imports, receiver type |
| `E0061` wrong arg count | Check function signature |
| lifetime errors | Add lifetime annotations or restructure borrows |

### Integration

- Use `/build-fix` when cargo fails
- Use `/verify` for full build + test + clippy cycle
- Use `/code-review` after fixes are complete
