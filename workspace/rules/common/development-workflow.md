# Development Workflow (Zeus / Rust)

## Feature Implementation Workflow

0. **Research & Reuse** _(mandatory before any new implementation)_
   - Search the Zeus codebase first: existing crates, traits, patterns
   - Check `crates.io` before writing utility code — prefer battle-tested libraries
   - Check OpenClaw / ECC for prior art on agent/AI patterns
   - Prefer adapting a proven approach over writing net-new code

1. **Plan First**
   - Use **planner** agent for complex features
   - Break into phases, identify cross-crate dependencies
   - Identify which crates are touched and why

2. **TDD Approach**
   - Use **tdd-guide** agent
   - Write tests first (RED): `#[test]` or `#[tokio::test]`
   - Implement to pass tests (GREEN)
   - Refactor (IMPROVE)
   - Verify: `cargo test -p <crate>` passes

3. **Code Review**
   - Use **code-reviewer** agent immediately after writing code
   - Address CRITICAL and HIGH issues before PR

4. **Pre-commit Checks**
   ```bash
   cargo clippy --workspace -- -D warnings   # 0 warnings
   cargo fmt --check                          # 0 fmt issues
   cargo test --workspace                     # all tests pass
   ```

5. **Commit & Branch**
   - Branch naming: `feat/<sprint>-<description>` or `fix/<description>`
   - Detailed commit messages following conventional commits
   - Gate protocol: 4/4 non-author LGTMs for features, 2/2 for housekeeping

## Zeus-Specific Notes

- New crate? Add to `Cargo.toml` workspace members AND workspace.dependencies
- Cross-crate struct initialization: always `grep -rn "StructName {" crates/` before pushing to catch missing field errors
- `#[allow(dead_code)]` only with comment explaining why; prefer gating with `#[cfg(target_os = "macos")]`
- Env-gated tests: use early-return `eprintln!` skip pattern (not `#[cfg]` attribute)
