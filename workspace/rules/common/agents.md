# Agent Orchestration

## Available Agents (Zeus Fleet)

| Agent | Purpose | When to Use |
|-------|---------|-------------|
| planner | Implementation planning | Complex features, refactoring |
| architect | System design | Architectural decisions, new crates |
| tdd-guide | Test-driven development | New features, bug fixes |
| code-reviewer | Code review | After writing code |
| security-reviewer | Security analysis | Before commits touching aegis/auth/tools |
| build-error-resolver | Fix build errors | When cargo build fails |
| refactor-cleaner | Dead code cleanup | Code maintenance |
| doc-updater | Documentation | Updating docs |

## Immediate Agent Usage

No user prompt needed:
1. Complex feature requests → use **planner** agent
2. Code just written/modified → use **code-reviewer** agent
3. Bug fix or new feature → use **tdd-guide** agent
4. Architectural decision (new crate, major refactor) → use **architect** agent
5. Code touching security/auth/tools → use **security-reviewer** agent

## Parallel Task Execution

ALWAYS use parallel Task execution for independent operations:

```
# GOOD: Parallel execution
Launch 3 agents in parallel:
1. Agent 1: Security analysis of zeus-aegis changes
2. Agent 2: Performance review of zeus-mnemosyne query path
3. Agent 3: Type checking of zeus-core changes

# BAD: Sequential when unnecessary
First agent 1, then agent 2, then agent 3
```

## Gate Protocol (Zeus Fleet)

- Features: 4/4 non-author LGTMs before merge
- Housekeeping: 2/2 non-author LGTMs before merge
- Self-gates do not count
- Branch author cannot gate their own branch

## Multi-Perspective Analysis

For complex problems, use split-role sub-agents:
- Factual reviewer
- Senior Rust engineer
- Security expert (aegis focus)
- Consistency reviewer (cross-crate)
- Redundancy checker
