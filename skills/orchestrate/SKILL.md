---
name: orchestrate
description: "Run a sequential agent pipeline: plan → implement (TDD) → review → security check. Full workflow for feature development."
user-invocable: true
skillKey: orchestrate
read_when:
  - "orchestrate"
  - "full workflow"
  - "end to end"
---

# Orchestrate — Sequential Agent Pipeline

Run a complete development workflow: plan → implement → review → verify → security check.

## When to Use

- Implementing a complete feature from scratch
- When you want the full development lifecycle applied
- Complex features that need planning, TDD, review, and security check

## Pipeline Stages

### Stage 1: Plan
- Restate requirements
- Identify affected crates
- Break into phases
- Assess risks
- **Gate**: Wait for user confirmation

### Stage 2: Implement (TDD)
For each phase:
- Write failing test (RED)
- Implement minimal code (GREEN)
- Refactor (IMPROVE)
- Run `cargo test -p <crate>`

### Stage 3: Verify
- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace`
- `cargo fmt -- --check`

### Stage 4: Code Review
- Check code quality (functions < 50 lines, files < 800)
- Check error handling (no unwrap on fallible paths)
- Check naming and structure
- Flag CRITICAL/HIGH/MEDIUM issues

### Stage 5: Security Review
- No hardcoded secrets
- Input validation at boundaries
- No unsafe without safety comment
- File paths sanitized
- SQL parameterized

### Stage 6: Summary
- List all changes made
- List all tests added
- Report any remaining issues
- Suggest next steps

## Rules

- Each stage must complete before the next begins
- If a stage fails, fix issues before proceeding
- Plan stage requires explicit user confirmation
- Security issues are always CRITICAL — fix immediately

## Integration

- `/orchestrate` runs all stages in sequence
- Individual stages available as: `/plan`, `/tdd`, `/verify`, `/code-review`, `/security-review`
