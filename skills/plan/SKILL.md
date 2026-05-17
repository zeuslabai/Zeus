---
name: plan
description: "Create an implementation plan before writing code. Restate requirements, assess risks, break into phases, and WAIT for user confirmation."
user-invocable: true
skillKey: plan
read_when:
  - "plan"
  - "implementation plan"
  - "design"
  - "architect"
---

# Plan — Implementation Planning

Create a structured implementation plan and wait for explicit user confirmation before writing any code.

## When to Use

- Starting a new feature
- Making architectural changes
- Complex refactoring (multiple crates)
- Requirements are unclear or ambiguous
- Multiple files/crates will be affected

## How It Works

### Planning Steps

1. **Restate Requirements** — Clarify what needs to be built in your own words
2. **Identify Affected Crates** — Which Zeus crates will be modified
3. **Break Into Phases** — Sequential implementation steps
4. **Assess Risks** — What could go wrong (HIGH/MEDIUM/LOW)
5. **Estimate Complexity** — Simple / Moderate / Complex
6. **Present Plan** — Show the full plan
7. **WAIT** — Do NOT write code until user says "yes" / "proceed" / "go"

### Plan Format

```markdown
# Implementation Plan: [Feature Name]

## Requirements
- [bullet points restating what's needed]

## Affected Crates
- zeus-xxx (reason)
- zeus-yyy (reason)

## Phases

### Phase 1: [name]
- [ ] Step 1
- [ ] Step 2

### Phase 2: [name]
- [ ] Step 3
- [ ] Step 4

## Risks
- HIGH: [description]
- MEDIUM: [description]
- LOW: [description]

## Complexity: [Simple/Moderate/Complex]

**WAITING FOR CONFIRMATION** — Proceed? (yes/no/modify)
```

### Rules

- NEVER write code before confirmation
- Keep phases small and independently testable
- Identify dependencies between phases
- Flag any crate boundary changes
- If user says "modify" — revise and re-present

## Integration

- Use `/plan` to design the approach
- Use `/tdd` to implement each phase
- Use `/verify` after each phase
- Use `/code-review` when complete
